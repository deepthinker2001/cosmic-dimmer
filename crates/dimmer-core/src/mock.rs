//! In-memory backend for tests and future UI development; no hardware I/O.
use std::{collections::BTreeMap, sync::Mutex, time::Duration};

use crate::{Backend, BackendError, Capabilities, Level, LevelKind, Output, OutputId};

pub struct MockBackend {
    levels: Mutex<BTreeMap<String, Level>>,
    kind: LevelKind,
    delay: Duration,
}

impl MockBackend {
    pub fn new(names: impl IntoIterator<Item = impl Into<String>>, kind: LevelKind) -> Self {
        Self {
            levels: Mutex::new(
                names
                    .into_iter()
                    .map(|name| (name.into(), Level::FULL))
                    .collect(),
            ),
            kind,
            delay: Duration::ZERO,
        }
    }

    #[must_use]
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    /// Inspect mock state without invoking the simulated transport delay.
    #[must_use]
    pub fn level(&self, output: &OutputId) -> Option<Level> {
        self.levels.lock().ok()?.get(&output.0).copied()
    }

    async fn wait(&self) {
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
    }
}

impl Backend for MockBackend {
    async fn outputs(&self) -> Result<Vec<Output>, BackendError> {
        self.wait().await;
        let levels = self
            .levels
            .lock()
            .map_err(|_| BackendError::Unavailable("mock state poisoned".into()))?;
        Ok(levels
            .keys()
            .map(|name| Output {
                id: OutputId(name.clone()),
                name: name.clone(),
            })
            .collect())
    }

    async fn capabilities(&self, output: &OutputId) -> Result<Capabilities, BackendError> {
        self.wait().await;
        if self.level(output).is_none() {
            return Err(BackendError::UnknownOutput);
        }
        Ok(Capabilities {
            kind: self.kind,
            writable: true,
        })
    }

    async fn set_level(&self, output: &OutputId, level: Level) -> Result<(), BackendError> {
        self.wait().await;
        let mut levels = self
            .levels
            .lock()
            .map_err(|_| BackendError::Unavailable("mock state poisoned".into()))?;
        let current = levels
            .get_mut(&output.0)
            .ok_or(BackendError::UnknownOutput)?;
        *current = level;
        Ok(())
    }
}
