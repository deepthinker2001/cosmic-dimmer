//! Software alpha dimming through applet-owned wlr-layer-shell overlay
//! surfaces on stock cosmic-comp (R2, F2).
//!
//! One overlay-layer surface per output, black at opacity
//! `1 - retained level`, with an empty input region so every click and key
//! goes through. The design, the decisions and the live checks are in
//! docs/overlay-backend.md.
//!
//! - **Identity:** outputs are named by the EDID identity dimmer-core keys
//!   settings by ([`identity`]): [`OutputId`] is [`dimmer_core::EdidId::entry_name`],
//!   so the applet joins a monitor's saved levels to its overlay without a table.
//! - **No cost by default:** at full brightness there is no surface at all.
//! - **Floor:** nothing below [`state::ALPHA_FLOOR`] is ever drawn, whatever
//!   the caller asks (R12's last line of defence).
//! - **Lifecycle:** surfaces follow hotplug, configure (resolution, scale,
//!   rotation), compositor `closed` and reconnects; levels are kept by
//!   identity, so a monitor that returns (replug, wake) is dimmed again.
//! - **Isolation:** its own thread and its own Wayland connection inside the
//!   applet process. Any error at that boundary drops the connection (the
//!   compositor then removes every surface: undimmed, never stuck dark),
//!   reports `Unavailable`, and reconnects with backoff.
//!
//! Every call is non-blocking for the caller; bound it with
//! [`dimmer_core::with_timeout`] as for any backend.

use std::future::Future;

use dimmer_core::{Backend, BackendError, Capabilities, Level, LevelKind, Output, OutputId};
use smithay_client_toolkit::reexports::calloop::channel;
use tokio::sync::{oneshot, watch};

pub mod connect;
pub mod identity;
pub mod state;
mod worker;

pub use state::{ALPHA_FLOOR, OutputView};
pub use worker::{Config, NAMESPACE, Snapshot, Status};

use worker::Command;

/// The alpha backend. Creating it starts the overlay thread, which connects
/// to the compositor on its own; dropping it ends the thread and removes
/// every surface.
pub struct OverlayBackend {
    commands: Option<channel::Sender<Command>>,
    snapshot: watch::Receiver<Snapshot>,
}

impl std::fmt::Debug for OverlayBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OverlayBackend")
            .field("snapshot", &*self.snapshot.borrow())
            .finish_non_exhaustive()
    }
}

impl OverlayBackend {
    /// Start with the default configuration: the compositor named by
    /// `WAYLAND_DISPLAY` (never `WAYLAND_SOCKET`; see [`connect`]) and EDIDs
    /// from `/sys/class/drm`.
    #[must_use]
    pub fn spawn() -> Self {
        Self::spawn_with(Config::default())
    }

    /// Start with an explicit configuration (tests, live checks).
    #[must_use]
    pub fn spawn_with(config: Config) -> Self {
        let (commands, snapshot) = worker::spawn(config);
        Self { commands, snapshot }
    }

    /// The current state without waiting: connection status and outputs.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.borrow().clone()
    }

    /// A receiver that sees every new snapshot (outputs appearing, levels
    /// applied, the connection lost), for a UI subscription.
    #[must_use]
    pub fn watch(&self) -> watch::Receiver<Snapshot> {
        self.snapshot.clone()
    }

    /// Wait until the first connection attempt has finished, then return the
    /// snapshot, or why the overlay is unavailable.
    async fn settled(mut snapshot: watch::Receiver<Snapshot>) -> Result<Snapshot, BackendError> {
        let settled = snapshot
            .wait_for(|snapshot| snapshot.status != Status::Connecting)
            .await
            .map(|snapshot| snapshot.clone())
            .map_err(|_| BackendError::Unavailable("the overlay thread stopped".into()))?;
        match &settled.status {
            Status::Unavailable(reason) => Err(BackendError::Unavailable(reason.clone())),
            _ => Ok(settled),
        }
    }
}

