//! Which Wayland server the overlay talks to, and how long it waits to retry.
//!
//! An applet does **not** get the real compositor through the usual
//! variables. cosmic-panel starts each applet with `WAYLAND_SOCKET` set to a
//! socket of the panel's own embedded server (the applet's popups and button
//! live there), and, for desktop entries with `X-HostWaylandDisplay=true`, a
//! file descriptor in `X_PRIVILEGED_WAYLAND_SOCKET` that libcosmic's
//! applet-token handler takes for itself, plus the host's `WAYLAND_DISPLAY`.
//! A layer surface made on the panel's server would never reach cosmic-comp,
//! and the privileged descriptor can carry only one connection. So the
//! overlay opens its own connection to the socket `WAYLAND_DISPLAY` names,
//! the way a terminal client would, and never reads `WAYLAND_SOCKET`.
//! (`Connection::connect_to_env` prefers `WAYLAND_SOCKET`; it is not used.)
//! That client has no security context, which cosmic-comp's filter for
//! wlr-layer-shell admits.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    time::Duration,
};

use dimmer_core::BackendError;

/// First wait before reconnecting after a failure.
pub const BACKOFF_START: Duration = Duration::from_secs(1);
/// Longest wait between reconnection attempts.
pub const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// The compositor socket from `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR`: an
/// absolute `WAYLAND_DISPLAY` is used as is, a name is looked up in the
/// runtime directory, as libwayland does.
///
/// # Errors
/// `Unavailable` when either is missing, empty or unusable.
pub fn host_socket_path(
    wayland_display: Option<&OsStr>,
    runtime_dir: Option<&OsStr>,
) -> Result<PathBuf, BackendError> {
    let display = wayland_display
        .filter(|display| !display.is_empty())
        .ok_or_else(|| {
            BackendError::Unavailable(
                "WAYLAND_DISPLAY is not set (an applet needs X-HostWaylandDisplay=true)".into(),
            )
        })?;
    let display = Path::new(display);
    if display.is_absolute() {
        return Ok(display.to_path_buf());
    }
    let runtime = runtime_dir
        .map(Path::new)
        .filter(|dir| dir.is_absolute())
        .ok_or_else(|| BackendError::Unavailable("XDG_RUNTIME_DIR is not set".into()))?;
    Ok(runtime.join(display))
}

/// The wait after this one: doubled, at most [`BACKOFF_MAX`].
#[must_use]
pub fn next_backoff(current: Duration) -> Duration {
    current.saturating_mul(2).clamp(BACKOFF_START, BACKOFF_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_from_name_or_absolute_path() {
        assert_eq!(
            host_socket_path(
                Some(OsStr::new("wayland-1")),
                Some(OsStr::new("/run/user/1000"))
            ),
            Ok(PathBuf::from("/run/user/1000/wayland-1"))
        );
        assert_eq!(
            host_socket_path(Some(OsStr::new("/tmp/w")), None),
            Ok(PathBuf::from("/tmp/w"))
        );
    }

    #[test]
    fn missing_or_relative_environment_is_unavailable() {
        for (display, runtime) in [
            (None, Some("/run/user/1000")),
            (Some(""), Some("/run/user/1000")),
            (Some("wayland-1"), None),
            (Some("wayland-1"), Some("relative/dir")),
        ] {
            assert!(matches!(
                host_socket_path(display.map(OsStr::new), runtime.map(OsStr::new)),
                Err(BackendError::Unavailable(_))
            ));
        }
    }

    #[test]
    fn backoff_doubles_up_to_the_cap() {
        assert_eq!(next_backoff(BACKOFF_START), Duration::from_secs(2));
        assert_eq!(next_backoff(Duration::from_secs(20)), BACKOFF_MAX);
        assert_eq!(next_backoff(BACKOFF_MAX), BACKOFF_MAX);
        assert_eq!(next_backoff(Duration::ZERO), BACKOFF_START);
    }
}
