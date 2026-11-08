//! The brightness shortcuts in COSMIC's custom shortcuts, as the popup shows
//! and changes them: the on/off toggle, and capturing a new key combination
//! for a key.
//!
//! The config work runs off the UI thread through `keys`; this module keeps
//! what the popup knows and reacts to the results. While a capture waits, the
//! applet's own shortcuts are released, because COSMIC keeps a bound key from
//! the popup; they are written back when the capture ends.

use cosmic::app::Task;
use cosmic::iced::{self, Subscription};
use cosmic::widget::{button, spin_button, text, toggler};
use cosmic::{Element, applet};

use crate::app::{Dimmer, Message};
use crate::view::{percent, setting_row};
use crate::{fl, keys, settings};

/// Whether the brightness-key D-Bus service is up.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum KeyService {
    #[default]
    Starting,
    Ready,
    Failed(String),
}

/// What the popup knows about the shortcuts.
#[derive(Debug, Default)]
pub struct Shortcuts {
    pub service: KeyService,
    /// Whether the shortcuts are set, as last read.
    pub on: bool,
    /// Why the last change failed. Success shows in the toggle.
    pub refused: Option<keys::Refused>,
    /// The key waiting for its new combination.
    pub capturing: Option<keys::Key>,
    /// Where the release of the shortcuts for a capture stands.
    release: Release,
}

/// The release of the applet's own shortcuts while a capture waits.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Release {
    #[default]
    Idle,
    Running,
    /// Running, and the capture has already ended: restore when it is done.
    RunningThenRestore,
}

#[derive(Clone, Debug)]
pub enum ShortcutMessage {
    /// The toggle: add the shortcuts, or remove them.
    Toggle(bool),
    /// What COSMIC's custom shortcuts hold, read when the popup opens.
    Read(keys::Current),
    /// Wait for the next key combination for this key.
    Capture(keys::Key),
    /// A key pressed while a capture waits.
    Captured(iced::keyboard::Key, iced::keyboard::Modifiers),
    /// A captured combination was written, or refused.
    Rebound(keys::Key, String, Result<(), keys::Refused>),
    /// The shortcuts were released for a capture, or could not be.
    Released(Option<keys::Refused>),
    /// The toggle's change was written, or refused.
    Done(Result<keys::Changed, keys::Refused>),
}

/// Run blocking shortcuts-config work off the UI thread.
fn run(work: impl FnOnce() -> ShortcutMessage + Send + 'static) -> Task<Message> {
    cosmic::task::future(async move { Message::Shortcuts(work()) })
}

/// Read the shortcuts again: COSMIC Settings can change them too.
pub fn read() -> Task<Message> {
    run(|| {
        ShortcutMessage::Read(
            keys::shortcuts_config()
                .map(|config| keys::current(&config))
                .unwrap_or_default(),
        )
    })
}

/// Key presses for the capture, listened to only while one is waiting.
pub fn captured_keys() -> Subscription<Message> {
    iced::event::listen_with(|event, _status, _window| match event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => Some(
            Message::Shortcuts(ShortcutMessage::Captured(key, modifiers)),
        ),
        _ => None,
    })
}

impl Dimmer {
    pub(crate) fn shortcut_message(&mut self, message: ShortcutMessage) -> Task<Message> {
        match message {
            ShortcutMessage::Toggle(on) => {
                self.shortcuts.refused = None;
                self.shortcuts.capturing = None;
                let keymap = self.settings.keymap();
                return run(move || {
                    ShortcutMessage::Done(keys::with_shortcuts(|config| {
                        if on {
                            keys::add_shortcuts(config, &keymap)
                        } else {
                            keys::remove_shortcuts(config)
                        }
                    }))
                });
            }
            ShortcutMessage::Read(current) => {
                self.shortcuts.on = current.on;
                // COSMIC Settings can move them too; its choice wins.
                let mut keymap = self.settings.keymap();
                keymap.raise = current.raise.unwrap_or(keymap.raise);
                keymap.lower = current.lower.unwrap_or(keymap.lower);
                if keymap != self.settings.keymap() {
                    self.settings.set_keymap(&keymap);
                    self.save_settings();
                }
            }
            ShortcutMessage::Capture(key) => return self.start_capture(key),
            ShortcutMessage::Captured(pressed, modifiers) => {
                return self.captured(&pressed, modifiers);
            }
            ShortcutMessage::Rebound(key, text, result) => {
                match result {
                    Ok(()) => {
                        let mut keymap = self.settings.keymap();
                        match key {
                            keys::Key::Lower => keymap.lower = text,
                            keys::Key::Raise => keymap.raise = text,
                            // Never captured: Reset has no shortcut.
                            keys::Key::Reset => {}
                        }
                        self.settings.set_keymap(&keymap);
                        self.save_settings();
                    }
                    Err(refused) => {
                        tracing::warn!(?refused, "brightness shortcut not changed");
                        self.shortcuts.refused = Some(refused);
                    }
                }
                return self.restore_shortcuts();
            }
            ShortcutMessage::Released(refused) => {
                let restore = self.shortcuts.release == Release::RunningThenRestore;
                self.shortcuts.release = Release::Idle;
                if restore {
                    return self.restore_shortcuts();
                }
                if refused.is_some() {
                    tracing::warn!(?refused, "brightness shortcuts not released");
                    self.shortcuts.refused = refused;
                }
            }
            ShortcutMessage::Done(result) => match result {
                Ok(keys::Changed::Removed(_)) => self.shortcuts.on = false,
                Ok(_) => self.shortcuts.on = true,
                Err(refused) => {
                    tracing::warn!(?refused, "brightness shortcuts not changed");
                    self.shortcuts.refused = Some(refused);
                }
            },
        }
        Task::none()
    }

