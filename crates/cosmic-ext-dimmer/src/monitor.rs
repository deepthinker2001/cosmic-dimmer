//! What the applet knows about each display, independent of the UI toolkit.
//!
//! A [`Monitor`] pairs a hardware output and an alpha output (each may be
//! missing), holds its [`MonitorLevels`] and tracks what each backend has
//! confirmed. Every operation returns the [`Write`]s to start; the caller runs
//! them asynchronously and reports back with [`Monitor::finished`], so the UI
//! never waits for a display (R10).
//!
//! Writes are coalesced here, per channel: at most one is in flight, and while
//! it runs only the newest requested level is kept, so dragging a slider over
//! a slow DDC/CI display sends the latest value next instead of a backlog.

use std::time::Duration;

use dimmer_core::profile::{Profile, Target, step_toward};
use dimmer_core::sun::Phase;
use dimmer_core::{
    Backend, BackendError, Change, EdidId, Floors, MonitorLevels, Output, OutputId, with_timeout,
};

/// Which of a monitor's two levels.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Channel {
    Hardware,
    Alpha,
}

/// Why a channel has no usable output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Unavailable {
    /// The backend works but did not list this display: for hardware, the
    /// display does not answer DDC/CI.
    NotFound,
    /// The backend lists the display but cannot write it.
    ReadOnly,
    /// The backend itself failed (or timed out) during discovery.
    Backend(BackendError),
}

/// A channel's output, or why there is none.
pub type Endpoint = Result<OutputId, Unavailable>;

/// One display as discovery found it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Found {
    /// The output ID both backends use for it: the pairing key. By the
    /// overlay backend's convention (docs/overlay-backend.md) it is the EDID
    /// identity, [`EdidId::entry_name`], or `connector:<name>` without one.
    pub key: String,
    /// What the backend calls it (the overlay: model and connector).
    pub name: String,
    pub hardware: Endpoint,
    pub alpha: Endpoint,
}

impl Found {
    /// The EDID identity the key names, which keys saved levels; `None` for
    /// a connector or mock key, whose levels are not saved.
    #[must_use]
    pub fn edid(&self) -> Option<EdidId> {
        EdidId::from_entry_name(&self.key)
    }
}

/// A write for the caller to run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Write {
    pub channel: Channel,
    pub output: OutputId,
    pub level: u8,
}

/// One channel's write pipeline and what the backend last confirmed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WriteState {
    in_flight: Option<u8>,
    queued: Option<u8>,
    confirmed: Option<u8>,
    error: Option<BackendError>,
}

impl WriteState {
    /// Ask for `level`; returns it when it should be sent now.
    pub fn request(&mut self, level: u8) -> Option<u8> {
        if self.in_flight.is_some() {
            self.queued = Some(level);
            None
        } else {
            self.in_flight = Some(level);
            Some(level)
        }
    }

    /// The write of `level` ended; returns the next level to send, if any.
    pub fn finished(&mut self, level: u8, result: &Result<(), BackendError>) -> Option<u8> {
        if self.in_flight == Some(level) {
            self.in_flight = None;
        }
        match result {
            Ok(()) => {
                self.confirmed = Some(level);
                self.error = None;
            }
            // A newer level replaced this one inside the backend; its own
            // result will arrive.
            Err(BackendError::Superseded) => {}
            Err(error) => self.error = Some(error.clone()),
        }
        if self.in_flight.is_some() {
            return None;
        }
        let next = self.queued.take()?;
        self.in_flight = Some(next);
        Some(next)
    }

    #[must_use]
    pub fn confirmed(&self) -> Option<u8> {
        self.confirmed
    }

    #[must_use]
    pub fn error(&self) -> Option<&BackendError> {
        self.error.as_ref()
    }
}

/// A display in the popup.
#[derive(Clone, Debug)]
pub struct Monitor {
    pub found: Found,
    levels: MonitorLevels,
    /// The levels the user chose at times of day, for auto dimming.
    profile: Profile,
    hardware_writes: WriteState,
    alpha_writes: WriteState,
}

