//! One thread per matched display: all of that display's I2C traffic, one
//! job at a time, so a slow or silent monitor never holds up another one or
//! the caller. Rapid level changes coalesce into the latest.

use std::{
    io,
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread,
    time::{Duration, Instant},
};

use dimmer_core::Level;
use tokio::sync::oneshot;

use crate::{
    DdcError,
    edid::Edid,
    exchange::{Timing, get_vcp, io_error, set_vcp},
    protocol::{VcpValue, level_to_raw},
    sysfs::Sysfs,
    transport::BusOpener,
};

/// Where one display's I/O goes, and what to check before using it.
#[derive(Clone)]
pub(crate) struct Target {
    pub bus: u32,
    /// The connector the bus was matched to (`card0-HDMI-A-1`).
    pub connector: String,
    /// The EDID the match was made on.
    pub edid: Edid,
    pub sysfs: Sysfs,
    pub opener: Arc<dyn BusOpener>,
}

impl Target {
    /// The connector still holds the matched EDID. A sysfs read, no bus
    /// traffic; done before every job, so a monitor swapped between scans
    /// is never written with the old one's bus.
    fn still_matched(&self) -> bool {
        self.sysfs
            .connector_edid(&self.connector)
            .and_then(|bytes| Edid::parse(&bytes).ok())
            .is_some_and(|edid| edid == self.edid)
    }
}

/// Settings a worker needs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorkerConfig {
    pub code: u8,
    pub timing: Timing,
    /// Deadline for one job's I/O, from its start.
    pub job_timeout: Duration,
    /// Failed writes in a row before a display that answered reads is
    /// declared unavailable.
    pub max_write_failures: u8,
}

/// What is known about the display.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Health {
    /// Not asked yet.
    Unprobed,
    /// Answered; the last value read or written.
    Ready(VcpValue),
    /// Failed; no I/O until a re-probe (hotplug or rescan after backoff).
    Unavailable { reason: DdcError, since: Instant },
}

type Reply<T> = oneshot::Sender<Result<T, DdcError>>;

struct State {
    health: Health,
    reads: Vec<Reply<VcpValue>>,
    /// The latest level asked for and not started yet, with everyone
    /// waiting on it. A newer level replaces it and supersedes them.
    write: Option<(Level, Vec<Reply<()>>)>,
    write_failures: u8,
    shutdown: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic on the worker leaves consistent data; keep serving.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

enum Job {
    Read(Vec<Reply<VcpValue>>),
    Write(Level, Vec<Reply<()>>),
}

/// Handle to a display's worker thread; dropping it stops the thread after
/// its current job.
pub(crate) struct Worker {
    shared: Arc<Shared>,
    target: Target,
}

impl Worker {
    /// Start the thread. Nothing touches the bus until the first request.
    pub fn spawn(target: Target, config: WorkerConfig) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                health: Health::Unprobed,
                reads: Vec::new(),
                write: None,
                write_failures: 0,
                shutdown: false,
            }),
            wake: Condvar::new(),
        });
        let thread_shared = shared.clone();
        let thread_target = target.clone();
        thread::Builder::new()
            .name(format!("dimmer-ddc-i2c-{}", target.bus))
            .spawn(move || run(&thread_shared, &thread_target, config))?;
        Ok(Self { shared, target })
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    /// The level waiting to be written, if any.
    #[cfg(test)]
    pub fn pending_write(&self) -> Option<Level> {
        self.shared.lock().write.as_ref().map(|(level, _)| *level)
    }

    pub fn health(&self) -> Health {
        self.shared.lock().health.clone()
    }

    /// Ask for the current value. An unavailable display answers at once.
    pub fn request_read(&self) -> oneshot::Receiver<Result<VcpValue, DdcError>> {
        let (tx, rx) = oneshot::channel();
        let mut state = self.shared.lock();
        if let Health::Unavailable { reason, .. } = &state.health {
            let _ = tx.send(Err(reason.clone()));
        } else {
            state.reads.push(tx);
            self.shared.wake.notify_one();
        }
        rx
    }

    /// Ask for `level`. Replaces a level not started yet, whose callers get
    /// [`DdcError::Superseded`]; a write in progress finishes first.
    pub fn request_write(&self, level: Level) -> oneshot::Receiver<Result<(), DdcError>> {
        let (tx, rx) = oneshot::channel();
        let mut state = self.shared.lock();
        if let Health::Unavailable { reason, .. } = &state.health {
            let _ = tx.send(Err(reason.clone()));
            return rx;
        }
        if let Some((_, superseded)) = state.write.take() {
            for waiter in superseded {
                let _ = waiter.send(Err(DdcError::Superseded));
            }
        }
        state.write = Some((level, vec![tx]));
        self.shared.wake.notify_one();
        rx
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.lock().shutdown = true;
        self.shared.wake.notify_all();
    }
}