impl Backend for OverlayBackend {
    fn outputs(&self) -> impl Future<Output = Result<Vec<Output>, BackendError>> + Send {
        let snapshot = self.snapshot.clone();
        async move {
            Ok(Self::settled(snapshot)
                .await?
                .outputs
                .into_iter()
                .map(|view| Output {
                    id: view.id,
                    name: view.name,
                })
                .collect())
        }
    }

    fn capabilities(
        &self,
        output: &OutputId,
    ) -> impl Future<Output = Result<Capabilities, BackendError>> + Send {
        let snapshot = self.snapshot.clone();
        let output = output.clone();
        async move {
            let view = Self::settled(snapshot)
                .await?
                .outputs
                .into_iter()
                .find(|view| view.id == output)
                .ok_or(BackendError::UnknownOutput)?;
            Ok(Capabilities {
                kind: LevelKind::Software,
                writable: view.writable,
            })
        }
    }

    /// Answered once the new level is committed to the compositor (or the
    /// surface destroyed at full brightness). Levels below
    /// [`ALPHA_FLOOR`] are applied at the floor; [`Self::snapshot`] shows the
    /// level actually applied.
    fn set_level(
        &self,
        output: &OutputId,
        level: Level,
    ) -> impl Future<Output = Result<(), BackendError>> + Send {
        let (reply, answer) = oneshot::channel();
        let sent = match &self.commands {
            Some(commands) => commands
                .send(Command::SetLevel {
                    id: output.clone(),
                    level,
                    reply,
                })
                .map_err(|_| BackendError::Unavailable("the overlay thread stopped".into())),
            None => Err(BackendError::Unavailable(
                "the overlay thread could not start".into(),
            )),
        };
        async move {
            sent?;
            answer
                .await
                .map_err(|_| BackendError::Unavailable("the overlay thread stopped".into()))?
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Write, os::unix::net::UnixListener, path::PathBuf, time::Duration};

    use dimmer_core::with_timeout;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dimmer-overlay-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn config(socket: PathBuf, sysfs: PathBuf) -> Config {
        Config {
            socket: Some(socket),
            sysfs_root: sysfs,
        }
    }

    async fn assert_unavailable(backend: &OverlayBackend) {
        let limit = Duration::from_secs(10);
        assert!(matches!(
            with_timeout(limit, backend.outputs()).await,
            Err(BackendError::Unavailable(_))
        ));
        assert!(matches!(
            with_timeout(
                limit,
                backend.capabilities(&OutputId("SAM-00000001-00000000".into()))
            )
            .await,
            Err(BackendError::Unavailable(_))
        ));
        assert!(matches!(
            with_timeout(
                limit,
                backend.set_level(&OutputId("SAM-00000001-00000000".into()), Level::FULL)
            )
            .await,
            Err(BackendError::Unavailable(_))
        ));
        assert!(matches!(backend.snapshot().status, Status::Unavailable(_)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_compositor_is_unavailable_not_a_panic() {
        let dir = scratch("none");
        let backend = OverlayBackend::spawn_with(config(dir.join("absent"), dir.clone()));
        assert_unavailable(&backend).await;
        drop(backend);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_server_that_hangs_up_is_unavailable() {
        let dir = scratch("hangup");
        let socket = dir.join("wayland-test");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            // Accept a few attempts (the worker retries) and hang up at once.
            for stream in listener.incoming().take(2) {
                drop(stream);
            }
        });
        let backend = OverlayBackend::spawn_with(config(socket, dir.clone()));
        assert_unavailable(&backend).await;
        drop(backend);
        drop(server);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_server_that_sends_garbage_is_unavailable() {
        let dir = scratch("garbage");
        let socket = dir.join("wayland-test");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            if let Some(Ok(mut stream)) = listener.incoming().next() {
                // A message header for an object that does not exist.
                let _ = stream.write_all(&[0xFF; 64]);
                std::thread::sleep(Duration::from_millis(200));
            }
        });
        let backend = OverlayBackend::spawn_with(config(socket, dir.clone()));
        assert_unavailable(&backend).await;
        drop(backend);
        let _ = server.join();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn backend_is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<OverlayBackend>();
    }
}