impl Monitor {
    /// A display at `levels` (normally loaded from the store), with `floors`
    /// applied and a missing channel held at 100 %.
    #[must_use]
    pub fn new(found: Found, levels: MonitorLevels, floors: Floors) -> Self {
        let mut monitor = Self {
            found,
            levels,
            profile: Profile::default(),
            hardware_writes: WriteState::default(),
            alpha_writes: WriteState::default(),
        };
        let _ = monitor.levels.set_floors(monitor.fitted(floors));
        monitor.hold_unavailable();
        monitor
    }

    /// The floors this display obeys: without hardware, the alpha floor alone.
    fn fitted(&self, floors: Floors) -> Floors {
        if self.available(Channel::Hardware) {
            floors
        } else {
            floors.software_only()
        }
    }

    /// A channel the applet cannot write is kept at 100 % in the model: the
    /// floors assume both levels are real, and a lock would otherwise move a
    /// level that does not exist.
    fn hold_unavailable(&mut self) {
        let hardware = self.available(Channel::Hardware);
        let alpha = self.available(Channel::Alpha);
        if !hardware || !alpha {
            self.levels = MonitorLevels::clamped(
                if hardware {
                    self.levels.hardware().percent()
                } else {
                    100
                },
                if alpha {
                    self.levels.alpha().percent()
                } else {
                    100
                },
                false,
                self.levels.floors(),
            );
        }
    }

    /// The same display with a saved auto-dimming profile.
    #[must_use]
    pub fn with_profile(mut self, profile: Profile) -> Self {
        self.profile = profile;
        self
    }

    #[must_use]
    pub fn levels(&self) -> &MonitorLevels {
        &self.levels
    }

    #[must_use]
    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// Remember the current levels as the user's choice at `phase`.
    pub fn learn(&mut self, phase: Phase) {
        let hardware = self.level(Channel::Hardware);
        let alpha = self.level(Channel::Alpha);
        self.profile.learn(phase, hardware, alpha);
    }

    /// Move each level one percent toward `target`, within the floors. When
    /// the levels are locked, the software level leads and the lock carries
    /// the hardware level with it.
    pub fn follow(&mut self, target: Target) -> Vec<Write> {
        let mut change = Change::default();
        if self.available(Channel::Alpha) {
            let next = step_toward(self.level(Channel::Alpha), target.alpha);
            let moved = self.levels.set_alpha(next);
            change.hardware |= moved.hardware;
            change.alpha |= moved.alpha;
        }
        if self.available(Channel::Hardware) && !self.levels.locked() {
            let next = step_toward(self.level(Channel::Hardware), target.hardware);
            change.hardware |= self.levels.set_hardware(next).hardware;
        }
        self.apply(change)
    }

    #[must_use]
    pub fn edid(&self) -> Option<EdidId> {
        self.found.edid()
    }

    pub fn endpoint(&self, channel: Channel) -> &Endpoint {
        match channel {
            Channel::Hardware => &self.found.hardware,
            Channel::Alpha => &self.found.alpha,
        }
    }

    #[must_use]
    pub fn available(&self, channel: Channel) -> bool {
        self.endpoint(channel).is_ok()
    }

    /// Locking needs both levels.
    #[must_use]
    pub fn can_lock(&self) -> bool {
        self.available(Channel::Hardware) && self.available(Channel::Alpha)
    }

    #[must_use]
    pub fn level(&self, channel: Channel) -> u8 {
        match channel {
            Channel::Hardware => self.levels.hardware().percent(),
            Channel::Alpha => self.levels.alpha().percent(),
        }
    }

    #[must_use]
    pub fn writes(&self, channel: Channel) -> &WriteState {
        match channel {
            Channel::Hardware => &self.hardware_writes,
            Channel::Alpha => &self.alpha_writes,
        }
    }

