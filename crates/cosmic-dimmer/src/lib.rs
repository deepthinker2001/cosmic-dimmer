//! Standalone COSMIC panel applet: per-display hardware and software dimming.
//!
//! The popup lists each display by model name with a hardware slider (or why
//! there is none) and a software (alpha overlay) slider, a lock that moves both
//! together and a reset to 100 %. Levels follow dimmer-core's
//! [`dimmer_core::MonitorLevels`] policy, so no slider goes below its floor or
//! takes the combined level below the effective floor (R11, R12).
//!
//! Nothing here waits for a display: every backend call runs as a task on the
//! executor, bounded by the configured timeout, and its result comes back as a
//! message (R10). See `monitor.rs` for the model and `backend.rs` for the seam
//! where the real DDC and overlay backends plug in.

mod app;
mod auto;
mod backend;
mod keys;
mod localize;
mod location;
mod monitor;
mod reset;
mod settings;
mod shortcuts;
mod view;

use cosmic::{applet, iced};

/// Reverse-DNS application ID. It names the desktop entry in `data/`, the
/// applet's config namespace and its D-Bus name, so changing it later moves
/// users' settings and breaks their brightness-key shortcuts.
pub const APP_ID: &str = "io.github.deepthinker2001.CosmicDimmer";

/// Panel icon: the applet's own symbolic icon, installed into the hicolor
/// theme from `data/icons`.
const ICON: &str = "io.github.deepthinker2001.CosmicDimmer-symbolic";

/// Run the applet under cosmic-panel.
///
/// # Errors
/// Returns libcosmic's error if the Wayland connection or panel handshake fails.
pub fn run() -> iced::Result {
    localize::localize();
    applet::run::<app::Dimmer>(())
}

/// The command-line help text.
#[must_use]
pub fn usage() -> String {
    localize::localize();
    localize::plain(&fl!("usage"))
}

/// `cosmic-dimmer --reset`: every display back to 100 % from a
/// terminal, for a screen too dark to use the applet. See `reset.rs`.
#[must_use]
pub fn reset_from_terminal() -> std::process::ExitCode {
    localize::localize();
    reset::run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_id_matches_desktop_entry_file() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/data/");
        let entry = std::fs::read_to_string(format!("{path}{APP_ID}.desktop"))
            .expect("desktop entry named after APP_ID");
        assert!(entry.contains("X-CosmicApplet=true"));
        assert!(entry.contains("NoDisplay=true"));
        assert!(entry.contains("Exec=cosmic-dimmer\n"));
        assert!(entry.contains(&format!("Icon={ICON}\n")));
    }

    #[test]
    fn fallback_strings_resolve() {
        // fl! panics at compile time on unknown IDs; this checks they load
        // and that arguments are substituted.
        assert_eq!(fl!("applet-name"), "Dimmer");
        // Fluent isolates arguments with U+2068/U+2069 for right-to-left
        // languages, as in the stock applets; compare without them.
        let status = fl!("software-unavailable", reason = fl!("reason-unsupported"));
        assert!(localize::plain(&status).ends_with(&fl!("reason-unsupported")));
        assert!(fl!("level", value = 40).contains("40"));
    }

    #[test]
    fn bus_name_is_the_app_id() {
        assert_eq!(keys::BUS_NAME, APP_ID);
        assert_eq!(
            keys::OBJECT_PATH,
            format!("/{}", APP_ID.replace('.', "/")),
            "object path follows the bus name"
        );
    }
}
