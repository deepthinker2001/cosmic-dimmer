//! DDC/CI hardware brightness backend over the kernel's i2c-dev interface
//! (R1, R8, R9, R10). Design, defaults and coexistence with the stock
//! cosmic-settings-daemon: see this crate's README.md.
//!
//! - Discovery reads sysfs, then each display controller's I2C buses' EDID,
//!   and assigns a bus to a DRM connector only when the EDIDs are identical
//!   (`matching`), so a write can never reach another monitor.
//! - Every matched display has its own worker thread (`worker`): its I/O is
//!   serialized, others are unaffected, and rapid level changes coalesce.
//! - Every call waits at most [`DdcConfig::call_timeout`]; the worker also
//!   stops retrying at that deadline.
//! - A display that fails is reported unavailable from cached state, with
//!   no further bus traffic, until hotplug or a rescan after
//!   [`DdcConfig::reprobe_backoff`].
//!
//! No root and no external program: the caller needs read-write access to
//! `/dev/i2c-*` through a udev rule and group (R4).

use std::{
    collections::BTreeMap,
    fmt,
    future::Future,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    thread,
    time::{Duration, Instant},
};

use dimmer_core::{
    Backend, BackendError, Capabilities, EdidId, Level, LevelKind, Output, OutputId,
};
use tokio::sync::oneshot;

pub mod edid;
mod exchange;
pub mod matching;
pub mod protocol;
pub mod sysfs;
pub mod transport;
mod worker;

pub use edid::Edid;
pub use exchange::Timing;
pub use matching::MatchProblem;
pub use protocol::VcpValue;
pub use worker::Health;

use crate::{
    matching::ProbedBus,
    sysfs::{Adapter, Connector, Sysfs},
    transport::{BusOpener, I2cDevOpener},
    worker::{Target, Worker, WorkerConfig},
};

/// What went wrong, in more detail than [`BackendError`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DdcError {
    /// A system call failed (`EIO`/`ENXIO`/`EREMOTEIO`: nothing answered).
    Io {
        op: &'static str,
        errno: Option<i32>,
        detail: String,
    },
    /// Replies came back, but none was a valid one.
    BadReply(protocol::ReplyError),
    /// The bus returned bytes that are not an EDID.
    BadEdid(String),
    /// The display says it does not support the VCP code.
    UnsupportedCode(u8),
    /// The display reports a maximum of zero for the code.
    BadMaximum,
    /// Another process holds the bus's advisory lock.
    BusBusy,
    /// The connector now holds another EDID than the one the bus was
    /// matched on; nothing is sent until a rescan matches it again.
    DisplayChanged,
    /// The connector has no bus this backend may use.
    NotMatched(MatchProblem),
    /// A newer level for this output replaced this one before it was written.
    Superseded,
    /// The deadline passed.
    TimedOut,
    /// The output was removed or the backend dropped.
    WorkerStopped,
}

impl fmt::Display for DdcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const EACCES: i32 = 13;
        const EIO: i32 = 5;
        match self {
            Self::Io {
                op,
                errno: Some(EACCES),
                ..
            } => write!(
                f,
                "{op}: permission denied on the I2C device (needs the i2c udev rule and group)"
            ),
            Self::Io {
                op,
                errno: Some(EIO),
                ..
            } => write!(f, "{op}: the display did not answer DDC/CI (EIO)"),
            Self::Io { op, detail, .. } => write!(f, "{op}: {detail}"),
            Self::BadReply(err) => write!(f, "no valid DDC/CI reply: {err}"),
            Self::BadEdid(err) => f.write_str(err),
            Self::UnsupportedCode(code) => write!(f, "display does not support VCP {code:#04x}"),
            Self::BadMaximum => f.write_str("display reports a maximum of 0"),
            Self::BusBusy => f.write_str("I2C bus is in use by another process"),
            Self::DisplayChanged => f.write_str("a different monitor is now on this connector"),
            Self::NotMatched(problem) => problem.fmt(f),
            Self::Superseded => f.write_str("superseded by a newer level"),
            Self::TimedOut => f.write_str("DDC/CI operation timed out"),
            Self::WorkerStopped => f.write_str("output was removed"),
        }
    }
}

impl std::error::Error for DdcError {}

impl From<DdcError> for BackendError {
    fn from(err: DdcError) -> Self {
        match err {
            DdcError::TimedOut => Self::TimedOut,
            DdcError::UnsupportedCode(_) => Self::Unsupported,
            DdcError::Superseded => Self::Superseded,
            other => Self::Unavailable(other.to_string()),
        }
    }
}

