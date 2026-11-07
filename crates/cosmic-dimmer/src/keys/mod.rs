//! Brightness keys for the applet: a session D-Bus interface the applet serves
//! (`service.rs`), and an opt-in writer of COSMIC custom shortcuts that call
//! it (`custom.rs`). This file holds what both share: the keys, the
//! combinations bound to them and how a key press becomes one.
//!
//! Why this shape: on Wayland only the compositor sees global keys. cosmic-comp
//! reads its bindings from the `com.system76.CosmicSettings.Shortcuts` config,
//! where a custom binding can only *spawn a command*. So the running applet
//! serves `Raise` and `Lower` (and `Reset`, for `cosmic-dimmer --reset`)
//! on the session bus, and the shortcuts run
//! `busctl --user call ...`, the same pattern the stock brightness keys use
//! for cosmic-settings-daemon.
//!
//! The plain brightness keys stay bound to the stock daemon (which raises the
//! hardware level over DDC/CI where a display supports it); this writer binds
//! combinations of the user's choice (Super + F1 / F2 until changed), so
//! nothing stock is taken over. See `docs/applet.md` for the limits.

mod custom;
mod service;

use cosmic::iced::keyboard::{self, key::Named};
use cosmic_settings_config::Binding;

pub use custom::{
    Changed, Current, Refused, add_shortcuts, current, rebind, remove_shortcuts, shortcuts_config,
    with_shortcuts,
};
pub use service::{BUS_NAME, Event, OBJECT_PATH, subscription};

/// A brightness key (or the terminal reset), applied to every display.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    Raise,
    Lower,
    /// Every level back to 100 %, which also removes every overlay.
    Reset,
}

impl Key {
    const fn method(self) -> &'static str {
        match self {
            Self::Raise => "Raise",
            Self::Lower => "Lower",
            Self::Reset => "Reset",
        }
    }

    /// The command a shortcut spawns to send this key to the applet.
    #[must_use]
    pub fn command(self) -> String {
        format!(
            "busctl --user call {BUS_NAME} {OBJECT_PATH} {BUS_NAME} {}",
            self.method()
        )
    }
}

/// The key combinations the shortcuts use, as binding text ("Super+F2").
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Keymap {
    pub raise: String,
    pub lower: String,
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            raise: "Super+F2".into(),
            lower: "Super+F1".into(),
        }
    }
}

impl Keymap {
    fn text(&self, key: Key) -> Option<&str> {
        match key {
            Key::Raise => Some(&self.raise),
            Key::Lower => Some(&self.lower),
            Key::Reset => None,
        }
    }
}

/// Whether `text` is a key combination COSMIC can bind.
#[must_use]
pub fn is_binding(text: &str) -> bool {
    Binding::from_str_partial(text).is_ok_and(|binding| binding.is_set())
}

/// The binding text for a captured key press, or `None` while it is not one
/// to accept: a lone modifier, or a key that needs a modifier so that binding
/// it would not take the key away from typing. F-keys and brightness keys can
/// stand alone.
#[must_use]
pub fn combination(key: &keyboard::Key, modifiers: keyboard::Modifiers) -> Option<String> {
    let (name, alone_is_fine) = match key {
        keyboard::Key::Character(character) if character.as_str() != "+" => {
            (character.to_lowercase(), false)
        }
        keyboard::Key::Named(named) => match named {
            Named::BrightnessUp => ("XF86MonBrightnessUp".to_owned(), true),
            Named::BrightnessDown => ("XF86MonBrightnessDown".to_owned(), true),
            Named::ArrowUp => ("Up".to_owned(), false),
            Named::ArrowDown => ("Down".to_owned(), false),
            Named::ArrowLeft => ("Left".to_owned(), false),
            Named::ArrowRight => ("Right".to_owned(), false),
            Named::PageUp => ("Page_Up".to_owned(), false),
            Named::PageDown => ("Page_Down".to_owned(), false),
            Named::Home => ("Home".to_owned(), false),
            Named::End => ("End".to_owned(), false),
            Named::Insert => ("Insert".to_owned(), false),
            Named::Delete => ("Delete".to_owned(), false),
            other => {
                // F1 to F35: iced names these variants exactly as xkb names
                // the keys, so the Debug text is the binding's key name. The
                // test `captured_presses_become_bindable_text` checks it.
                let name = format!("{other:?}");
                let digits = name.strip_prefix('F')?;
                if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                    return None;
                }
                (name, true)
            }
        },
        keyboard::Key::Character(_) | keyboard::Key::Unidentified => return None,
    };
    if !(alone_is_fine || modifiers.logo() || modifiers.control() || modifiers.alt()) {
        return None;
    }
    let mut text = String::new();
    for (held, label) in [
        (modifiers.logo(), "Super+"),
        (modifiers.control(), "Ctrl+"),
        (modifiers.alt(), "Alt+"),
        (modifiers.shift(), "Shift+"),
    ] {
        if held {
            text.push_str(label);
        }
    }
    text.push_str(&name);
    is_binding(&text).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_call_the_served_interface() {
        assert_eq!(
            Key::Raise.command(),
            "busctl --user call io.github.deepthinker2001.CosmicDimmer \
             /io/github/deepthinker2001/CosmicDimmer \
             io.github.deepthinker2001.CosmicDimmer Raise"
        );
        assert!(Key::Lower.command().ends_with(" Lower"));
    }

    #[test]
    fn captured_presses_become_bindable_text() {
        use keyboard::Modifiers as Held;
        let letter = keyboard::Key::Character("K".into());
        let f5 = keyboard::Key::Named(Named::F5);
        assert_eq!(
            combination(&letter, Held::LOGO | Held::SHIFT).as_deref(),
            Some("Super+Shift+k")
        );
        assert_eq!(
            combination(&f5, Held::CTRL | Held::ALT).as_deref(),
            Some("Ctrl+Alt+F5")
        );
        for (named, text) in [(Named::F1, "F1"), (Named::F12, "F12"), (Named::F35, "F35")] {
            assert_eq!(
                combination(&keyboard::Key::Named(named), Held::empty()).as_deref(),
                Some(text),
                "F-keys stand alone, named as xkb names them"
            );
        }
        assert_eq!(
            combination(&keyboard::Key::Named(Named::PageUp), Held::LOGO).as_deref(),
            Some("Super+Page_Up")
        );
        assert_eq!(
            combination(&keyboard::Key::Named(Named::BrightnessUp), Held::empty()).as_deref(),
            Some("XF86MonBrightnessUp")
        );
        assert_eq!(
            combination(&letter, Held::empty()),
            None,
            "needs a modifier"
        );
        assert_eq!(combination(&letter, Held::SHIFT), None, "typing");
        assert_eq!(combination(&f5, Held::empty()).as_deref(), Some("F5"));
        assert_eq!(
            combination(&keyboard::Key::Named(Named::Super), Held::LOGO),
            None,
            "a lone modifier keeps waiting"
        );
        assert_eq!(
            combination(&keyboard::Key::Named(Named::Enter), Held::LOGO),
            None
        );
        assert_eq!(
            combination(&keyboard::Key::Character("+".into()), Held::LOGO),
            None
        );
    }
}