fn run(shared: &Shared, target: &Target, config: WorkerConfig) {
    loop {
        let job = {
            let mut state = shared.lock();
            loop {
                if state.shutdown {
                    let reads = std::mem::take(&mut state.reads);
                    let write = state.write.take();
                    drop(state);
                    for waiter in reads {
                        let _ = waiter.send(Err(DdcError::WorkerStopped));
                    }
                    for waiter in write.into_iter().flat_map(|(_, w)| w) {
                        let _ = waiter.send(Err(DdcError::WorkerStopped));
                    }
                    return;
                }
                if !state.reads.is_empty() {
                    break Job::Read(std::mem::take(&mut state.reads));
                }
                if let Some((level, waiters)) = state.write.take() {
                    break Job::Write(level, waiters);
                }
                state = shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        match job {
            Job::Read(waiters) => {
                let result = read(shared, target, config);
                for waiter in waiters {
                    let _ = waiter.send(result.clone());
                }
            }
            Job::Write(level, waiters) => {
                // Everyone who asked gave up (timed out, dropped the future):
                // the level is no longer wanted, so it is not sent.
                let abandoned = || waiters.iter().all(oneshot::Sender::is_closed);
                if abandoned() {
                    continue;
                }
                let result = write(shared, target, config, level, abandoned);
                for waiter in waiters {
                    let _ = waiter.send(result.clone());
                }
            }
        }
    }
}

/// Whether a failure says something about the display (rather than about
/// contention for the bus), so it should change the display's health.
fn is_display_failure(err: &DdcError) -> bool {
    !matches!(err, DdcError::BusBusy | DdcError::Superseded)
}

/// Run `io` on an opened, locked bus, after the identity check. An
/// unavailable display fails here without touching the bus.
fn with_bus<T>(
    shared: &Shared,
    target: &Target,
    config: WorkerConfig,
    io: impl FnOnce(&mut dyn crate::transport::Bus, Instant) -> Result<T, DdcError>,
) -> Result<T, DdcError> {
    if let Health::Unavailable { reason, .. } = &shared.lock().health {
        return Err(reason.clone());
    }
    if !target.still_matched() {
        return Err(DdcError::DisplayChanged);
    }
    let deadline = Instant::now() + config.job_timeout;
    let mut bus = target
        .opener
        .open(target.bus)
        .map_err(|e| io_error("open", &e))?;
    bus.lock(deadline).map_err(|e| {
        if e.kind() == io::ErrorKind::WouldBlock {
            DdcError::BusBusy
        } else {
            io_error("lock", &e)
        }
    })?;
    io(bus.as_mut(), deadline)
    // The bus closes here, releasing the lock: no descriptor is kept idle.
}

fn mark_unavailable(shared: &Shared, reason: &DdcError) {
    let mut state = shared.lock();
    if !matches!(state.health, Health::Unavailable { .. }) {
        state.health = Health::Unavailable {
            reason: reason.clone(),
            since: Instant::now(),
        };
    }
    // Queued requests would only fail the same way: answer them now.
    let reads = std::mem::take(&mut state.reads);
    let write = state.write.take();
    drop(state);
    for waiter in reads {
        let _ = waiter.send(Err(reason.clone()));
    }
    for waiter in write.into_iter().flat_map(|(_, w)| w) {
        let _ = waiter.send(Err(reason.clone()));
    }
}

fn read(shared: &Shared, target: &Target, config: WorkerConfig) -> Result<VcpValue, DdcError> {
    let result = with_bus(shared, target, config, |bus, deadline| {
        get_vcp(bus, config.code, config.timing, deadline)
    });
    match &result {
        Ok(value) => {
            let mut state = shared.lock();
            state.health = Health::Ready(*value);
            state.write_failures = 0;
        }
        Err(err) if is_display_failure(err) => mark_unavailable(shared, err),
        Err(_) => {}
    }
    result
}

fn write(
    shared: &Shared,
    target: &Target,
    config: WorkerConfig,
    level: Level,
    abandoned: impl Fn() -> bool,
) -> Result<(), DdcError> {
    // The raw scale comes from the display; probe it first if needed. The
    // state lock must be released before `read`, which takes it again.
    let health = shared.lock().health.clone();
    let maximum = match health {
        Health::Ready(value) => value.maximum,
        Health::Unprobed => read(shared, target, config)?.maximum,
        Health::Unavailable { reason, .. } => return Err(reason),
    };
    if abandoned() {
        return Err(DdcError::TimedOut);
    }
    if maximum == 0 {
        let err = DdcError::BadMaximum;
        mark_unavailable(shared, &err);
        return Err(err);
    }
    let raw = level_to_raw(level, maximum);
    let result = with_bus(shared, target, config, |bus, deadline| {
        set_vcp(bus, config.code, raw, config.timing, deadline)
    });
    match &result {
        Ok(()) => {
            let mut state = shared.lock();
            state.health = Health::Ready(VcpValue {
                current: raw,
                maximum,
            });
            state.write_failures = 0;
        }
        Err(err @ (DdcError::DisplayChanged | DdcError::UnsupportedCode(_))) => {
            mark_unavailable(shared, err);
        }
        Err(err) if is_display_failure(err) => {
            let failures = {
                let mut state = shared.lock();
                state.write_failures = state.write_failures.saturating_add(1);
                state.write_failures
            };
            if failures >= config.max_write_failures {
                mark_unavailable(shared, err);
            }
        }
        Err(_) => {}
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        edid::tests::SAMSUNG_FRAME,
        protocol::{
            DDC_ADDRESS, get_vcp_request, set_vcp_request, tests::REPLY_BRIGHTNESS_50_OF_100,
        },
        sysfs::fixture::Tree,
        transport::fake::{FakeOpener, Gate, Step},
    };

    const EIO: i32 = 5;

    struct Setup {
        tree: Tree,
        opener: FakeOpener,
    }

    impl Setup {
        fn new() -> Self {
            let tree = Tree::new();
            let gpu = tree.pci("0000:01:00.0", "0x030000");
            tree.connector(&gpu, "card0-HDMI-A-1", true, &SAMSUNG_FRAME, None);
            Self {
                tree,
                opener: FakeOpener::default(),
            }
        }

        fn worker(&self, max_write_failures: u8) -> Worker {
            let target = Target {
                bus: 3,
                connector: "card0-HDMI-A-1".into(),
                edid: Edid::parse(&SAMSUNG_FRAME).unwrap(),
                sysfs: Sysfs::new(&self.tree.root),
                opener: Arc::new(self.opener.clone()),
            };
            Worker::spawn(
                target,
                WorkerConfig {
                    code: 0x10,
                    timing: Timing {
                        write_to_read: Duration::ZERO,
                        after_set: Duration::ZERO,
                        retry: Duration::ZERO,
                        max_tries: 2,
                    },
                    job_timeout: Duration::from_secs(10),
                    max_write_failures,
                },
            )
            .unwrap()
        }
    }

    fn probe_ok() -> [Step; 2] {
        [
            Step::Write(DDC_ADDRESS, get_vcp_request(0x10).to_vec(), None),
            Step::Read(DDC_ADDRESS, Ok(REPLY_BRIGHTNESS_50_OF_100.to_vec())),
        ]
    }

    fn set(value: u16, errno: Option<i32>) -> Step {
        Step::Write(DDC_ADDRESS, set_vcp_request(0x10, value).to_vec(), errno)
    }

    fn level(percent: u8) -> Level {
        Level::new(percent).unwrap()
    }

    #[test]
    fn first_write_probes_the_scale_then_writes() {
        let setup = Setup::new();
        setup.opener.push(3, probe_ok());
        setup.opener.push(3, [set(40, None)]);
        let worker = setup.worker(2);
        assert_eq!(
            worker.request_write(level(40)).blocking_recv().unwrap(),
            Ok(())
        );
        assert_eq!(
            worker.health(),
            Health::Ready(VcpValue {
                current: 40,
                maximum: 100
            })
        );
        assert_eq!(setup.opener.remaining(3), 0);
    }

    #[test]
    fn rapid_moves_coalesce_into_the_latest() {
        let setup = Setup::new();
        let gate = Arc::new(Gate::default());
        setup.opener.push(3, probe_ok());
        setup
            .opener
            .push(3, [Step::Gate(gate.clone()), set(20, None), set(80, None)]);
        let worker = setup.worker(2);

        let first = worker.request_write(level(20));
        gate.wait_until_held(); // the worker is inside the write of 20
        let second = worker.request_write(level(40));
        let third = worker.request_write(level(60));
        let last = worker.request_write(level(80));
        gate.open();

        assert_eq!(first.blocking_recv().unwrap(), Ok(()));
        assert_eq!(second.blocking_recv().unwrap(), Err(DdcError::Superseded));
        assert_eq!(third.blocking_recv().unwrap(), Err(DdcError::Superseded));
        assert_eq!(last.blocking_recv().unwrap(), Ok(()));
        let sets: Vec<_> = setup
            .opener
            .log(3)
            .into_iter()
            .filter(|l| l.contains("[51, 84,"))
            .collect();
        assert_eq!(sets.len(), 2, "only 20 and 80 reach the bus: {sets:?}");
    }

    #[test]
    fn eio_display_is_unavailable_and_not_retried() {
        let setup = Setup::new();
        let eio = Step::Write(DDC_ADDRESS, get_vcp_request(0x10).to_vec(), Some(EIO));
        setup.opener.push(3, [eio.clone(), eio]);
        let worker = setup.worker(2);
        let err = worker.request_read().blocking_recv().unwrap().unwrap_err();
        assert!(matches!(
            err,
            DdcError::Io {
                errno: Some(EIO),
                ..
            }
        ));
        assert!(matches!(worker.health(), Health::Unavailable { .. }));
        let traffic = setup.opener.log(3).len();
        assert_eq!(traffic, 2);

        // Later calls answer at once from the cached state: no bus traffic.
        let start = Instant::now();
        for _ in 0..50 {
            assert_eq!(
                worker.request_write(level(30)).blocking_recv().unwrap(),
                Err(err.clone())
            );
            assert_eq!(
                worker.request_read().blocking_recv().unwrap(),
                Err(err.clone())
            );
        }
        assert!(start.elapsed() < Duration::from_millis(500));
        assert_eq!(setup.opener.log(3).len(), traffic);
    }

    #[test]
    fn repeated_write_failures_make_it_unavailable() {
        let setup = Setup::new();
        setup.opener.push(3, probe_ok());
        setup
            .opener
            .push(3, [set(30, Some(EIO)), set(30, Some(EIO))]);
        setup
            .opener
            .push(3, [set(31, Some(EIO)), set(31, Some(EIO))]);
        let worker = setup.worker(2);
        assert!(
            worker
                .request_write(level(30))
                .blocking_recv()
                .unwrap()
                .is_err()
        );
        assert!(matches!(worker.health(), Health::Ready(_)));
        assert!(
            worker
                .request_write(level(31))
                .blocking_recv()
                .unwrap()
                .is_err()
        );
        assert!(matches!(worker.health(), Health::Unavailable { .. }));
    }

    #[test]
    fn swapped_monitor_is_never_written() {
        let setup = Setup::new();
        setup.opener.push(3, probe_ok());
        let worker = setup.worker(2);
        assert!(worker.request_read().blocking_recv().unwrap().is_ok());
        // Another monitor now sits on the connector; no rescan has run yet.
        setup
            .tree
            .set_edid("card0-HDMI-A-1", &crate::edid::tests::other_monitor());
        assert_eq!(
            worker.request_write(level(30)).blocking_recv().unwrap(),
            Err(DdcError::DisplayChanged)
        );
        assert_eq!(setup.opener.remaining(3), 0);
        assert_eq!(
            setup.opener.log(3).len(),
            2,
            "only the probe touched the bus"
        );
    }

    #[test]
    fn permission_denied_is_reported_as_such() {
        let setup = Setup::new();
        setup.opener.script(3).lock().unwrap().open_error = Some(13);
        let worker = setup.worker(2);
        let err = worker.request_read().blocking_recv().unwrap().unwrap_err();
        assert!(matches!(
            err,
            DdcError::Io {
                op: "open",
                errno: Some(13),
                ..
            }
        ));
        assert!(err.to_string().contains("udev"), "{err}");
    }

    #[test]
    fn dropping_the_handle_stops_the_worker() {
        let setup = Setup::new();
        let gate = Arc::new(Gate::default());
        setup.opener.push(3, [Step::Gate(gate.clone())]);
        setup.opener.push(3, probe_ok());
        let worker = setup.worker(2);
        let held = worker.request_read();
        gate.wait_until_held();
        let queued = worker.request_write(level(10));
        drop(worker);
        gate.open();
        assert!(held.blocking_recv().unwrap().is_ok());
        assert_eq!(
            queued.blocking_recv().unwrap(),
            Err(DdcError::WorkerStopped)
        );
    }
}
