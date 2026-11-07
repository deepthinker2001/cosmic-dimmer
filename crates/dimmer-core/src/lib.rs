//! Types, policy and asynchronous backend contract for independent outputs.
//!
//! Levels denote retained brightness, not overlay opacity. [`MonitorLevels`]
//! is the per-monitor policy (R11, R12): hardware and alpha levels, a lock,
//! and floors enforced here, including a lower bound on the combined effective
//! brightness. Its floor defaults are **provisional**: no safe hardware floor
//! has been measured yet (see [`Floors`]).
//!
//! No hardware or compositor I/O happens in this crate. The only I/O is the
//! [`store`] module reading and writing the applet's own cosmic-config files.

use std::{future::Future, time::Duration};

pub mod edid;
pub mod levels;
pub mod mock;
pub mod profile;
pub mod store;
pub mod sun;

pub use edid::{EdidError, EdidId};
pub use levels::{Change, Floors, MonitorLevels};

/// Backend-scoped output identity, not a globally unique display identifier.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct OutputId(pub String);

/// An output discovered by one backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Output {
    pub id: OutputId,
    pub name: String,
}

/// Retained brightness in whole percent, from 1 to 100 inclusive.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Level(u8);

impl Level {
    pub const FULL: Self = Self(100);

    /// Validate a nonzero percentage. Hardware-specific floors belong in policy.
    ///
    /// # Errors
    /// Returns `InvalidLevel` outside 1..=100.
    pub const fn new(percent: u8) -> Result<Self, BackendError> {
        if matches!(percent, 1..=100) {
            Ok(Self(percent))
        } else {
            Err(BackendError::InvalidLevel)
        }
    }

    #[must_use]
    pub const fn percent(self) -> u8 {
        self.0
    }
}

/// Which kind of brightness this backend controls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LevelKind {
    Hardware,
    /// Retained brightness: the eventual overlay's opacity is 1 - level / 100.
    Software,
}

/// Per-output support; unknown hardware must not advertise a measured floor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Capabilities {
    pub kind: LevelKind,
    pub writable: bool,
}

/// Explicit failures; a successful command must not be inferred from UI intent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendError {
    InvalidLevel,
    UnknownOutput,
    Unsupported,
    TimedOut,
    Unavailable(String),
    /// A newer level for this output replaced this one before it was written.
    /// Not a failure of the display: the newer level is (or will be) applied.
    Superseded,
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLevel => f.write_str("level must be between 1 and 100 percent"),
            Self::UnknownOutput => f.write_str("output is not known to this backend"),
            Self::Unsupported => f.write_str("operation is not supported by this backend"),
            Self::TimedOut => f.write_str("backend operation timed out"),
            Self::Unavailable(reason) => write!(f, "backend unavailable: {reason}"),
            Self::Superseded => {
                f.write_str("a newer level for this output replaced this one before it was written")
            }
        }
    }
}

impl std::error::Error for BackendError {}

/// Nonblocking backend operations. Real implementations must move blocking I/O
/// to bounded workers and enforce transport deadlines there, too. Dropping a
/// future does not cancel an already-running kernel ioctl or worker operation.
/// Use [`with_timeout`] at the caller boundary for every operation.
///
/// Enumeration and capability discovery are async because they may need I/O.
/// Output IDs are scoped to this backend; they may go stale on hot unplug.
pub trait Backend: Send + Sync {
    fn outputs(&self) -> impl Future<Output = Result<Vec<Output>, BackendError>> + Send;
    fn capabilities(
        &self,
        output: &OutputId,
    ) -> impl Future<Output = Result<Capabilities, BackendError>> + Send;
    fn set_level(
        &self,
        output: &OutputId,
        level: Level,
    ) -> impl Future<Output = Result<(), BackendError>> + Send;
}

/// Bound caller waiting time on a Tokio runtime with time enabled.
///
/// This cancels the future on expiry, not arbitrary underlying blocking I/O.
/// The future must yield; a blocking poll cannot be interrupted by this helper.
///
/// # Errors
/// Returns `TimedOut` on expiry, otherwise propagates the backend result.
pub async fn with_timeout<T>(
    timeout: Duration,
    operation: impl Future<Output = Result<T, BackendError>>,
) -> Result<T, BackendError> {
    tokio::time::timeout(timeout, operation)
        .await
        .map_err(|_| BackendError::TimedOut)?
}

#[cfg(test)]
mod tests {
    use super::{mock::MockBackend, *};

    #[test]
    fn level_rejects_zero_and_out_of_range() {
        for percent in 0..=u8::MAX {
            assert_eq!(Level::new(percent).is_ok(), (1..=100).contains(&percent));
        }
    }

    fn backend() -> MockBackend {
        MockBackend::new(["left", "right"], LevelKind::Hardware)
    }

    #[tokio::test]
    async fn independent_outputs_and_capabilities() {
        let backend = backend();
        let outputs = backend.outputs().await.unwrap();
        assert_eq!(outputs.len(), 2);
        backend
            .set_level(&outputs[0].id, Level::new(40).unwrap())
            .await
            .unwrap();
        assert_eq!(backend.level(&outputs[0].id), Some(Level::new(40).unwrap()));
        assert_eq!(backend.level(&outputs[1].id), Some(Level::FULL));
        assert_eq!(
            backend.capabilities(&outputs[0].id).await.unwrap(),
            Capabilities {
                kind: LevelKind::Hardware,
                writable: true,
            }
        );
    }

    #[tokio::test]
    async fn missing_output_is_not_success() {
        let backend = backend();
        let missing = OutputId("unplugged".into());
        assert_eq!(
            backend.set_level(&missing, Level::FULL).await,
            Err(BackendError::UnknownOutput)
        );
        assert_eq!(
            backend.capabilities(&missing).await,
            Err(BackendError::UnknownOutput)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_drops_pending_write_without_mutation() {
        let backend = backend().with_delay(Duration::from_secs(10));
        let output = OutputId("left".into());
        let result = with_timeout(
            Duration::from_secs(1),
            backend.set_level(&output, Level::new(10).unwrap()),
        )
        .await;
        assert_eq!(result, Err(BackendError::TimedOut));
        tokio::time::advance(Duration::from_secs(20)).await;
        assert_eq!(backend.level(&output), Some(Level::FULL));
    }

    #[tokio::test(start_paused = true)]
    async fn discovery_is_also_bounded() {
        let backend = backend().with_delay(Duration::from_secs(10));
        assert_eq!(
            with_timeout(Duration::from_secs(1), backend.outputs()).await,
            Err(BackendError::TimedOut)
        );
        assert_eq!(
            with_timeout(
                Duration::from_secs(1),
                backend.capabilities(&OutputId("left".into()))
            )
            .await,
            Err(BackendError::TimedOut)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn completion_before_deadline_and_errors_are_preserved() {
        let backend = backend().with_delay(Duration::from_millis(10));
        assert_eq!(
            with_timeout(
                Duration::from_secs(1),
                backend.set_level(&OutputId("left".into()), Level::FULL)
            )
            .await,
            Ok(())
        );
        assert_eq!(
            with_timeout(
                Duration::from_secs(1),
                backend.set_level(&OutputId("missing".into()), Level::FULL)
            )
            .await,
            Err(BackendError::UnknownOutput)
        );
    }
}