    /// The lowest value the channel's slider can reach now: its own floor,
    /// or higher where the effective floor (or the lock) stops it first.
    #[must_use]
    pub fn minimum(&self, channel: Channel) -> u8 {
        let mut probe = self.levels;
        let _ = match channel {
            Channel::Hardware => probe.set_hardware(0),
            Channel::Alpha => probe.set_alpha(0),
        };
        match channel {
            Channel::Hardware => probe.hardware().percent(),
            Channel::Alpha => probe.alpha().percent(),
        }
    }

    /// Move one slider (both when locked).
    pub fn set(&mut self, channel: Channel, percent: u8) -> Vec<Write> {
        if !self.available(channel) {
            return Vec::new();
        }
        let change = match channel {
            Channel::Hardware => self.levels.set_hardware(percent),
            Channel::Alpha => self.levels.set_alpha(percent),
        };
        self.apply(change)
    }

    pub fn set_locked(&mut self, locked: bool) {
        self.levels.set_locked(locked && self.can_lock());
    }

    /// The terminal recovery: both levels to 100 % and both written even if
    /// the model already says 100, in case the display and the model disagree.
    pub fn force_reset(&mut self) -> Vec<Write> {
        let before = self.levels;
        self.levels = MonitorLevels::clamped(100, 100, before.locked(), before.floors());
        self.apply(Change {
            hardware: true,
            alpha: true,
        })
    }

    pub fn set_floors(&mut self, floors: Floors) -> Vec<Write> {
        let change = self.levels.set_floors(self.fitted(floors));
        self.apply(change)
    }

    /// A brightness-up key: dimmer-core lifts the lowest level (R12).
    pub fn raise(&mut self, step: u8) -> Vec<Write> {
        let change = self.levels.raise(step);
        self.apply(change)
    }

    /// A brightness-down key: lowers the highest available level (both on a
    /// tie, or when locked), stopping at the floors.
    pub fn lower(&mut self, step: u8) -> Vec<Write> {
        let hardware = self.level(Channel::Hardware);
        let alpha = self.level(Channel::Alpha);
        let channels: &[Channel] = match (
            self.available(Channel::Hardware),
            self.available(Channel::Alpha),
        ) {
            (false, false) => &[],
            (true, false) => &[Channel::Hardware],
            (false, true) => &[Channel::Alpha],
            // A locked move already moves both.
            _ if self.levels.locked() || hardware > alpha => &[Channel::Hardware],
            _ if alpha > hardware => &[Channel::Alpha],
            _ => &[Channel::Hardware, Channel::Alpha],
        };
        let mut change = Change::default();
        for channel in channels {
            let moved = match channel {
                Channel::Hardware => self.levels.set_hardware(hardware.saturating_sub(step)),
                Channel::Alpha => self.levels.set_alpha(alpha.saturating_sub(step)),
            };
            change.hardware |= moved.hardware;
            change.alpha |= moved.alpha;
        }
        self.apply(change)
    }

    /// Writes restoring saved levels at startup: only levels below 100 %, so
    /// a display at full keeps whatever its own controls set.
    pub fn restore(&mut self) -> Vec<Write> {
        self.apply(Change {
            hardware: self.level(Channel::Hardware) < 100,
            alpha: self.level(Channel::Alpha) < 100,
        })
    }

    /// A write ended; returns the follow-up write, if a newer level waits.
    pub fn finished(
        &mut self,
        channel: Channel,
        level: u8,
        result: &Result<(), BackendError>,
    ) -> Option<Write> {
        let next = match channel {
            Channel::Hardware => self.hardware_writes.finished(level, result),
            Channel::Alpha => self.alpha_writes.finished(level, result),
        }?;
        let output = self.endpoint(channel).clone().ok()?;
        Some(Write {
            channel,
            output,
            level: next,
        })
    }

    /// Replace what discovery knows, keeping levels and write state. Hardware
    /// may have come or gone, which changes the floors this display obeys.
    pub fn rediscovered(&mut self, found: Found, floors: Floors) -> Vec<Write> {
        self.found = found;
        self.set_floors(floors)
    }