/// Why a [`DdcConfig`] was refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigError(pub &'static str);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid DDC configuration: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

/// Tunables. The defaults are the DDC/CI 1.1 spec's timings and ddcutil's
/// default try count. None is measured on the Samsung Frame: no DDC/CI
/// exchange has ever succeeded with it (docs/ddc-findings.md), so there is
/// nothing to measure yet. They are not Samsung-specific.
#[derive(Clone, Debug, PartialEq)]
pub struct DdcConfig {
    /// MCCS code written for brightness. Default `0x10` (luminance).
    pub brightness_code: u8,
    /// Scales the spec's delays (40 ms write-to-read, 50 ms after a set).
    /// Default 1.0 (ddcutil's default); 0.1 to 10. Slow displays need more.
    pub sleep_multiplier: f32,
    /// Tries per exchange, the first included. Default 4 (ddcutil's
    /// write-read default); 1 to 10.
    pub max_tries: u8,
    /// Longest any call waits, and the worker's deadline for one job.
    /// Default 3 s, above the worst case of 4 failed tries at multiplier
    /// 5 (1.85 s); must cover [`Timing::worst_case`].
    pub call_timeout: Duration,
    /// After a display fails, how long before a rescan may probe it again.
    /// Default 30 s. A hotplug of that display probes it at once.
    pub reprobe_backoff: Duration,
    /// Failed writes in a row before a display that answered is declared
    /// unavailable. Default 2.
    pub max_write_failures: u8,
    /// Root of the sysfs tree; `/` except in tests.
    pub sysfs_root: PathBuf,
    /// Where the `i2c-N` nodes are; `/dev` except in tests.
    pub dev_root: PathBuf,
}

impl Default for DdcConfig {
    fn default() -> Self {
        Self {
            brightness_code: 0x10,
            sleep_multiplier: 1.0,
            max_tries: 4,
            call_timeout: Duration::from_secs(3),
            reprobe_backoff: Duration::from_secs(30),
            max_write_failures: 2,
            sysfs_root: PathBuf::from("/"),
            dev_root: PathBuf::from("/dev"),
        }
    }
}

impl DdcConfig {
    /// The exchange timing this configuration gives.
    #[must_use]
    pub fn timing(&self) -> Timing {
        Timing::from_spec(self.sleep_multiplier, self.max_tries)
    }

    /// # Errors
    /// Names the first setting out of range.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !(self.sleep_multiplier.is_finite() && (0.1..=10.0).contains(&self.sleep_multiplier)) {
            return Err(ConfigError("sleep_multiplier must be within 0.1 to 10"));
        }
        if !(1..=10).contains(&self.max_tries) {
            return Err(ConfigError("max_tries must be within 1 to 10"));
        }
        if self.max_write_failures == 0 {
            return Err(ConfigError("max_write_failures must be at least 1"));
        }
        if self.call_timeout < self.timing().worst_case() {
            return Err(ConfigError(
                "call_timeout is shorter than the retries and delays it must cover",
            ));
        }
        if self.call_timeout > Duration::from_secs(60) {
            return Err(ConfigError("call_timeout must be at most 60 s"));
        }
        Ok(())
    }
}

/// What the backend knows about one output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayInfo {
    pub id: OutputId,
    /// DRM connector, `card0-HDMI-A-1`.
    pub connector: String,
    /// The I2C bus matched to it, if any.
    pub bus: Option<u32>,
    /// The identity settings are keyed by; `None` without a valid EDID.
    pub identity: Option<EdidId>,
    /// Why there is no bus, when there is none.
    pub problem: Option<MatchProblem>,
    pub health: Health,
}

struct Display {
    connector: String,
    identity: Option<EdidId>,
    matched: Result<Worker, MatchProblem>,
}

#[derive(Default)]
struct Inventory {
    /// Connected connectors and their EDIDs at the last scan; a change is a
    /// hotplug.
    fingerprint: Option<Vec<(String, Option<Vec<u8>>)>>,
    /// By output ID (`OutputId`'s string; it is not `Ord`).
    displays: BTreeMap<String, Display>,
    outputs: Vec<Output>,
}

struct Inner {
    config: DdcConfig,
    sysfs: Sysfs,
    opener: Arc<dyn BusOpener>,
    inventory: Mutex<Inventory>,
    /// Serializes scans, which do bus I/O.
    scan: Mutex<()>,
}

