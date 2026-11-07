//! Saved per-monitor settings, keyed by EDID identity, through cosmic-config.
//!
//! This is the crate's only I/O: reading and writing the applet's own config
//! files (one cosmic-config key, so one file, per monitor). It never talks to
//! a display or the compositor.
//!
//! Loading never fails and never trusts the file: a missing, unreadable or
//! malformed entry gives full brightness, and every value read is clamped into
//! [`MonitorLevels`]' invariant, so no config file can black a screen. What was
//! wrong is reported as [`LoadIssue`]s for the caller to log.

use std::fmt;

use cosmic_config::{ConfigGet, ConfigSet};
use serde::{Deserialize, Serialize};

use crate::profile::{Point, Profile};
use crate::sun::Phase;
use crate::{EdidId, Floors, MonitorLevels};

/// What every monitor's entry name starts with.
const ENTRY_PREFIX: &str = "monitor-";

/// The config version to open the applet's [`cosmic_config::Config`] with.
pub const CONFIG_VERSION: u64 = 1;

/// One monitor's entry as written to disk. Fields are wide signed integers so
/// that out-of-range numbers are read and clamped rather than rejecting the
/// whole entry; missing fields take their defaults.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct MonitorConfig {
    pub hardware: i64,
    pub alpha: i64,
    pub locked: bool,
    pub hardware_floor: i64,
    pub alpha_floor: i64,
    pub effective_floor: i64,
    /// The auto-dimming profile: `(phase, hardware, alpha)` for each point,
    /// the phase in thousandths of the sun's cycle (see [`Phase`]).
    pub auto_points: Vec<(i64, i64, i64)>,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self::new(&MonitorLevels::default(), &Profile::default())
    }
}

impl MonitorConfig {
    /// The entry for a display's levels and auto-dimming profile.
    #[must_use]
    pub fn new(levels: &MonitorLevels, profile: &Profile) -> Self {
        let floors = levels.floors();
        Self {
            hardware: levels.hardware().percent().into(),
            alpha: levels.alpha().percent().into(),
            locked: levels.locked(),
            hardware_floor: floors.hardware().into(),
            alpha_floor: floors.alpha().into(),
            effective_floor: floors.effective().into(),
            auto_points: profile
                .points()
                .iter()
                .map(|point| {
                    (
                        point.phase.thousandths().into(),
                        point.hardware.into(),
                        point.alpha.into(),
                    )
                })
                .collect(),
        }
    }

    /// The auto-dimming profile. Points with a phase outside the cycle are
    /// dropped and levels are clamped, so a bad file cannot dim the screen
    /// below what [`MonitorLevels`] allows.
    #[must_use]
    pub fn profile(&self) -> Profile {
        let narrow = |value: i64| u8::try_from(value.clamp(0, 255)).unwrap_or(0);
        Profile::from_points(
            self.auto_points
                .iter()
                .filter_map(|&(phase, hardware, alpha)| {
                    let phase = u16::try_from(phase)
                        .ok()
                        .filter(|&phase| phase < Phase::CYCLE)?;
                    Some(Point {
                        phase: Phase::new(phase),
                        hardware: narrow(hardware),
                        alpha: narrow(alpha),
                    })
                }),
        )
    }
}

/// Something wrong with a saved entry, already corrected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadIssue {
    /// The entry could not be read or parsed; full-brightness defaults used.
    Unreadable(String),
    /// A field's value was out of bounds and was changed.
    Adjusted {
        field: &'static str,
        from: i64,
        to: u8,
    },
}

impl fmt::Display for LoadIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(reason) => {
                write!(f, "saved settings unreadable, using defaults: {reason}")
            }
            Self::Adjusted { field, from, to } => {
                write!(f, "saved {field} {from} is out of bounds; using {to}")
            }
        }
    }
}

impl MonitorConfig {
    /// The levels this entry describes, brought within every floor and hard
    /// minimum, with what had to change.
    #[must_use]
    pub fn validate(&self) -> (MonitorLevels, Vec<LoadIssue>) {
        // Saturate into 0..=255 first; Floors and MonitorLevels clamp the rest.
        let narrow = |value: i64| u8::try_from(value.clamp(0, 255)).unwrap_or(0);
        let floors = Floors::clamped(
            narrow(self.hardware_floor),
            narrow(self.alpha_floor),
            narrow(self.effective_floor),
        );
        let levels = MonitorLevels::clamped(
            narrow(self.hardware),
            narrow(self.alpha),
            self.locked,
            floors,
        );
        let mut issues = Vec::new();
        for (field, from, to) in [
            ("hardware_floor", self.hardware_floor, floors.hardware()),
            ("alpha_floor", self.alpha_floor, floors.alpha()),
            ("effective_floor", self.effective_floor, floors.effective()),
            ("hardware", self.hardware, levels.hardware().percent()),
            ("alpha", self.alpha, levels.alpha().percent()),
        ] {
            if from != i64::from(to) {
                issues.push(LoadIssue::Adjusted { field, from, to });
            }
        }
        (levels, issues)
    }
}