    fn apply(&mut self, change: Change) -> Vec<Write> {
        self.hold_unavailable();
        let mut writes = Vec::new();
        for (channel, changed) in [
            (Channel::Hardware, change.hardware),
            (Channel::Alpha, change.alpha),
        ] {
            let Ok(output) = self.endpoint(channel).clone() else {
                continue;
            };
            if !changed {
                continue;
            }
            let level = self.level(channel);
            let state = match channel {
                Channel::Hardware => &mut self.hardware_writes,
                Channel::Alpha => &mut self.alpha_writes,
            };
            if let Some(level) = state.request(level) {
                writes.push(Write {
                    channel,
                    output,
                    level,
                });
            }
        }
        writes
    }
}

/// One backend's outputs with a usable flag each, bounded by `timeout`.
async fn list<B: Backend>(backend: &B, timeout: Duration) -> Listed {
    let outputs = with_timeout(timeout, backend.outputs()).await?;
    let mut listed = Vec::with_capacity(outputs.len());
    for output in outputs {
        let usable = match with_timeout(timeout, backend.capabilities(&output.id)).await {
            Ok(capabilities) if capabilities.writable => Ok(()),
            Ok(_) => Err(Unavailable::ReadOnly),
            Err(error) => Err(Unavailable::Backend(error)),
        };
        listed.push((output, usable));
    }
    Ok(listed)
}

/// One backend's discovery: its outputs with a usable flag each, or its error.
pub type Listed = Result<Vec<(Output, Result<(), Unavailable>)>, BackendError>;

/// Pair two backends' outputs by ID: every display either lists appears
/// once, in the alpha backend's order (it sees every output), then any the
/// hardware backend alone found. The name is the alpha backend's, else the
/// hardware backend's.
#[must_use]
pub fn pair(hardware: &Listed, alpha: &Listed) -> Vec<Found> {
    let endpoint = |listed: &Listed, key: &OutputId| match listed {
        Err(error) => Err(Unavailable::Backend(error.clone())),
        Ok(outputs) => outputs
            .iter()
            .find(|(output, _)| output.id == *key)
            .map_or(Err(Unavailable::NotFound), |(output, usable)| {
                usable.clone().map(|()| output.id.clone())
            }),
    };
    let mut seen: Vec<&Output> = Vec::new();
    for outputs in [alpha, hardware].into_iter().flatten() {
        for (output, _) in outputs {
            if !seen.iter().any(|known| known.id == output.id) {
                seen.push(output);
            }
        }
    }
    seen.into_iter()
        .map(|output| Found {
            key: output.id.0.clone(),
            name: output.name.clone(),
            hardware: endpoint(hardware, &output.id),
            alpha: endpoint(alpha, &output.id),
        })
        .collect()
}

/// Find every display: ask both backends (each call bounded by `timeout`)
/// and pair them. Runs on the executor, never the UI thread.
pub async fn discover<H: Backend, A: Backend>(
    hardware: &H,
    alpha: &A,
    timeout: Duration,
) -> Vec<Found> {
    let (hardware, alpha) = (list(hardware, timeout).await, list(alpha, timeout).await);
    if let Err(error) = &hardware {
        tracing::info!(%error, "hardware backend: no outputs");
    }
    if let Err(error) = &alpha {
        tracing::warn!(%error, "alpha backend: no outputs");
    }
    pair(&hardware, &alpha)
}

#[cfg(test)]
mod tests {
    use dimmer_core::{LevelKind, OutputId, mock::MockBackend};

    use super::*;

    fn found(name: &str, hardware: bool, alpha: bool) -> Found {
        let endpoint = |ok: bool| {
            if ok {
                Ok(OutputId(name.into()))
            } else {
                Err(Unavailable::NotFound)
            }
        };
        Found {
            key: name.into(),
            name: name.into(),
            hardware: endpoint(hardware),
            alpha: endpoint(alpha),
        }
    }

    fn both() -> Monitor {
        Monitor::new(
            found("HDMI-A-1", true, true),
            MonitorLevels::default(),
            Floors::default(),
        )
    }

    fn levels(monitor: &Monitor) -> (u8, u8) {
        (
            monitor.level(Channel::Hardware),
            monitor.level(Channel::Alpha),
        )
    }

