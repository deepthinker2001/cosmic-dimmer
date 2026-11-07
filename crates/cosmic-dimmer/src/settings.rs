//! Applet-wide settings: the floors every monitor uses, how long to wait for a
//! backend, how far a brightness key moves, and auto dimming.
//!
//! Stored as COSMIC applets store their settings: one cosmic-config key per
//! field (`~/.config/cosmic/<APP_ID>/v1/key_step` and so on), next to the
//! per-monitor entries that `dimmer_core::store` writes, and watched so a
//! change made elsewhere reaches the running applet. Loading never fails: a
//! key that is missing or unreadable takes its default, and every value is
//! clamped, so no file can disable a floor.
//!
//! Earlier versions kept every field in one `settings` key. [`Settings::load`]
//! moves that over once, when none of the per-field keys exist yet.

use crate::keys::{Keymap, is_binding};
use crate::location;
use std::time::Duration;

use cosmic::cosmic_config::{
    self, ConfigGet, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry,
};
use dimmer_core::Floors;
use dimmer_core::sun::Location;
use serde::{Deserialize, Serialize};

/// The single key earlier versions kept all settings in.
const LEGACY_ENTRY: &str = "settings";
/// The per-field keys, as the derive names them: the fields of [`Settings`].
const FIELDS: [&str; 9] = [
    "hardware_floor",
    "alpha_floor",
    "effective_floor",
    "timeout_ms",
    "key_step",
    "raise_binding",
    "lower_binding",
    "auto_dimming",
    "location",
];

/// Bounds of the backend timeout, in milliseconds. DDC/CI writes take tens of
/// milliseconds when a display answers; a display that does not answer
/// should not hold a write for long.
pub const TIMEOUT_MS_MIN: u32 = 250;
pub const TIMEOUT_MS_MAX: u32 = 10_000;
pub const TIMEOUT_MS_DEFAULT: u32 = 2_000;

/// Bounds of the brightness-key step, in percent. The step moves in whole
/// multiples of [`STEP_MIN`].
pub const STEP_MIN: u8 = 10;
pub const STEP_MAX: u8 = 50;
pub const STEP_DEFAULT: u8 = 10;

/// Applet-wide settings, as written to disk. Numbers are wide signed fields
/// so an out-of-range one is clamped to its bound rather than replaced by
/// the default.
#[derive(Clone, CosmicConfigEntry, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[version = 1]
#[serde(default)]
pub struct Settings {
    pub hardware_floor: i64,
    pub alpha_floor: i64,
    pub effective_floor: i64,
    /// How long one backend call (discovery or a write) may take.
    pub timeout_ms: i64,
    /// How far one brightness key press moves, in percent.
    pub key_step: i64,
    /// The brightness shortcuts' combinations, kept while they are off.
    pub raise_binding: String,
    pub lower_binding: String,
    /// Follow each display's auto-dimming profile through the day.
    pub auto_dimming: bool,
    /// Typed coordinates for sunrise and sunset, `latitude, longitude` in
    /// degrees; empty to use the time zone's reference city.
    pub location: String,
}

impl Default for Settings {
    fn default() -> Self {
        let floors = Floors::default();
        let keymap = Keymap::default();
        Self {
            hardware_floor: floors.hardware().into(),
            alpha_floor: floors.alpha().into(),
            effective_floor: floors.effective().into(),
            timeout_ms: TIMEOUT_MS_DEFAULT.into(),
            key_step: STEP_DEFAULT.into(),
            raise_binding: keymap.raise,
            lower_binding: keymap.lower,
            auto_dimming: false,
            location: String::new(),
        }
    }
}

fn narrow(value: i64) -> u8 {
    u8::try_from(value.clamp(0, 255)).unwrap_or(0)
}

impl Settings {
    /// The floors, clamped to dimmer-core's hard minimums.
    #[must_use]
    pub fn floors(&self) -> Floors {
        Floors::clamped(
            narrow(self.hardware_floor),
            narrow(self.alpha_floor),
            narrow(self.effective_floor),
        )
    }

    pub fn set_floors(&mut self, floors: Floors) {
        self.hardware_floor = floors.hardware().into();
        self.alpha_floor = floors.alpha().into();
        self.effective_floor = floors.effective().into();
    }

    /// The backend timeout, within [`TIMEOUT_MS_MIN`]..=[`TIMEOUT_MS_MAX`].
    #[must_use]
    pub fn timeout_ms(&self) -> u32 {
        u32::try_from(
            self.timeout_ms
                .clamp(TIMEOUT_MS_MIN.into(), TIMEOUT_MS_MAX.into()),
        )
        .unwrap_or(TIMEOUT_MS_DEFAULT)
    }