    /// End a waiting capture, as when the popup closes, and put the
    /// shortcuts back.
    pub(crate) fn end_capture(&mut self) -> Task<Message> {
        if self.shortcuts.capturing.take().is_some() {
            self.restore_shortcuts()
        } else {
            Task::none()
        }
    }

    /// Wait for a new combination for `key`. With the shortcuts on, the
    /// applet's own are released first (see the module comment).
    fn start_capture(&mut self, key: keys::Key) -> Task<Message> {
        self.shortcuts.refused = None;
        let waiting = self.shortcuts.capturing.replace(key).is_some();
        if waiting || !self.shortcuts.on {
            return Task::none();
        }
        self.shortcuts.release = Release::Running;
        run(|| ShortcutMessage::Released(keys::with_shortcuts(keys::remove_shortcuts).err()))
    }

    /// A key pressed while a capture waits: Esc cancels, a usable
    /// combination is written, anything else (a lone modifier) is ignored.
    fn captured(
        &mut self,
        pressed: &iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
    ) -> Task<Message> {
        use iced::keyboard::{Key, key::Named};
        let Some(key) = self.shortcuts.capturing else {
            return Task::none();
        };
        if *pressed == Key::Named(Named::Escape) {
            self.shortcuts.capturing = None;
            return self.restore_shortcuts();
        }
        let Some(text) = keys::combination(pressed, modifiers) else {
            return Task::none();
        };
        self.shortcuts.capturing = None;
        let keymap = self.settings.keymap();
        run(move || {
            let result = keys::with_shortcuts(|config| keys::rebind(config, key, &text, &keymap));
            ShortcutMessage::Rebound(key, text, result)
        })
    }

    /// Write the shortcuts back after a capture released them. With the
    /// shortcuts off, a capture released nothing and only the keymap changes.
    ///
    /// A restore asked for while the release is still running waits for it:
    /// otherwise the release could land after the restore and leave the
    /// shortcuts off.
    fn restore_shortcuts(&mut self) -> Task<Message> {
        if !self.shortcuts.on {
            return Task::none();
        }
        if self.shortcuts.release != Release::Idle {
            self.shortcuts.release = Release::RunningThenRestore;
            return Task::none();
        }
        let keymap = self.settings.keymap();
        run(move || {
            ShortcutMessage::Done(keys::with_shortcuts(|config| {
                keys::add_shortcuts(config, &keymap)
            }))
        })
    }

    /// The settings rows: the toggle, a button per key showing its
    /// combination (click to capture a new one), the step per press, and why
    /// the last change was refused.
    pub(crate) fn shortcut_rows(&self) -> Vec<Element<'_, Message>> {
        if let KeyService::Failed(error) = &self.shortcuts.service {
            return vec![
                applet::padded_control(text::caption(fl!(
                    "keys-unavailable",
                    error = error.as_str()
                )))
                .into(),
            ];
        }
        let shortcut = |message| Message::Shortcuts(message);
        let mut rows = Vec::with_capacity(6);
        rows.push(setting_row(
            fl!("shortcuts-toggle"),
            toggler(self.shortcuts.on)
                .on_toggle(move |on| shortcut(ShortcutMessage::Toggle(on)))
                .into(),
        ));
        let keymap = self.settings.keymap();
        for (key, label, combination) in [
            (keys::Key::Lower, fl!("shortcut-lower"), keymap.lower),
            (keys::Key::Raise, fl!("shortcut-raise"), keymap.raise),
        ] {
            let shown = if self.shortcuts.capturing == Some(key) {
                fl!("shortcut-press")
            } else {
                combination.replace('+', " + ")
            };
            rows.push(setting_row(
                label,
                button::standard(shown)
                    .on_press(shortcut(ShortcutMessage::Capture(key)))
                    .into(),
            ));
        }
        if self.shortcuts.capturing.is_some() {
            rows.push(applet::padded_control(text::caption(fl!("shortcut-hint"))).into());
        }
        let step = self.settings.key_step();
        rows.push(setting_row(
            fl!("key-step"),
            spin_button(
                percent(step),
                step,
                settings::STEP_MIN,
                settings::STEP_MIN,
                settings::STEP_MAX,
                Message::KeyStep,
            )
            .into(),
        ));
        if let Some(refused) = &self.shortcuts.refused {
            rows.push(applet::padded_control(text::caption(refused_note(refused))).into());
        }
        rows
    }
}

/// Why a shortcuts change was refused, for the note under the shortcuts.
fn refused_note(refused: &keys::Refused) -> String {
    match refused {
        keys::Refused::Taken(binding) => fl!("shortcuts-taken", binding = binding.as_str()),
        keys::Refused::Unreadable(_) => fl!("shortcuts-unreadable"),
        keys::Refused::Invalid(_) => fl!("shortcuts-invalid"),
        keys::Refused::Write(error) => fl!("shortcuts-failed", error = error.as_str()),
    }
}