    #[test]
    fn writes_are_coalesced_per_channel() {
        let mut state = WriteState::default();
        assert_eq!(state.request(80), Some(80));
        assert_eq!(state.request(70), None, "one write in flight");
        assert_eq!(state.request(60), None);
        assert!(state.in_flight.is_some());
        assert_eq!(state.finished(80, &Ok(())), Some(60), "only the newest");
        assert_eq!(state.confirmed(), Some(80));
        assert_eq!(state.finished(60, &Ok(())), None);
        assert_eq!(state.confirmed(), Some(60));
        assert!(state.in_flight.is_none() && state.queued.is_none());
    }

    #[test]
    fn failures_are_kept_and_cleared_by_success() {
        let mut state = WriteState::default();
        let _ = state.request(50);
        assert_eq!(state.finished(50, &Err(BackendError::TimedOut)), None);
        assert_eq!(state.error(), Some(&BackendError::TimedOut));
        assert_eq!(state.confirmed(), None, "a failed write is never confirmed");
        let _ = state.request(40);
        let _ = state.finished(40, &Err(BackendError::Superseded));
        assert_eq!(
            state.error(),
            Some(&BackendError::TimedOut),
            "superseded is not a failure"
        );
        let _ = state.request(30);
        let _ = state.finished(30, &Ok(()));
        assert_eq!((state.confirmed(), state.error()), (Some(30), None));
    }

    #[test]
    fn slider_moves_write_only_changed_channels() {
        let mut monitor = both();
        let writes = monitor.set(Channel::Alpha, 60);
        assert_eq!(
            writes,
            [Write {
                channel: Channel::Alpha,
                output: OutputId("HDMI-A-1".into()),
                level: 60
            }]
        );
        assert_eq!(monitor.set(Channel::Alpha, 50), [], "coalesced");
        let next = monitor.finished(Channel::Alpha, 60, &Ok(()));
        assert_eq!(next.map(|w| w.level), Some(50));
        monitor.set_locked(true);
        let writes = monitor.set(Channel::Hardware, 80);
        assert_eq!(writes.len(), 1, "alpha is in flight, so only hardware now");
        assert_eq!(levels(&monitor), (80, 30));
    }

    #[test]
    fn sliders_cannot_go_below_their_floors() {
        let mut monitor = both();
        assert_eq!(monitor.minimum(Channel::Hardware), 10);
        assert_eq!(monitor.minimum(Channel::Alpha), 20);
        let _ = monitor.set(Channel::Alpha, 20);
        // 50 × 20 % = 10 %, the effective floor.
        assert_eq!(monitor.minimum(Channel::Hardware), 50);
        let _ = monitor.set(Channel::Hardware, 1);
        assert_eq!(levels(&monitor), (50, 20));
    }

