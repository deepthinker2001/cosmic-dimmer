//! The opt-in writer of COSMIC custom shortcuts that call the applet's D-Bus
//! interface. It reads the user's custom shortcuts strictly and refuses
//! anything it cannot read exactly or a combination already in use, so it
//! never loses or replaces one of the user's own bindings.

use std::collections::HashMap;

use cosmic::cosmic_config::{self, ConfigGet, ConfigSet};
use cosmic_settings_config::{Action, Binding, Shortcuts, shortcuts};

use super::{Key, Keymap};
use crate::{fl, legacy};

/// The result of turning the shortcuts on or off.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Changed {
    /// This many bindings were written.
    Written(usize),
    /// Both were already there.
    Present,
    /// This many of the applet's bindings were removed.
    Removed(usize),
}

/// What the shortcuts config says now.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Current {
    /// Whether any of the applet's bindings is set.
    pub on: bool,
    /// The bound combinations, which COSMIC Settings can also change.
    pub raise: Option<String>,
    pub lower: Option<String>,
}

/// Why the shortcuts were not changed. Nothing is written in any case.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refused {
    /// A binding is already used for something else; it is left alone.
    Taken(String),
    /// The custom shortcuts could not be read exactly (for example an action
    /// this build does not know); rewriting them could lose the user's.
    Unreadable(String),
    /// The combination itself cannot be used as a shortcut.
    Invalid(String),
    Write(String),
}

/// The shortcut for `key` on the combination `text`; `None` for a key that
/// has no shortcut (Reset) or text that is not a combination.
fn binding_for(key: Key, text: &str) -> Option<Binding> {
    // Shown in COSMIC Settings' shortcut list. Matching compares only the
    // keys and modifiers, so a translated description is safe.
    let description = match key {
        Key::Raise => fl!("shortcut-description-raise"),
        Key::Lower => fl!("shortcut-description-lower"),
        Key::Reset => return None,
    };
    let mut binding = Binding::from_str_partial(text).ok()?;
    binding.description = Some(description);
    Some(binding)
}

/// The custom bindings this applet adds.
fn wanted(keymap: &Keymap) -> Vec<(Binding, Action)> {
    [Key::Raise, Key::Lower]
        .into_iter()
        .filter_map(|key| {
            let binding = binding_for(key, keymap.text(key)?)?;
            Some((binding, Action::Spawn(key.command())))
        })
        .collect()
}

fn is_ours(action: &Action) -> bool {
    [Key::Raise, Key::Lower]
        .iter()
        .any(|key| *action == Action::Spawn(key.command()))
}

/// The user's custom shortcuts, read strictly: an action this build does not
/// know fails the read instead of being turned into `Disable` (as `Shortcuts`'
/// lenient reader does), so a rewrite can never lose a newer COSMIC's bindings.
fn read_custom(config: &cosmic_config::Config) -> Result<HashMap<Binding, Action>, Refused> {
    match config.get("custom") {
        Ok(custom) => Ok(custom),
        Err(error) if !error.is_err() => Ok(HashMap::new()),
        Err(error) => Err(Refused::Unreadable(error.to_string())),
    }
}

/// What the custom shortcuts hold for the applet. Unreadable shortcuts read
/// as off. Blocking file I/O: call it off the UI thread.
#[must_use]
pub fn current(config: &cosmic_config::Config) -> Current {
    let custom = read_custom(config).unwrap_or_default();
    let find = |key: Key| {
        let action = Action::Spawn(key.command());
        custom
            .iter()
            .find_map(|(binding, bound)| (*bound == action).then(|| binding.to_string()))
    };
    let (raise, lower) = (find(Key::Raise), find(Key::Lower));
    Current {
        on: raise.is_some() || lower.is_some(),
        raise,
        lower,
    }
}

/// Remove the applet's bindings from the custom shortcuts, on whatever keys
/// they sit. A binding that runs something else is the user's and stays.
/// Blocking file I/O: call it off the UI thread.
///
/// # Errors
/// [`Refused`], with nothing written.
pub fn remove_shortcuts(config: &cosmic_config::Config) -> Result<Changed, Refused> {
    let mut custom = read_custom(config)?;
    let before = custom.len();
    custom.retain(|_, action| !is_ours(action));
    let removed = before - custom.len();
    if removed > 0 {
        config
            .set("custom", custom)
            .map_err(|error| Refused::Write(error.to_string()))?;
    }
    Ok(Changed::Removed(removed))
}