    #[must_use]
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms().into())
    }

    /// The brightness-key step, within [`STEP_MIN`]..=[`STEP_MAX`] and rounded
    /// to a multiple of [`STEP_MIN`].
    #[must_use]
    pub fn key_step(&self) -> u8 {
        let step = narrow(self.key_step).clamp(STEP_MIN, STEP_MAX);
        (step + STEP_MIN / 2) / STEP_MIN * STEP_MIN
    }

    /// The shortcut combinations, each replaced by its default if unusable.
    #[must_use]
    pub fn keymap(&self) -> Keymap {
        let fallback = Keymap::default();
        let usable = |text: &String| is_binding(text).then(|| text.clone());
        Keymap {
            raise: usable(&self.raise_binding).unwrap_or(fallback.raise),
            lower: usable(&self.lower_binding).unwrap_or(fallback.lower),
        }
    }

    pub fn set_keymap(&mut self, keymap: &Keymap) {
        self.raise_binding.clone_from(&keymap.raise);
        self.lower_binding.clone_from(&keymap.lower);
    }

    /// The typed location, if it parses.
    #[must_use]
    pub fn typed_location(&self) -> Option<Location> {
        location::parse_coordinates(&self.location)
    }

    /// The same settings with every value in bounds.
    #[must_use]
    pub fn clamped(&self) -> Self {
        let mut settings = Self {
            timeout_ms: self.timeout_ms().into(),
            key_step: self.key_step().into(),
            auto_dimming: self.auto_dimming,
            location: if self.typed_location().is_some() {
                self.location.trim().to_owned()
            } else {
                String::new()
            },
            ..Self::default()
        };
        settings.set_floors(self.floors());
        settings.set_keymap(&self.keymap());
        settings
    }

    /// The names of the fields that differ between `self` and `other`.
    fn differences(&self, other: &Self) -> Vec<&'static str> {
        [
            (
                "hardware_floor",
                self.hardware_floor != other.hardware_floor,
            ),
            ("alpha_floor", self.alpha_floor != other.alpha_floor),
            (
                "effective_floor",
                self.effective_floor != other.effective_floor,
            ),
            ("timeout_ms", self.timeout_ms != other.timeout_ms),
            ("key_step", self.key_step != other.key_step),
            ("raise_binding", self.raise_binding != other.raise_binding),
            ("lower_binding", self.lower_binding != other.lower_binding),
            ("auto_dimming", self.auto_dimming != other.auto_dimming),
            ("location", self.location != other.location),
        ]
        .into_iter()
        .filter_map(|(name, differs)| differs.then_some(name))
        .collect()
    }

    /// Load from the applet's config, falling back to defaults key by key;
    /// never fails. Settings in the earlier single-key form are moved over.
    #[must_use]
    pub fn load(config: &cosmic_config::Config) -> Self {
        if let Some(migrated) = migrate(config) {
            return migrated;
        }
        Self::read(config)
    }

    /// The settings as the per-field files hold them now, missing or
    /// unreadable ones at their defaults, clamped.
    #[must_use]
    pub fn read(config: &cosmic_config::Config) -> Self {
        let settings = match Self::get_entry(config) {
            Ok(settings) => settings,
            Err((errors, partial)) => {
                // A missing key is normal (a new setting, or a first run).
                for error in errors.iter().filter(|error| error.is_err()) {
                    tracing::warn!(%error, "a saved setting is unreadable; using its default");
                }
                partial
            }
        };
        settings.checked()
    }

    /// The same settings clamped, logging which fields had to change.
    #[must_use]
    pub fn checked(&self) -> Self {
        let clamped = self.clamped();
        let changed = self.differences(&clamped);
        if !changed.is_empty() {
            // Names only: `location` holds the user's coordinates.
            tracing::warn!(?changed, "saved settings out of bounds; corrected");
        }
        clamped
    }

    /// Save, clamped.
    ///
    /// # Errors
    /// The cosmic-config error when a write fails.
    pub fn save(&self, config: &cosmic_config::Config) -> Result<(), cosmic_config::Error> {
        self.clamped().write_entry(config)
    }
}