impl Inner {
    fn inventory(&self) -> MutexGuard<'_, Inventory> {
        self.inventory
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn worker_config(&self) -> WorkerConfig {
        WorkerConfig {
            code: self.config.brightness_code,
            timing: self.config.timing(),
            job_timeout: self.config.call_timeout,
            max_write_failures: self.config.max_write_failures,
        }
    }
}

/// The DDC/CI backend. Cheap to clone; clones share workers and state.
///
/// Its futures need a Tokio runtime with the timer enabled (as
/// `dimmer_core::with_timeout` does). Call [`Backend::outputs`] before the
/// others: until a scan has run, every output is unknown.
#[derive(Clone)]
pub struct DdcBackend {
    inner: Arc<Inner>,
}

impl fmt::Debug for DdcBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DdcBackend")
            .field("config", &self.inner.config)
            .finish_non_exhaustive()
    }
}

impl Default for DdcBackend {
    /// The real `/dev/i2c-*` with [`DdcConfig::default`]. Nothing is opened
    /// until a scan.
    fn default() -> Self {
        // The defaults are valid; a test (`config_defaults_are_valid...`)
        // holds them to it.
        Self::new(DdcConfig::default()).expect("default DDC configuration is valid")
    }
}

impl DdcBackend {
    /// A backend on the real `/dev/i2c-*`. Nothing is opened until a scan.
    ///
    /// # Errors
    /// The configuration is out of range.
    pub fn new(config: DdcConfig) -> Result<Self, ConfigError> {
        let opener = Arc::new(I2cDevOpener::new(&config.dev_root));
        Self::with_opener(config, opener)
    }

    /// A backend on any bus implementation (tests use a scripted one).
    ///
    /// # Errors
    /// The configuration is out of range.
    pub fn with_opener(config: DdcConfig, opener: Arc<dyn BusOpener>) -> Result<Self, ConfigError> {
        config.validate()?;
        Ok(Self {
            inner: Arc::new(Inner {
                sysfs: Sysfs::new(&config.sysfs_root),
                config,
                opener,
                inventory: Mutex::new(Inventory::default()),
                scan: Mutex::new(()),
            }),
        })
    }

    /// Scan again even without a hotplug: re-read every bus's EDID, and
    /// re-probe displays that failed at least [`DdcConfig::reprobe_backoff`]
    /// ago (sooner ones keep their cached failure).
    ///
    /// # Errors
    /// `Unavailable` when sysfs cannot be read; `TimedOut` past the deadline.
    pub async fn rescan(&self) -> Result<Vec<Output>, BackendError> {
        scan_off_thread(self.inner.clone(), true).await
    }

    /// Everything known about each output from the last scan.
    #[must_use]
    pub fn displays(&self) -> Vec<DisplayInfo> {
        self.inner
            .inventory()
            .displays
            .iter()
            .map(|(id, display)| DisplayInfo {
                id: OutputId(id.clone()),
                connector: display.connector.clone(),
                bus: display.matched.as_ref().ok().map(|w| w.target().bus),
                identity: display.identity,
                problem: display.matched.as_ref().err().cloned(),
                health: display
                    .matched
                    .as_ref()
                    .map_or(Health::Unprobed, Worker::health),
            })
            .collect()
    }

    /// Read the display's current value (a DDC read; a cached failure
    /// answers at once).
    ///
    /// # Errors
    /// `UnknownOutput`, `Unavailable` with the reason, or `TimedOut`.
    pub async fn read_value(&self, output: &OutputId) -> Result<VcpValue, BackendError> {
        let rx = self.with_worker(output, Worker::request_read)?;
        await_reply(rx, self.inner.config.call_timeout).await
    }

    fn with_worker<T>(
        &self,
        output: &OutputId,
        request: impl FnOnce(&Worker) -> T,
    ) -> Result<T, BackendError> {
        let inventory = self.inner.inventory();
        let display = inventory
            .displays
            .get(&output.0)
            .ok_or(BackendError::UnknownOutput)?;
        match &display.matched {
            Ok(worker) => Ok(request(worker)),
            Err(problem) => Err(DdcError::NotMatched(problem.clone()).into()),
        }
    }
}

async fn await_reply<T>(
    rx: oneshot::Receiver<Result<T, DdcError>>,
    timeout: Duration,
) -> Result<T, BackendError> {
    match tokio::time::timeout(timeout, rx).await {
        Err(_) => Err(BackendError::TimedOut),
        Ok(Err(_)) => Err(DdcError::WorkerStopped.into()),
        Ok(Ok(result)) => result.map_err(BackendError::from),
    }
}