/// A saved entry, validated.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Loaded {
    pub levels: MonitorLevels,
    pub profile: Profile,
    /// Empty when the entry was absent or entirely valid.
    pub issues: Vec<LoadIssue>,
}

/// A save that did not reach the disk.
#[derive(Debug)]
pub struct StoreError {
    pub entry: String,
    pub source: Box<cosmic_config::Error>,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "could not save settings {}: {}", self.entry, self.source)
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.source)
    }
}

/// Per-monitor settings in a cosmic-config namespace the caller opens, for
/// the applet `Config::new(APP_ID, CONFIG_VERSION)`.
pub struct MonitorStore {
    config: cosmic_config::Config,
}

impl MonitorStore {
    #[must_use]
    pub const fn new(config: cosmic_config::Config) -> Self {
        Self { config }
    }

    /// The cosmic-config entry (a file name) for a monitor.
    #[must_use]
    pub fn entry_name(id: &EdidId) -> String {
        format!("{ENTRY_PREFIX}{}", id.entry_name())
    }

    /// The monitor a cosmic-config entry name belongs to; `None` for any
    /// other entry, such as a setting.
    #[must_use]
    pub fn monitor_of(entry: &str) -> Option<EdidId> {
        EdidId::from_entry_name(entry.strip_prefix(ENTRY_PREFIX)?)
    }

    /// The monitor's saved settings, or full-brightness defaults when there
    /// are none or they cannot be read. Never fails; see [`Loaded::issues`].
    #[must_use]
    pub fn load(&self, id: &EdidId) -> Loaded {
        match self.config.get::<MonitorConfig>(&Self::entry_name(id)) {
            Ok(entry) => {
                let (levels, issues) = entry.validate();
                Loaded {
                    levels,
                    profile: entry.profile(),
                    issues,
                }
            }
            Err(error) if !error.is_err() => Loaded {
                levels: MonitorLevels::default(),
                profile: Profile::default(),
                issues: Vec::new(),
            },
            Err(error) => Loaded {
                levels: MonitorLevels::default(),
                profile: Profile::default(),
                issues: vec![LoadIssue::Unreadable(error.to_string())],
            },
        }
    }

    /// Save the monitor's levels and auto-dimming profile, atomically
    /// replacing its entry.
    ///
    /// # Errors
    /// The cosmic-config error, with the entry name, when the write fails.
    pub fn save(
        &self,
        id: &EdidId,
        levels: &MonitorLevels,
        profile: &Profile,
    ) -> Result<(), StoreError> {
        let entry = Self::entry_name(id);
        self.config
            .set(&entry, MonitorConfig::new(levels, profile))
            .map_err(|source| StoreError {
                entry,
                source: Box::new(source),
            })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::atomic::{AtomicU32, Ordering},
    };

    use super::*;

    const NAME: &str = "dimmer-core-test";

