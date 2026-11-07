//! `cosmic-dimmer --reset`: the terminal recovery for a screen too dark
//! to use the applet.
//!
//! Overlay surfaces belong to the applet's own Wayland connection, so another
//! process cannot remove them; they go when the applet sets the level to 100 %
//! or when its connection closes. So the reset:
//!
//! 1. asks the running applet over its D-Bus interface (`Reset`) to set every
//!    display to 100 %, which removes every overlay and writes hardware 100 %;
//! 2. sets every saved level to 100 %, so the next start cannot dim again;
//! 3. when no applet answered, writes hardware 100 % itself through the DDC
//!    backend (overlays need nothing then: a stopped applet has none).
//!
//! If an applet is running but does not answer, it says how to end it
//! (`pkill`), which makes the compositor remove its overlays.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use cosmic::cosmic_config;
use dimmer_core::store::{CONFIG_VERSION, MonitorStore};
use dimmer_core::{Backend, BackendError, Level, MonitorLevels, with_timeout};

use crate::keys::{BUS_NAME, OBJECT_PATH};
use crate::localize::plain;
use crate::{APP_ID, backend, fl, settings::Settings};

/// How long to wait for the running applet to answer.
const APPLET_TIMEOUT: Duration = Duration::from_secs(3);

/// What the running applet said.
#[derive(Debug, Eq, PartialEq)]
pub enum Applet {
    /// It took the request and is setting every display to 100 %.
    Reset,
    /// No applet owns the bus name: nothing is dimmed by an overlay.
    NotRunning,
    /// It (or the bus) did not answer.
    NoAnswer(String),
}

/// Ask the running applet to reset.
pub async fn ask_applet(timeout: Duration) -> Applet {
    let connection = match zbus::Connection::session().await {
        Ok(connection) => connection,
        Err(error) => return Applet::NoAnswer(error.to_string()),
    };
    let call = connection.call_method(Some(BUS_NAME), OBJECT_PATH, Some(BUS_NAME), "Reset", &());
    match tokio::time::timeout(timeout, call).await {
        Err(_) => Applet::NoAnswer(fl!("reason-timed-out")),
        Ok(Ok(_)) => Applet::Reset,
        Ok(Err(error)) => match zbus::fdo::Error::from(error) {
            zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_) => {
                Applet::NotRunning
            }
            other => Applet::NoAnswer(other.to_string()),
        },
    }
}

/// The folder cosmic-config keeps user settings under, as `Config::new` finds
/// it: `$XDG_CONFIG_HOME` when absolute, else `$HOME/.config`.
#[must_use]
pub fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

/// Set every saved display's levels to 100 % under `config_home`, keeping
/// each display's lock, floors and auto-dimming profile, and turn auto
/// dimming off so it cannot dim the screen again. Returns how many display
/// entries were reset.
///
/// # Errors
/// The first error opening, listing or writing the applet's settings.
pub fn reset_saved(config_home: &Path) -> Result<usize, String> {
    let folder = config_home
        .join("cosmic")
        .join(APP_ID)
        .join(format!("v{CONFIG_VERSION}"));
    let entries = match std::fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.to_string()),
    };
    let config =
        cosmic_config::Config::with_custom_path(APP_ID, CONFIG_VERSION, config_home.to_owned())
            .map_err(|error| error.to_string())?;
    // Turning auto dimming off must not stop the levels below from being
    // reset: a failure is reported once they are.
    let mut settings = Settings::load(&config);
    let mut settings_error = None;
    if settings.auto_dimming {
        settings.auto_dimming = false;
        settings_error = settings.save(&config).err().map(|error| error.to_string());
    }
    let store = MonitorStore::new(config);
    let mut reset = 0;
    for entry in entries {
        let name = entry.map_err(|error| error.to_string())?.file_name();
        let Some(edid) = name.to_str().and_then(MonitorStore::monitor_of) else {
            continue;
        };
        let loaded = store.load(&edid);
        let levels = loaded.levels;
        let full = MonitorLevels::clamped(100, 100, levels.locked(), levels.floors());
        // Errors carry the entry name, which holds a serial: report the cause only.
        store
            .save(&edid, &full, &loaded.profile)
            .map_err(|error| error.source.to_string())?;
        reset += 1;
    }
    settings_error.map_or(Ok(reset), Err)
}

/// Write 100 % to every output `backend` lists. Returns how many took it and
/// how many failed.
///
/// # Errors
/// The backend's error when it cannot list its outputs.
pub async fn reset_hardware<B: Backend>(
    backend: &B,
    timeout: Duration,
) -> Result<(usize, usize), BackendError> {
    let outputs = with_timeout(timeout, backend.outputs()).await?;
    let (mut done, mut failed) = (0, 0);
    for output in outputs {
        match with_timeout(timeout, backend.set_level(&output.id, Level::FULL)).await {
            Ok(()) => done += 1,
            Err(error) => {
                tracing::warn!(display = output.name, %error, "hardware reset failed");
                failed += 1;
            }
        }
    }
    Ok((done, failed))
}

/// Print a line of terminal output, without Fluent's isolation marks.
fn out(text: impl AsRef<str>) {
    println!("{}", plain(text.as_ref()));
}

/// Print a line to stderr, without Fluent's isolation marks.
fn err(text: impl AsRef<str>) {
    eprintln!("{}", plain(text.as_ref()));
}