/// Run a scan on its own thread; the caller waits at most the call timeout.
async fn scan_off_thread(inner: Arc<Inner>, force: bool) -> Result<Vec<Output>, BackendError> {
    let timeout = inner.config.call_timeout;
    let (tx, rx) = oneshot::channel();
    thread::Builder::new()
        .name("dimmer-ddc-scan".into())
        .spawn(move || {
            let _ = tx.send(scan(&inner, force).map_err(|e| exchange::io_error("sysfs scan", &e)));
        })
        .map_err(|e| BackendError::Unavailable(format!("cannot start scan thread: {e}")))?;
    match tokio::time::timeout(timeout, rx).await {
        Err(_) => Err(BackendError::TimedOut),
        Ok(Err(_)) => Err(BackendError::Unavailable("scan thread stopped".into())),
        Ok(Ok(result)) => result.map_err(|err| BackendError::Unavailable(err.to_string())),
    }
}

/// Discover connectors and buses and update the inventory. Bus I/O only on
/// a hotplug (connected connectors or their EDIDs changed) or when forced.
fn scan(inner: &Inner, force: bool) -> std::io::Result<Vec<Output>> {
    let _serial = inner.scan.lock().unwrap_or_else(PoisonError::into_inner);
    let connectors = inner.sysfs.connectors()?;
    let connected: Vec<&Connector> = connectors.iter().filter(|c| c.connected).collect();
    let fingerprint: Vec<_> = connected
        .iter()
        .map(|c| (c.sysfs_name.clone(), c.edid.clone()))
        .collect();
    let hotplug = inner.inventory().fingerprint.as_ref() != Some(&fingerprint);
    if !hotplug && !force {
        return Ok(inner.inventory().outputs.clone());
    }

    let adapters = inner.sysfs.adapters()?;
    let probed = probe_buses(inner, &adapters, &connectors);
    let assignments = matching::assign(&connected, &probed);

    let mut inventory = inner.inventory();
    let mut previous = std::mem::take(&mut inventory.displays);
    let mut outputs = Vec::new();
    let ids = output_ids(&connected);
    for (index, (connector, assignment)) in connected.iter().zip(assignments).enumerate() {
        let id = ids[index].clone();
        let edid = connector.edid.as_deref().and_then(|e| Edid::parse(e).ok());
        let identity = connector
            .edid
            .as_deref()
            .and_then(|e| EdidId::from_edid(e).ok());
        let matched = worker_for(
            inner,
            previous.remove(&id.0),
            connector,
            assignment,
            edid.as_ref(),
        );
        // Without an EDID the connector alone names it: the applet adds no
        // English word of its own to a name it shows.
        let name = edid.as_ref().map_or_else(
            || connector.name.clone(),
            |edid| format!("{} ({})", edid.label(), connector.name),
        );
        outputs.push(Output {
            id: id.clone(),
            name,
        });
        inventory.displays.insert(
            id.0,
            Display {
                connector: connector.sysfs_name.clone(),
                identity,
                matched,
            },
        );
    }
    // Workers of displays that are gone stop when `previous` drops here.
    drop(previous);
    inventory.fingerprint = Some(fingerprint);
    inventory.outputs.clone_from(&outputs);
    Ok(outputs)
}

/// Read the EDID on every bus that could carry a connected display: buses of
/// display controllers only, and never one the kernel ties to a connector
/// with nothing plugged in. No bus I/O at all when no connector has an EDID
/// to match against.
fn probe_buses<'a>(
    inner: &Inner,
    adapters: &'a [Adapter],
    connectors: &[Connector],
) -> Vec<ProbedBus<'a>> {
    if !connectors.iter().any(|c| c.connected && c.edid.is_some()) {
        return Vec::new();
    }
    adapters
        .iter()
        .filter(|a| a.on_display_controller)
        .filter(|a| !connectors.iter().any(|c| !c.connected && a.tied_to(c)))
        .map(|adapter| ProbedBus {
            adapter,
            edid: read_bus_edid(inner, adapter.bus),
        })
        .collect()
}