    /// A fresh config root under the system temporary folder, removed on drop.
    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            static COUNT: AtomicU32 = AtomicU32::new(0);
            let path = std::env::temp_dir().join(format!(
                "dimmer-core-store-{}-{}",
                std::process::id(),
                COUNT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }

        fn store(&self) -> MonitorStore {
            MonitorStore::new(
                cosmic_config::Config::with_custom_path(NAME, CONFIG_VERSION, self.0.clone())
                    .unwrap(),
            )
        }

        fn entry_path(&self, id: &EdidId) -> PathBuf {
            self.0
                .join("cosmic")
                .join(NAME)
                .join(format!("v{CONFIG_VERSION}"))
                .join(MonitorStore::entry_name(id))
        }

        fn write_raw(&self, id: &EdidId, text: &str) {
            let path = self.entry_path(id);
            std::fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn monitor(serial: u32) -> EdidId {
        // Made-up identities; never a real device's serial.
        EdidId::new("TST", 0x0001, serial).unwrap()
    }

    #[test]
    fn missing_entry_is_full_brightness_without_issues() {
        let root = TempRoot::new();
        let loaded = root.store().load(&monitor(1));
        assert_eq!(loaded.levels, MonitorLevels::default());
        assert_eq!(loaded.issues, []);
    }

    #[test]
    fn round_trip_per_monitor() {
        let root = TempRoot::new();
        let store = root.store();
        let mut first = MonitorLevels::new(Floors::clamped(15, 25, 12));
        let _ = first.set_hardware(70);
        let _ = first.set_alpha(45);
        first.set_locked(true);
        let mut second = MonitorLevels::default();
        let _ = second.set_alpha(60);
        store
            .save(&monitor(1), &first, &Profile::default())
            .unwrap();
        store
            .save(&monitor(2), &second, &Profile::default())
            .unwrap();

        // A new store over the same files, as after a restart.
        let reopened = root.store();
        let loaded = reopened.load(&monitor(1));
        assert_eq!(loaded.levels, first);
        assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
        assert_eq!(reopened.load(&monitor(2)).levels, second);
        assert_eq!(
            reopened.load(&monitor(3)).levels,
            MonitorLevels::default(),
            "monitors do not share entries"
        );
        assert!(root.entry_path(&monitor(1)).is_file());
    }

    #[test]
    fn malformed_entry_falls_back_to_full_brightness() {
        let root = TempRoot::new();
        for text in [
            "",
            "not ron at all",
            "(hardware: 1.5)",
            "(hardware: \"dark\")",
            "(hardware: 99999999999999999999999)",
        ] {
            root.write_raw(&monitor(1), text);
            let loaded = root.store().load(&monitor(1));
            assert_eq!(loaded.levels, MonitorLevels::default(), "{text:?}");
            assert!(
                matches!(loaded.issues.as_slice(), [LoadIssue::Unreadable(_)]),
                "{text:?}: {:?}",
                loaded.issues
            );
        }
    }

    #[test]
    fn out_of_range_values_are_clamped_and_reported() {
        let root = TempRoot::new();
        root.write_raw(
            &monitor(1),
            "(hardware: 0, alpha: -40, locked: true, hardware_floor: 0, \
             alpha_floor: 0, effective_floor: 0)",
        );
        let loaded = root.store().load(&monitor(1));
        let levels = loaded.levels;
        assert_eq!(levels.floors(), Floors::HARD_MINIMUM);
        assert!(levels.locked());
        assert!(levels.effective_percent() >= Floors::HARD_MINIMUM.effective());
        assert!(levels.hardware().percent() >= Floors::HARD_MINIMUM.hardware());
        assert!(levels.alpha().percent() >= Floors::HARD_MINIMUM.alpha());
        let fields: Vec<_> = loaded
            .issues
            .iter()
            .map(|issue| match issue {
                LoadIssue::Adjusted { field, .. } => *field,
                LoadIssue::Unreadable(reason) => panic!("unexpected: {reason}"),
            })
            .collect();
        assert_eq!(
            fields,
            [
                "hardware_floor",
                "alpha_floor",
                "effective_floor",
                "hardware",
                "alpha"
            ]
        );
    }

    #[test]
    fn effective_floor_applies_to_saved_levels() {
        let root = TempRoot::new();
        // Both levels at their own floors would give 2 % effective.
        root.write_raw(&monitor(1), "(hardware: 10, alpha: 20)");
        let loaded = root.store().load(&monitor(1));
        assert!(loaded.levels.effective_percent() >= 10);
        assert_ne!(loaded.issues, []);
    }

    #[test]
    fn missing_fields_take_defaults_and_unknown_fields_are_ignored() {
        let root = TempRoot::new();
        root.write_raw(&monitor(1), "(alpha: 50, from_the_future: true)");
        let loaded = root.store().load(&monitor(1));
        assert_eq!(loaded.levels.alpha().percent(), 50);
        assert_eq!(loaded.levels.hardware().percent(), 100);
        assert_eq!(loaded.levels.floors(), Floors::PROVISIONAL_DEFAULT);
        assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
    }

    #[test]
    fn huge_floors_are_capped_so_full_brightness_still_holds() {
        let config = MonitorConfig {
            hardware: 5,
            alpha: 5,
            locked: false,
            hardware_floor: 1000,
            alpha_floor: 1000,
            effective_floor: 1000,
            auto_points: Vec::new(),
        };
        let (levels, issues) = config.validate();
        assert_eq!(
            (levels.hardware().percent(), levels.alpha().percent()),
            (100, 100)
        );
        assert_eq!(issues.len(), 5);
    }

    #[test]
    fn profiles_round_trip_and_bad_points_are_dropped() {
        let root = TempRoot::new();
        let store = root.store();
        let mut profile = Profile::default();
        profile.learn(Phase::new(200), 100, 90);
        profile.learn(Phase::new(1400), 80, 30);
        store
            .save(&monitor(1), &MonitorLevels::default(), &profile)
            .unwrap();
        assert_eq!(root.store().load(&monitor(1)).profile, profile);

        root.write_raw(
            &monitor(2),
            "(auto_points: [(-1, 50, 50), (2000, 50, 50), (300, -5, 999)])",
        );
        let loaded = root.store().load(&monitor(2)).profile;
        let point = Point {
            phase: Phase::new(300),
            hardware: 1,
            alpha: 100,
        };
        assert_eq!(loaded.points(), [point]);
    }
}
