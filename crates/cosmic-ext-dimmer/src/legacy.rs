//! The applet's earlier name, and the one-time move of what it saved.
//!
//! Before 0.2.0 the applet was `cosmic-dimmer`, with the App ID
//! `io.github.deepthinker2001.CosmicDimmer`. That ID named its settings
//! folder and its D-Bus name, which the brightness shortcuts it wrote call.
//! At start, [`carry_over`] copies the old settings folder to the new App
//! ID's and points those shortcuts at the new name. Both happen once: the
//! folder is copied only while the new one does not exist, so nothing saved
//! since is ever overwritten, and the old folder is left as it was.

use std::fs;
use std::io;
use std::path::Path;

use crate::keys::{self, Key};

/// The App ID before 0.2.0, also its D-Bus name and interface.
pub const APP_ID: &str = "io.github.deepthinker2001.CosmicDimmer";
const OBJECT_PATH: &str = "/io/github/deepthinker2001/CosmicDimmer";

/// The command a shortcut written before 0.2.0 spawns for `key`.
#[must_use]
pub fn command(key: Key) -> String {
    format!(
        "busctl --user call {APP_ID} {OBJECT_PATH} {APP_ID} {}",
        key.method()
    )
}

/// Move what the earlier version saved: settings and levels, then its
/// shortcuts. Logs what it did; a failure only means starting fresh.
/// Blocking file I/O: call it before the applet starts.
pub fn carry_over() {
    copy_settings_logged();
    match keys::with_shortcuts(keys::rename_legacy) {
        Ok(0) => {}
        Ok(renamed) => tracing::info!(renamed, "brightness shortcuts moved to the new name"),
        Err(error) => tracing::warn!(?error, "brightness shortcuts not moved to the new name"),
    }
}

/// Copy the earlier version's settings and levels, if there are any to copy.
pub fn copy_settings_logged() {
    let Some(config_home) = crate::reset::config_home() else {
        return;
    };
    match copy_settings(&config_home) {
        Ok(true) => tracing::info!("settings copied from {APP_ID}"),
        Ok(false) => {}
        Err(error) => tracing::warn!(%error, "settings not copied from {APP_ID}"),
    }
}

/// Copy `<config_home>/cosmic/<old App ID>` to the new App ID's folder, if
/// the old one exists and the new one does not. Returns whether it copied.
///
/// # Errors
/// The first I/O error; the new folder is then not created.
pub fn copy_settings(config_home: &Path) -> io::Result<bool> {
    let cosmic = config_home.join("cosmic");
    let old = cosmic.join(APP_ID);
    let new = cosmic.join(crate::APP_ID);
    if new.exists() || !old.is_dir() {
        return Ok(false);
    }
    // Copied beside the new folder and renamed into place, so an interrupted
    // copy is never taken for a finished one.
    let partial = cosmic.join(format!("{}.partial", crate::APP_ID));
    if partial.exists() {
        fs::remove_dir_all(&partial)?;
    }
    copy_tree(&old, &partial)?;
    fs::rename(&partial, &new)?;
    Ok(true)
}

/// Copy the folders and files under `from` to `to`, which must not exist.
/// Anything else (cosmic-config writes no links) is skipped.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    struct TempHome(PathBuf);

    impl TempHome {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "cosmic-ext-dimmer-legacy-{tag}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn folder(&self, app_id: &str) -> PathBuf {
            self.0.join("cosmic").join(app_id).join("v1")
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn copies_once_and_keeps_the_old_folder() {
        let home = TempHome::new("copy");
        let old = home.folder(APP_ID);
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("key_step"), "20").unwrap();

        assert!(copy_settings(&home.0).unwrap());
        let new = home.folder(crate::APP_ID);
        assert_eq!(fs::read_to_string(new.join("key_step")).unwrap(), "20");
        assert!(old.join("key_step").exists(), "the old folder stays");

        // A change made since is never overwritten by a second start.
        fs::write(new.join("key_step"), "30").unwrap();
        assert!(!copy_settings(&home.0).unwrap());
        assert_eq!(fs::read_to_string(new.join("key_step")).unwrap(), "30");
    }

    #[test]
    fn nothing_to_copy_is_not_an_error() {
        let home = TempHome::new("none");
        assert!(!copy_settings(&home.0).unwrap());
        assert!(!home.0.join("cosmic").join(crate::APP_ID).exists());
    }

    #[test]
    fn the_old_command_names_the_old_interface() {
        assert_eq!(
            command(Key::Raise),
            "busctl --user call io.github.deepthinker2001.CosmicDimmer \
             /io/github/deepthinker2001/CosmicDimmer \
             io.github.deepthinker2001.CosmicDimmer Raise"
        );
        assert_ne!(command(Key::Lower), Key::Lower.command());
    }
}