/// Point `key` at the combination `text`. While the shortcuts are off this only
/// checks that the combination is free; `keymap` keeps it for when they go on.
/// Never takes a combination that is in use, including by the other key.
/// Blocking file I/O: call it off the UI thread.
///
/// # Errors
/// [`Refused`], with nothing written.
pub fn rebind(
    config: &cosmic_config::Config,
    key: Key,
    text: &str,
    keymap: &Keymap,
) -> Result<(), Refused> {
    let mut custom = read_custom(config)?;
    let enabled = custom.values().any(is_ours);
    let Some(binding) = binding_for(key, text) else {
        return Err(Refused::Invalid(format!("not a combination: {text}")));
    };
    let other = match key {
        Key::Raise => Key::Lower,
        Key::Lower => Key::Raise,
        Key::Reset => return Err(Refused::Invalid("Reset has no shortcut".into())),
    };
    if keymap
        .text(other)
        .and_then(|other| Binding::from_str_partial(other).ok())
        .is_some_and(|other| other == binding)
    {
        return Err(Refused::Taken(binding.to_string()));
    }
    let defaults = config.get::<Shortcuts>("defaults").unwrap_or_default();
    let action = Action::Spawn(key.command());
    let existing = custom.get(&binding).or_else(|| defaults.0.get(&binding));
    if existing.is_some_and(|existing| *existing != action) {
        return Err(Refused::Taken(binding.to_string()));
    }
    if enabled {
        custom.retain(|_, bound| *bound != action);
        custom.insert(binding, action);
        config
            .set("custom", custom)
            .map_err(|error| Refused::Write(error.to_string()))?;
    }
    Ok(())
}

/// Add the combinations in `keymap` to the user's custom shortcuts in `config`
/// (normally `Config::new(shortcuts::ID, 1)`). Never replaces a binding the
/// user or the defaults already have, and never rewrites custom shortcuts it
/// could not read exactly. Blocking file I/O: call it off the UI thread.
///
/// # Errors
/// [`Refused`], with nothing written.
pub fn add_shortcuts(config: &cosmic_config::Config, keymap: &Keymap) -> Result<Changed, Refused> {
    let mut custom = read_custom(config)?;
    // Lenient is fine here: the defaults are only read.
    let defaults = config.get::<Shortcuts>("defaults").unwrap_or_default();
    let wanted = wanted(keymap);
    if wanted.len() != 2 {
        return Err(Refused::Invalid("brightness key names unknown".into()));
    }
    let mut written = 0;
    for (binding, action) in wanted {
        let existing = custom.get(&binding).or_else(|| defaults.0.get(&binding));
        match existing {
            Some(existing) if *existing == action => {}
            Some(_) => return Err(Refused::Taken(binding.to_string())),
            None => {
                custom.insert(binding, action);
                written += 1;
            }
        }
    }
    if written == 0 {
        return Ok(Changed::Present);
    }
    config
        .set("custom", custom)
        .map_err(|error| Refused::Write(error.to_string()))?;
    Ok(Changed::Written(written))
}

/// Point the applet's shortcuts from before 0.2.0, which call the old D-Bus
/// name, at the new one, on the same keys. Returns how many were changed.
/// Blocking file I/O: call it off the UI thread.
///
/// # Errors
/// [`Refused`], with nothing written.
pub fn rename_legacy(config: &cosmic_config::Config) -> Result<usize, Refused> {
    let mut custom = read_custom(config)?;
    let mut renamed = 0;
    for action in custom.values_mut() {
        for key in [Key::Raise, Key::Lower] {
            if *action == Action::Spawn(legacy::command(key)) {
                *action = Action::Spawn(key.command());
                renamed += 1;
            }
        }
    }
    if renamed > 0 {
        config
            .set("custom", custom)
            .map_err(|error| Refused::Write(error.to_string()))?;
    }
    Ok(renamed)
}

/// The user's COSMIC shortcuts config.
///
/// # Errors
/// The cosmic-config error if the config directory cannot be opened.
pub fn shortcuts_config() -> Result<cosmic_config::Config, cosmic_config::Error> {
    cosmic_config::Config::new(shortcuts::ID, 1)
}