/// The worker for a connected display: the previous scan's when it still
/// drives the same bus and monitor and has not been failing for longer than
/// the re-probe backoff, else a new one on the assigned bus.
fn worker_for(
    inner: &Inner,
    previous: Option<Display>,
    connector: &Connector,
    assignment: Result<u32, MatchProblem>,
    edid: Option<&Edid>,
) -> Result<Worker, MatchProblem> {
    let reuse = previous.and_then(|old| {
        let worker = old.matched.ok()?;
        let same = Ok(worker.target().bus) == assignment
            && Some(&worker.target().edid) == edid
            && worker.target().connector == connector.sysfs_name;
        let failed_long_ago = matches!(
            worker.health(),
            Health::Unavailable { since, .. }
                if since.elapsed() >= inner.config.reprobe_backoff
        );
        (same && !failed_long_ago).then_some(worker)
    });
    match (reuse, assignment, edid) {
        (Some(worker), _, _) => Ok(worker),
        (None, Ok(bus), Some(edid)) => Worker::spawn(
            Target {
                bus,
                connector: connector.sysfs_name.clone(),
                edid: edid.clone(),
                sysfs: inner.sysfs.clone(),
                opener: inner.opener.clone(),
            },
            inner.worker_config(),
        )
        .map_err(|_| MatchProblem::NoMatchingBus),
        (None, Ok(_), None) => Err(MatchProblem::NoConnectorEdid),
        (None, Err(problem), _) => Err(problem),
    }
}

/// One EDID read on one bus, holding its advisory lock briefly. Any failure
/// means "not this bus" for the scan.
fn read_bus_edid(inner: &Inner, bus: u32) -> Option<Edid> {
    let mut handle = inner.opener.open(bus).ok()?;
    handle
        .lock(Instant::now() + Duration::from_millis(500))
        .ok()?;
    exchange::read_edid(handle.as_mut()).ok()
}

/// Output IDs, by the convention every backend shares so the applet can
/// pair a display's hardware and alpha controls (docs/applet.md): the
/// [`EdidId::entry_name`] of the connector's EDID (which, for a matched
/// display, is byte-identical to the EDID read on its bus), else
/// `connector:<name>`. Two connected displays with the same entry name
/// (identical monitors without serial numbers) both fall back to their
/// connector, so no ID ever names two displays.
fn output_ids(connected: &[&Connector]) -> Vec<OutputId> {
    let entries: Vec<Option<String>> = connected
        .iter()
        .map(|c| {
            let edid = c.edid.as_deref()?;
            EdidId::from_edid(edid).ok().map(|id| id.entry_name())
        })
        .collect();
    connected
        .iter()
        .zip(&entries)
        .map(|(connector, entry)| {
            let unique = entry.as_ref().filter(|e| {
                entries
                    .iter()
                    .filter(|other| other.as_ref() == Some(*e))
                    .count()
                    == 1
            });
            OutputId(if let Some(entry) = unique {
                entry.clone()
            } else {
                let same_name = connected
                    .iter()
                    .filter(|c| c.name == connector.name)
                    .count();
                if same_name > 1 {
                    format!("connector:{}", connector.sysfs_name)
                } else {
                    format!("connector:{}", connector.name)
                }
            })
        })
        .collect()
}

impl Backend for DdcBackend {
    /// Connected monitors, including those without a usable bus (they report
    /// `Unavailable` from [`Backend::capabilities`], so the applet can show
    /// them as hardware-unavailable). Rescans only after a hotplug.
    fn outputs(&self) -> impl Future<Output = Result<Vec<Output>, BackendError>> + Send {
        scan_off_thread(self.inner.clone(), false)
    }

    /// Probes the display on first use; afterwards answers from its state.
    fn capabilities(
        &self,
        output: &OutputId,
    ) -> impl Future<Output = Result<Capabilities, BackendError>> + Send {
        let backend = self.clone();
        let output = output.clone();
        async move {
            let ready = Capabilities {
                kind: LevelKind::Hardware,
                writable: true,
            };
            match backend.with_worker(&output, Worker::health)? {
                Health::Ready(_) => Ok(ready),
                Health::Unavailable { reason, .. } => Err(reason.into()),
                Health::Unprobed => backend.read_value(&output).await.map(|_| ready),
            }
        }
    }

    /// Queues the level and waits for the write. A newer level for the same
    /// output, arriving before this one is written, supersedes it (error).
    fn set_level(
        &self,
        output: &OutputId,
        level: Level,
    ) -> impl Future<Output = Result<(), BackendError>> + Send {
        let backend = self.clone();
        let output = output.clone();
        async move {
            let rx = backend.with_worker(&output, |w| w.request_write(level))?;
            await_reply(rx, backend.inner.config.call_timeout).await
        }
    }
}

#[cfg(test)]
mod tests;