    #[test]
    fn missing_hardware_holds_100_and_cannot_lock() {
        let mut monitor = Monitor::new(
            found("HDMI-A-1", false, true),
            MonitorLevels::clamped(40, 60, true, Floors::default()),
            Floors::default(),
        );
        assert_eq!(levels(&monitor), (100, 60), "saved hardware level ignored");
        assert!(!monitor.levels().locked());
        assert!(!monitor.can_lock());
        monitor.set_locked(true);
        assert!(!monitor.levels().locked());
        assert_eq!(monitor.set(Channel::Hardware, 50), []);
        let writes = monitor.lower(10);
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].channel, Channel::Alpha);
        assert_eq!(levels(&monitor), (100, 50));
        // Down to the alpha floor, never below; hardware stays at 100.
        for _ in 0..20 {
            let _ = monitor.lower(10);
            let _ = monitor.finished(Channel::Alpha, monitor.level(Channel::Alpha), &Ok(()));
        }
        assert_eq!(levels(&monitor), (100, 20));
    }

    #[test]
    fn raise_and_lower_follow_the_levels() {
        let mut monitor = both();
        let _ = monitor.set(Channel::Hardware, 40);
        let _ = monitor.raise(10);
        assert_eq!(levels(&monitor), (50, 100), "raise lifts the lowest");
        let _ = monitor.lower(10);
        assert_eq!(levels(&monitor), (50, 90), "lower drops the highest");
        let mut tie = both();
        let _ = tie.lower(10);
        assert_eq!(levels(&tie), (90, 90), "a tie lowers both");
        let mut locked = both();
        let _ = locked.set(Channel::Alpha, 80);
        locked.set_locked(true);
        let _ = locked.lower(10);
        assert_eq!(levels(&locked), (90, 70), "locked keeps the offset");
    }

    #[test]
    fn force_reset_writes_every_available_channel() {
        let mut monitor = both();
        let writes = monitor.force_reset();
        let summary: Vec<_> = writes.iter().map(|w| (w.channel, w.level)).collect();
        assert_eq!(
            summary,
            [(Channel::Hardware, 100), (Channel::Alpha, 100)],
            "written even though the model was already at 100"
        );
        let mut alpha_only = Monitor::new(
            found("HDMI-A-1", false, true),
            MonitorLevels::clamped(100, 40, false, Floors::default()),
            Floors::default(),
        );
        let writes = alpha_only.force_reset();
        assert_eq!(writes.len(), 1);
        assert_eq!((writes[0].channel, writes[0].level), (Channel::Alpha, 100));
        assert_eq!(levels(&alpha_only), (100, 100));
    }

    #[test]
    fn without_hardware_the_alpha_floor_decides_alone() {
        // A saved effective floor of 30 must not hold alpha above its 10.
        let floors = Floors::clamped(10, 10, 30);
        let mut monitor = Monitor::new(
            found("HDMI-A-1", false, true),
            MonitorLevels::new(floors),
            floors,
        );
        let writes = monitor.set(Channel::Alpha, 10);
        assert_eq!(writes.len(), 1);
        assert_eq!(monitor.level(Channel::Alpha), 10);

        // The same floors on a display with hardware keep the combined floor.
        let mut monitor = Monitor::new(
            found("DP-1", true, true),
            MonitorLevels::new(floors),
            floors,
        );
        let _ = monitor.set(Channel::Alpha, 10);
        assert_eq!(monitor.level(Channel::Alpha), 30);
    }

    #[test]
    fn hardware_arriving_brings_the_combined_floor_back() {
        let floors = Floors::clamped(10, 10, 30);
        let mut monitor = Monitor::new(
            found("DP-1", false, true),
            MonitorLevels::new(floors),
            floors,
        );
        let _ = monitor.set(Channel::Alpha, 10);
        let writes = monitor.rediscovered(found("DP-1", true, true), floors);
        assert_eq!(monitor.level(Channel::Alpha), 30);
        assert!(writes.is_empty(), "the write for 10 % is still in flight");
        let next = monitor.finished(Channel::Alpha, 10, &Ok(()));
        assert_eq!(
            next.map(|write| write.level),
            Some(30),
            "the raised level reaches the backend"
        );
    }

    #[test]
    fn restore_writes_only_dimmed_levels() {
        let mut monitor = Monitor::new(
            found("HDMI-A-1", true, true),
            MonitorLevels::clamped(100, 70, false, Floors::default()),
            Floors::default(),
        );
        let writes = monitor.restore();
        assert_eq!(writes.len(), 1);
        assert_eq!((writes[0].channel, writes[0].level), (Channel::Alpha, 70));
    }

    #[test]
    fn pairing_joins_by_id_and_reports_why_a_channel_is_missing() {
        let output = |id: &str, name: &str, usable: Result<(), Unavailable>| {
            (
                Output {
                    id: OutputId(id.into()),
                    name: name.into(),
                },
                usable,
            )
        };
        let paired = pair(
            &Ok(vec![
                output("TST-00000001-00000001", "ddc name", Ok(())),
                output(
                    "TST-00000001-00000002",
                    "ddc name 2",
                    Err(Unavailable::ReadOnly),
                ),
                output("TST-00000001-00000003", "DDC only", Ok(())),
            ]),
            &Ok(vec![
                output("connector:HDMI-A-1", "HDMI-A-1", Ok(())),
                output("TST-00000001-00000001", "Panel (DP-1)", Ok(())),
                output("TST-00000001-00000002", "Panel (DP-2)", Ok(())),
            ]),
        );
        let names: Vec<_> = paired.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            ["HDMI-A-1", "Panel (DP-1)", "Panel (DP-2)", "DDC only"],
            "alpha backend's order and names, then hardware-only displays"
        );
        assert_eq!(paired[0].hardware, Err(Unavailable::NotFound));
        assert_eq!(paired[0].edid(), None, "connector keys are not saved");
        assert_eq!(
            paired[1].hardware,
            Ok(OutputId("TST-00000001-00000001".into()))
        );
        assert_eq!(paired[1].edid(), Some(EdidId::new("TST", 1, 1).unwrap()));
        assert_eq!(paired[2].hardware, Err(Unavailable::ReadOnly));
        assert_eq!(paired[3].alpha, Err(Unavailable::NotFound));

        let paired = pair(
            &Err(BackendError::Unsupported),
            &Ok(vec![output("connector:HDMI-A-1", "HDMI-A-1", Ok(()))]),
        );
        assert_eq!(
            paired,
            [Found {
                key: "connector:HDMI-A-1".into(),
                name: "HDMI-A-1".into(),
                hardware: Err(Unavailable::Backend(BackendError::Unsupported)),
                alpha: Ok(OutputId("connector:HDMI-A-1".into())),
            }]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn discovery_is_bounded_and_never_fails() {
        let hardware =
            MockBackend::new(["MOCK-1"], LevelKind::Hardware).with_delay(Duration::from_secs(30));
        let alpha = MockBackend::new(["MOCK-1", "MOCK-2"], LevelKind::Software);
        let found = discover(&hardware, &alpha, Duration::from_secs(1)).await;
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|f| f.alpha.is_ok()));
        assert!(
            found
                .iter()
                .all(|f| f.hardware == Err(Unavailable::Backend(BackendError::TimedOut)))
        );
    }

    #[tokio::test]
    async fn mock_backends_pair_as_documented() {
        let backends = crate::backend::Backends::mock();
        let found = discover(
            &*backends.hardware,
            &*backends.alpha,
            Duration::from_secs(1),
        )
        .await;
        let summary: Vec<_> = found
            .iter()
            .map(|f| (f.name.as_str(), f.hardware.is_ok(), f.alpha.is_ok()))
            .collect();
        assert_eq!(summary, [("MOCK-1", true, true), ("MOCK-2", false, true)]);
    }

    #[test]
    fn auto_dimming_follows_one_percent_at_a_time_and_stops_at_floors() {
        let mut monitor = both();
        let target = Target {
            hardware: 97,
            alpha: 5,
        };
        let writes = monitor.follow(target);
        assert_eq!(levels(&monitor), (99, 99));
        assert_eq!(writes.len(), 2);
        for _ in 0..200 {
            let _ = monitor.follow(target);
        }
        let floor = monitor.minimum(Channel::Alpha);
        assert_eq!(levels(&monitor), (97, floor), "the alpha floor holds");
        assert!(monitor.follow(target).is_empty(), "nothing more to write");
    }

    #[test]
    fn auto_dimming_learns_the_current_levels() {
        let mut monitor = both();
        let _ = monitor.set(Channel::Alpha, 40);
        monitor.learn(Phase::SUNSET);
        let target = monitor.profile().target(Phase::SUNRISE).unwrap();
        assert_eq!((target.hardware, target.alpha), (100, 40));
    }

    #[test]
    fn a_locked_display_follows_with_the_software_level_leading() {
        let mut monitor = both();
        monitor.set_locked(true);
        let _ = monitor.follow(Target {
            hardware: 100,
            alpha: 50,
        });
        let (hardware, alpha) = levels(&monitor);
        assert_eq!(alpha, 99);
        assert!(hardware < 100, "the lock moved hardware too");
    }
}