/// Run the reset, printing what happened; the process's exit code.
#[must_use]
pub fn run() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            err(fl!("reset-failed", error = error.to_string()));
            return ExitCode::FAILURE;
        }
    };
    let mut ok = true;

    let applet = runtime.block_on(ask_applet(APPLET_TIMEOUT));
    match &applet {
        Applet::Reset => out(fl!("reset-applet-done")),
        Applet::NotRunning => out(fl!("reset-applet-not-running")),
        Applet::NoAnswer(error) => {
            ok = false;
            err(fl!("reset-applet-no-answer", error = error.as_str()));
        }
    }

    let home = config_home();
    let timeout = home
        .as_deref()
        .and_then(|home| {
            cosmic_config::Config::with_custom_path(APP_ID, CONFIG_VERSION, home.to_owned()).ok()
        })
        .map_or_else(
            || Settings::default().timeout(),
            |config| Settings::load(&config).timeout(),
        );
    match home.as_deref().map(reset_saved) {
        Some(Ok(count)) => out(fl!("reset-saved", count = count)),
        Some(Err(error)) => {
            ok = false;
            err(fl!("reset-saved-failed", error = error));
        }
        None => {
            ok = false;
            err(fl!("reset-saved-no-home"));
        }
    }

    // A running applet writes hardware itself; two writers would contend.
    if applet != Applet::Reset {
        let hardware = backend::hardware();
        match runtime.block_on(reset_hardware(&hardware, timeout)) {
            Ok((done, 0)) => out(fl!("reset-hardware", count = done)),
            Ok((done, failed)) => {
                ok = false;
                err(fl!("reset-hardware-partial", count = done, failed = failed));
            }
            Err(error) => {
                out(fl!(
                    "reset-hardware-skipped",
                    reason = crate::view::error_reason(&error)
                ));
                // Nothing logs in this mode, and the backend's own words (a
                // missing i2c group, say) are what tells the user why.
                err(format!("cosmic-dimmer: {error}"));
            }
        }
    }

    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use dimmer_core::profile::Profile;
    use dimmer_core::sun::Phase;
    use dimmer_core::{EdidId, Floors, LevelKind, OutputId, mock::MockBackend, store::LoadIssue};

    use super::*;

    struct TempHome(PathBuf);

    impl TempHome {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("cosmic-dimmer-reset-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }

        fn store(&self) -> MonitorStore {
            MonitorStore::new(
                cosmic_config::Config::with_custom_path(APP_ID, CONFIG_VERSION, self.0.clone())
                    .unwrap(),
            )
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn saved_levels_go_to_full_keeping_lock_and_floors() {
        let home = TempHome::new("saved");
        let store = home.store();
        // Made-up identities, never a real device's serial.
        let first = EdidId::new("TST", 1, 1).unwrap();
        let second = EdidId::new("TST", 1, 2).unwrap();
        let floors = Floors::clamped(15, 25, 12);
        let mut profile = Profile::default();
        profile.learn(Phase::SUNSET, 100, 20);
        store
            .save(
                &first,
                &MonitorLevels::clamped(40, 30, true, floors),
                &profile,
            )
            .unwrap();
        store
            .save(
                &second,
                &MonitorLevels::clamped(100, 20, false, Floors::default()),
                &Profile::default(),
            )
            .unwrap();
        // Not a monitor entry: only auto dimming is turned off in it.
        let config =
            cosmic_config::Config::with_custom_path(APP_ID, CONFIG_VERSION, home.0.clone())
                .unwrap();
        let settings = crate::settings::Settings {
            auto_dimming: true,
            key_step: 30,
            ..crate::settings::Settings::default()
        };
        settings.save(&config).unwrap();

        assert_eq!(reset_saved(&home.0), Ok(2));
        let loaded = home.store().load(&first);
        assert_eq!(loaded.issues, Vec::<LoadIssue>::new());
        assert_eq!(
            loaded.levels,
            MonitorLevels::clamped(100, 100, true, floors)
        );
        assert_eq!(loaded.profile, profile, "the profile is kept");
        assert_eq!(home.store().load(&second).levels, MonitorLevels::default());
        let after = crate::settings::Settings::load(&config);
        assert!(!after.auto_dimming, "auto dimming is off after a reset");
        assert_eq!(after.key_step, 30, "other settings are kept");
    }

    #[test]
    fn terminal_text_drops_isolation_marks() {
        assert_eq!(
            plain(&fl!("reset-saved", count = 2)),
            "Saved levels set to 100% for 2 displays."
        );
        assert_eq!(
            plain(&fl!("reset-saved", count = 1)),
            "Saved levels set to 100% for 1 display."
        );
    }

    #[test]
    fn nothing_saved_is_not_an_error() {
        let home = TempHome::new("empty");
        assert_eq!(reset_saved(&home.0), Ok(0));
    }

    #[tokio::test]
    async fn hardware_reset_writes_full_to_every_output() {
        let backend = MockBackend::new(["a", "b"], LevelKind::Hardware);
        for name in ["a", "b"] {
            backend
                .set_level(&OutputId(name.into()), Level::new(20).unwrap())
                .await
                .unwrap();
        }
        assert_eq!(
            reset_hardware(&backend, Duration::from_secs(1)).await,
            Ok((2, 0))
        );
        for name in ["a", "b"] {
            assert_eq!(backend.level(&OutputId(name.into())), Some(Level::FULL));
        }
    }

    #[tokio::test]
    async fn missing_hardware_is_reported_not_faked() {
        // The real DDC backend, pointed at no sysfs and no /dev: a unit test
        // must never scan or write the host's monitors.
        let nowhere = std::path::PathBuf::from("/nonexistent-cosmic-dimmer-test");
        let ddc = dimmer_ddc::DdcBackend::new(dimmer_ddc::DdcConfig {
            sysfs_root: nowhere.clone(),
            dev_root: nowhere,
            ..dimmer_ddc::DdcConfig::default()
        })
        .unwrap();
        assert!(matches!(
            reset_hardware(&backend::AnyBackend::Ddc(ddc), Duration::from_secs(1)).await,
            Err(BackendError::Unavailable(_))
        ));
    }
}