/// Move settings from the earlier single `settings` key to one key per field,
/// once: only while none of the per-field keys exist, so it can never
/// overwrite settings saved since. The old key is left as it was.
fn migrate(config: &cosmic_config::Config) -> Option<Settings> {
    let any_saved = FIELDS.iter().any(|key| {
        // Any stored value counts, readable or not: only absence is "not saved".
        !matches!(
            config.get::<serde::de::IgnoredAny>(key),
            Err(error) if !error.is_err()
        )
    });
    if any_saved {
        return None;
    }
    let legacy = config.get::<Settings>(LEGACY_ENTRY).ok()?.checked();
    match legacy.save(config) {
        Ok(()) => tracing::info!("settings moved to one key per field"),
        Err(error) => tracing::warn!(%error, "settings not moved; using them as read"),
    }
    Some(legacy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_in_bounds_and_match_core() {
        let settings = Settings::default();
        assert_eq!(settings.clamped(), settings);
        assert_eq!(settings.floors(), Floors::PROVISIONAL_DEFAULT);
        assert_eq!(settings.timeout(), Duration::from_secs(2));
        assert_eq!(settings.key_step(), 10);
    }

    #[test]
    fn out_of_range_values_clamp() {
        let settings = Settings {
            hardware_floor: -5,
            alpha_floor: 0,
            effective_floor: 1_000,
            timeout_ms: 0,
            key_step: 900,
            ..Settings::default()
        };
        assert_eq!(
            settings.floors().hardware(),
            Floors::HARD_MINIMUM.hardware()
        );
        assert_eq!(settings.floors().alpha(), Floors::HARD_MINIMUM.alpha());
        assert_eq!(settings.floors().effective(), 100);
        assert_eq!(settings.timeout_ms(), TIMEOUT_MS_MIN);
        assert_eq!(settings.key_step(), STEP_MAX);
        let settings = Settings {
            raise_binding: "not+a+key".into(),
            lower_binding: "Ctrl+Alt+Down".into(),
            ..Settings::default()
        };
        assert_eq!(settings.keymap().raise, Keymap::default().raise);
        assert_eq!(settings.keymap().lower, "Ctrl+Alt+Down");
        for (saved, step) in [(1, 10), (14, 10), (15, 20), (25, 30), (47, 50)] {
            let settings = Settings {
                key_step: saved,
                ..Settings::default()
            };
            assert_eq!(settings.key_step(), step, "saved {saved}");
        }
        let clamped = settings.clamped();
        assert_eq!(clamped.clamped(), clamped, "clamping is stable");
        let settings = Settings {
            location: "somewhere nice".into(),
            ..Settings::default()
        };
        assert_eq!(
            settings.clamped().location,
            "",
            "unparsable location dropped"
        );
        assert_eq!(settings.differences(&settings.clamped()), ["location"]);
    }

    #[test]
    fn round_trip_through_cosmic_config() {
        let root =
            std::env::temp_dir().join(format!("cosmic-dimmer-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let config =
            cosmic_config::Config::with_custom_path("dimmer-applet-test", 1, root.clone()).unwrap();
        assert_eq!(Settings::load(&config), Settings::default(), "absent");
        let mut settings = Settings {
            timeout_ms: 750,
            key_step: 30,
            auto_dimming: true,
            location: "39.74, -104.99".into(),
            ..Settings::default()
        };
        settings.set_floors(Floors::clamped(15, 25, 12));
        settings.save(&config).unwrap();
        assert_eq!(Settings::load(&config), settings);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn temp_config(name: &str) -> (std::path::PathBuf, cosmic_config::Config) {
        let root = std::env::temp_dir().join(format!(
            "cosmic-dimmer-settings-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let config =
            cosmic_config::Config::with_custom_path("dimmer-applet-test", 1, root.clone()).unwrap();
        (root, config)
    }

    #[test]
    fn single_key_settings_move_to_one_key_per_field_once() {
        use cosmic_config::ConfigSet;
        let (root, config) = temp_config("migrate");
        let legacy = Settings {
            key_step: 30,
            auto_dimming: true,
            ..Settings::default()
        };
        config.set(LEGACY_ENTRY, &legacy).unwrap();
        assert_eq!(Settings::load(&config), legacy, "moved over");
        assert_eq!(config.get::<i64>("key_step").unwrap(), 30, "per-field key");

        // Once moved, the old key is never read again.
        let newer = Settings {
            key_step: 50,
            ..legacy.clone()
        };
        newer.save(&config).unwrap();
        assert_eq!(Settings::load(&config), newer);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn one_bad_key_only_drops_that_setting() {
        let (root, config) = temp_config("bad-key");
        let settings = Settings {
            key_step: 30,
            ..Settings::default()
        };
        settings.save(&config).unwrap();
        let path = root
            .join("cosmic")
            .join("dimmer-applet-test")
            .join("v1")
            .join("timeout_ms");
        std::fs::write(path, "not a number").unwrap();
        let loaded = Settings::load(&config);
        assert_eq!(loaded.key_step, 30, "other keys kept");
        assert_eq!(loaded.timeout_ms(), TIMEOUT_MS_DEFAULT);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_field_list_matches_the_struct() {
        let names = Settings::default().differences(&Settings {
            hardware_floor: -1,
            alpha_floor: -1,
            effective_floor: -1,
            timeout_ms: -1,
            key_step: -1,
            raise_binding: "x".into(),
            lower_binding: "x".into(),
            auto_dimming: true,
            location: "x".into(),
        });
        assert_eq!(names, FIELDS);
    }
}
