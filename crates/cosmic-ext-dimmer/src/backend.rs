//! The seam between the applet and its backends.
//!
//! `dimmer_core::Backend` returns `impl Future`, so it cannot be a trait
//! object; [`AnyBackend`] is the one place that names every concrete backend.
//! [`Backends::system`] builds the real pair, the overlay backend and the DDC
//! backend; a backend that needs construction arguments is built there.
//!
//! The applet pairs a display's two outputs by [`dimmer_core::OutputId`], so
//! both backends must name a display the same way: the overlay backend uses
//! [`dimmer_core::EdidId::entry_name`] (`connector:<name>` without an EDID),
//! and the DDC backend has to as well.

use std::sync::Arc;

use dimmer_core::{
    Backend, BackendError, Capabilities, Level, LevelKind, Output, OutputId, mock::MockBackend,
};
use dimmer_ddc::DdcBackend;
use dimmer_overlay::OverlayBackend;

/// Set to `1` to run the applet on in-memory mock backends (development only):
/// two displays, one with hardware control and one without.
pub const MOCK_ENV: &str = "COSMIC_EXT_DIMMER_MOCK";

/// Mock display IDs (and names). Not EDID identities, so nothing is saved.
pub const MOCK_WITH_HARDWARE: &str = "MOCK-1";
pub const MOCK_WITHOUT_HARDWARE: &str = "MOCK-2";

/// Every backend the applet can drive.
pub enum AnyBackend {
    Mock(MockBackend),
    Ddc(DdcBackend),
    Overlay(OverlayBackend),
}

impl Backend for AnyBackend {
    async fn outputs(&self) -> Result<Vec<Output>, BackendError> {
        match self {
            Self::Mock(backend) => backend.outputs().await,
            Self::Ddc(backend) => backend.outputs().await,
            Self::Overlay(backend) => backend.outputs().await,
        }
    }

    async fn capabilities(&self, output: &OutputId) -> Result<Capabilities, BackendError> {
        match self {
            Self::Mock(backend) => backend.capabilities(output).await,
            Self::Ddc(backend) => backend.capabilities(output).await,
            Self::Overlay(backend) => backend.capabilities(output).await,
        }
    }

    async fn set_level(&self, output: &OutputId, level: Level) -> Result<(), BackendError> {
        match self {
            Self::Mock(backend) => backend.set_level(output, level).await,
            Self::Ddc(backend) => backend.set_level(output, level).await,
            Self::Overlay(backend) => backend.set_level(output, level).await,
        }
    }
}

/// The real hardware backend alone (DDC/CI), for `--reset`.
#[must_use]
pub fn hardware() -> AnyBackend {
    AnyBackend::Ddc(DdcBackend::default())
}

/// The hardware and the software (alpha) backend, shared with write tasks.
#[derive(Clone)]
pub struct Backends {
    pub hardware: Arc<AnyBackend>,
    pub alpha: Arc<AnyBackend>,
}

impl Backends {
    /// The real backends: DDC/CI for hardware, layer-shell overlays for alpha.
    /// Starts the overlay thread, which connects to the compositor on its own.
    #[must_use]
    pub fn system() -> Self {
        Self {
            hardware: Arc::new(hardware()),
            alpha: Arc::new(AnyBackend::Overlay(OverlayBackend::spawn())),
        }
    }

    /// In-memory backends: `MOCK-1` answers on both, `MOCK-2` has no hardware
    /// control, as a display that does not answer DDC/CI.
    #[must_use]
    pub fn mock() -> Self {
        Self {
            hardware: Arc::new(AnyBackend::Mock(MockBackend::new(
                [MOCK_WITH_HARDWARE],
                LevelKind::Hardware,
            ))),
            alpha: Arc::new(AnyBackend::Mock(MockBackend::new(
                [MOCK_WITH_HARDWARE, MOCK_WITHOUT_HARDWARE],
                LevelKind::Software,
            ))),
        }
    }

    /// Mock backends when [`MOCK_ENV`] is `1`, otherwise the real ones.
    #[must_use]
    pub fn from_env() -> Self {
        if std::env::var(MOCK_ENV).is_ok_and(|value| value == "1") {
            tracing::warn!("{MOCK_ENV}=1: using mock backends, no display is changed");
            Self::mock()
        } else {
            Self::system()
        }
    }
}