/// Run `change` on the user's COSMIC shortcuts config. Blocking file I/O:
/// call it off the UI thread.
///
/// # Errors
/// [`Refused::Write`] if the config cannot be opened, else `change`'s error.
pub fn with_shortcuts<T>(
    change: impl FnOnce(&cosmic_config::Config) -> Result<T, Refused>,
) -> Result<T, Refused> {
    let config = shortcuts_config().map_err(|error| Refused::Write(error.to_string()))?;
    change(&config)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    struct TempConfig(PathBuf, cosmic_config::Config);

    impl TempConfig {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "cosmic-ext-dimmer-keys-{tag}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            let config =
                cosmic_config::Config::with_custom_path(shortcuts::ID, 1, root.clone()).unwrap();
            Self(root, config)
        }

        fn custom_path(&self) -> PathBuf {
            self.0
                .join("cosmic")
                .join(shortcuts::ID)
                .join("v1")
                .join("custom")
        }

        fn write_custom(&self, text: &str) {
            let path = self.custom_path();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    impl Drop for TempConfig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn both_default_key_names_resolve() {
        assert_eq!(
            wanted(&Keymap::default()).len(),
            2,
            "both key names resolve in xkbcommon"
        );
    }

    #[test]
    fn adds_once_and_keeps_existing_shortcuts() {
        let temp = TempConfig::new("add");
        temp.write_custom("{ (modifiers: [Super, Alt], key: \"t\"): Spawn(\"cosmic-term\") }");
        assert_eq!(
            add_shortcuts(&temp.1, &Keymap::default()),
            Ok(Changed::Written(2))
        );
        assert_eq!(
            add_shortcuts(&temp.1, &Keymap::default()),
            Ok(Changed::Present),
            "idempotent"
        );
        let custom: HashMap<Binding, Action> = temp.1.get("custom").unwrap();
        assert_eq!(custom.len(), 3);
        let terminal = Binding::from_str_partial("Super+Alt+t").unwrap();
        assert_eq!(
            custom.get(&terminal),
            Some(&Action::Spawn("cosmic-term".into()))
        );
        for (binding, action) in wanted(&Keymap::default()) {
            assert_eq!(custom.get(&binding), Some(&action));
        }
    }

    #[test]
    fn removing_takes_only_the_applets_bindings() {
        let temp = TempConfig::new("remove");
        temp.write_custom("{ (modifiers: [Super, Alt], key: \"t\"): Spawn(\"cosmic-term\") }");
        assert!(!current(&temp.1).on);
        assert_eq!(remove_shortcuts(&temp.1), Ok(Changed::Removed(0)));
        add_shortcuts(&temp.1, &Keymap::default()).unwrap();
        assert!(current(&temp.1).on);
        assert_eq!(remove_shortcuts(&temp.1), Ok(Changed::Removed(2)));
        assert!(!current(&temp.1).on);
        let custom: HashMap<Binding, Action> = temp.1.get("custom").unwrap();
        assert_eq!(custom.len(), 1, "the user's shortcut stays");
        assert_eq!(
            add_shortcuts(&temp.1, &Keymap::default()),
            Ok(Changed::Written(2)),
            "and back on"
        );
    }

    #[test]
    fn removing_leaves_a_rebound_key_alone() {
        let temp = TempConfig::new("rebound");
        add_shortcuts(&temp.1, &Keymap::default()).unwrap();
        let raise = Binding::from_str_partial(&Keymap::default().raise).unwrap();
        let mut custom: HashMap<Binding, Action> = temp.1.get("custom").unwrap();
        custom.insert(raise.clone(), Action::Spawn("mine".into()));
        temp.1.set("custom", custom).unwrap();
        assert_eq!(remove_shortcuts(&temp.1), Ok(Changed::Removed(1)));
        let custom: HashMap<Binding, Action> = temp.1.get("custom").unwrap();
        assert_eq!(custom.get(&raise), Some(&Action::Spawn("mine".into())));
    }

    #[test]
    fn rebinding_moves_the_key_and_keeps_everything_else() {
        let temp = TempConfig::new("rebind");
        let keymap = Keymap::default();
        temp.write_custom("{ (modifiers: [Super, Alt], key: \"t\"): Spawn(\"cosmic-term\") }");
        add_shortcuts(&temp.1, &keymap).unwrap();
        rebind(&temp.1, Key::Raise, "Ctrl+Alt+Up", &keymap).unwrap();
        let now = current(&temp.1);
        assert!(now.on);
        assert_eq!(now.raise.as_deref(), Some("Ctrl+Alt+Up"));
        assert_eq!(now.lower.as_deref(), Some("Super+F1"));
        let custom: HashMap<Binding, Action> = temp.1.get("custom").unwrap();
        assert_eq!(
            custom.len(),
            3,
            "the old combination went, the user's stays"
        );
        assert_eq!(remove_shortcuts(&temp.1), Ok(Changed::Removed(2)));
        assert_eq!(current(&temp.1), Current::default());
    }

    #[test]
    fn rebinding_while_off_writes_nothing_and_the_new_keys_go_on_later() {
        let temp = TempConfig::new("rebind-off");
        let keymap = Keymap {
            raise: "Ctrl+Alt+Up".into(),
            ..Keymap::default()
        };
        rebind(&temp.1, Key::Raise, "Ctrl+Alt+Up", &Keymap::default()).unwrap();
        assert!(!temp.custom_path().exists());
        add_shortcuts(&temp.1, &keymap).unwrap();
        assert_eq!(current(&temp.1).raise.as_deref(), Some("Ctrl+Alt+Up"));
    }

    #[test]
    fn a_capture_can_release_the_keys_and_take_one_of_them_back() {
        let temp = TempConfig::new("recapture");
        let keymap = Keymap {
            raise: "F1".into(),
            lower: "F2".into(),
        };
        add_shortcuts(&temp.1, &keymap).unwrap();
        assert_eq!(remove_shortcuts(&temp.1), Ok(Changed::Removed(2)));
        rebind(&temp.1, Key::Raise, "F3", &keymap).unwrap();
        rebind(&temp.1, Key::Raise, "F1", &keymap).unwrap();
        let moved = Keymap {
            raise: "F3".into(),
            ..keymap
        };
        assert_eq!(add_shortcuts(&temp.1, &moved), Ok(Changed::Written(2)));
        let now = current(&temp.1);
        assert_eq!(now.raise.as_deref(), Some("F3"));
        assert_eq!(now.lower.as_deref(), Some("F2"));
    }

    #[test]
    fn rebinding_refuses_a_combination_in_use() {
        let temp = TempConfig::new("rebind-taken");
        let keymap = Keymap::default();
        temp.write_custom("{ (modifiers: [Super, Alt], key: \"t\"): Spawn(\"cosmic-term\") }");
        add_shortcuts(&temp.1, &keymap).unwrap();
        let original = std::fs::read_to_string(temp.custom_path()).unwrap();
        for text in ["Super+Alt+t", "Super+F1"] {
            assert!(
                matches!(
                    rebind(&temp.1, Key::Raise, text, &keymap),
                    Err(Refused::Taken(_))
                ),
                "{text}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(temp.custom_path()).unwrap(),
            original,
            "nothing written"
        );
    }

    #[test]
    fn removing_refuses_shortcuts_it_cannot_read_exactly() {
        let temp = TempConfig::new("remove-unknown");
        let original = "{ (modifiers: [Super], key: \"x\"): SomeFutureAction }";
        temp.write_custom(original);
        assert!(matches!(
            remove_shortcuts(&temp.1),
            Err(Refused::Unreadable(_))
        ));
        assert!(!current(&temp.1).on);
        assert_eq!(
            std::fs::read_to_string(temp.custom_path()).unwrap(),
            original
        );
    }

    #[test]
    fn absent_custom_shortcuts_are_created() {
        let temp = TempConfig::new("absent");
        assert_eq!(
            add_shortcuts(&temp.1, &Keymap::default()),
            Ok(Changed::Written(2))
        );
        assert!(temp.custom_path().is_file());
    }

    #[test]
    fn a_taken_binding_is_never_replaced() {
        let temp = TempConfig::new("taken");
        // Written through the API, not as text: see the secret scanner note.
        let mine = HashMap::from([(
            Binding::from_str_partial(&Keymap::default().raise).unwrap(),
            Action::Spawn("mine".into()),
        )]);
        temp.1.set("custom", mine).unwrap();
        let original = std::fs::read_to_string(temp.custom_path()).unwrap();
        assert!(matches!(
            add_shortcuts(&temp.1, &Keymap::default()),
            Err(Refused::Taken(_))
        ));
        assert_eq!(
            std::fs::read_to_string(temp.custom_path()).unwrap(),
            original,
            "nothing written"
        );
    }

    #[test]
    fn shortcuts_from_before_the_rename_move_to_the_new_name() {
        let temp = TempConfig::new("legacy");
        temp.write_custom(&format!(
            "{{ (modifiers: [Super], key: \"F2\"): Spawn({:?}), \
             (modifiers: [Super], key: \"F1\"): Spawn({:?}), \
             (modifiers: [Super, Alt], key: \"t\"): Spawn(\"cosmic-term\") }}",
            legacy::command(Key::Raise),
            legacy::command(Key::Lower),
        ));
        assert_eq!(rename_legacy(&temp.1), Ok(2));
        let now = current(&temp.1);
        assert_eq!(now.raise.as_deref(), Some("Super+F2"), "same keys");
        assert_eq!(now.lower.as_deref(), Some("Super+F1"));
        let custom: HashMap<Binding, Action> = temp.1.get("custom").unwrap();
        assert_eq!(custom.len(), 3, "the user's shortcut stays");
        assert_eq!(rename_legacy(&temp.1), Ok(0), "once");
    }

    #[test]
    fn unknown_actions_refuse_rather_than_rewrite() {
        let temp = TempConfig::new("unknown");
        let original = "{ (modifiers: [Super], key: \"x\"): SomeFutureAction }";
        temp.write_custom(original);
        assert!(matches!(
            add_shortcuts(&temp.1, &Keymap::default()),
            Err(Refused::Unreadable(_))
        ));
        assert_eq!(
            std::fs::read_to_string(temp.custom_path()).unwrap(),
            original
        );
    }
}
