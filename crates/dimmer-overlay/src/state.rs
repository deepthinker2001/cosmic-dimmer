//! The overlay's lifecycle as a pure state machine: Wayland and API events in,
//! [`Action`]s out. No I/O, so every lifecycle rule is unit-tested offline;
//! the worker thread only executes the actions with the protocol calls the
//! spike proved on cosmic-comp.
//!
//! Rules it enforces:
//!
//! - A surface exists only while its output's retained level is below 100:
//!   at full brightness it is destroyed, not left transparent, so the default
//!   costs the compositor nothing.
//! - Levels are kept per [`OutputId`] (the EDID identity), not per
//!   `wl_output`, so an unplugged-and-replugged monitor, a monitor that comes
//!   back after sleep, or a reconnected compositor gets its dimming back.
//! - No level below [`ALPHA_FLOOR`] is ever drawn, whatever the caller asks.
//! - A compositor `closed` is respected: the surface is dropped and only an
//!   explicit new level from the user recreates it, at most
//!   [`MAX_RECREATES_AFTER_CLOSE`] times per output, never in a loop.
//! - A request is answered `Ok` only once its pixels are committed (or the
//!   surface is destroyed); a request overtaken by a newer level for the same
//!   output before its first draw is answered `Superseded`.

use std::collections::HashMap;

use dimmer_core::{BackendError, Floors, Level, OutputId};

/// A `wl_output`'s global name: unique per connection, never reused for
/// another output while the connection lives.
pub type Handle = u32;

/// The lowest retained level this layer will ever draw: dimmer-core's hard
/// minimum for the alpha slider (with that at 10 %, the overlay is at most
/// 90 % opaque). dimmer-core already keeps levels at or above its floors; this
/// is the last line of defence if a caller bypasses it (R12).
pub const ALPHA_FLOOR: u8 = Floors::HARD_MINIMUM.alpha();

/// Outputs tracked at once; more are left undimmed (the worker logs it).
pub const MAX_OUTPUTS: usize = 16;
/// Largest configure dimension accepted (logical pixels).
pub const MAX_DIMENSION: u32 = 16_384;
/// Live buffer memory across all outputs.
pub const MAX_TOTAL_BYTES: usize = 512 * 1024 * 1024;
/// How often a user's new level may recreate a surface the compositor closed.
pub const MAX_RECREATES_AFTER_CLOSE: u8 = 3;

/// What the worker must do, in order.
#[derive(Debug, PartialEq, Eq)]
pub enum Action<T> {
    /// Create the layer surface for this output (empty input region, overlay
    /// layer, all anchors, size 0×0, exclusive zone -1, no keyboard), commit
    /// with no buffer and wait for configure.
    Create(Handle),
    /// Fill a new `width × height` ARGB8888 buffer with black at this
    /// premultiplied alpha byte, damage, attach and commit.
    Draw {
        output: Handle,
        size: (u32, u32),
        alpha: u8,
    },
    /// Commit with no change (applies a configure that kept the size).
    Commit(Handle),
    /// Destroy the layer surface and its buffer.
    Destroy(Handle),
    /// Answer a caller.
    Reply(T, Result<(), BackendError>),
}

/// The retained level actually applied: never below [`ALPHA_FLOOR`].
#[must_use]
pub fn floored(level: Level) -> Level {
    if level.percent() < ALPHA_FLOOR {
        Level::new(ALPHA_FLOOR).unwrap_or(Level::FULL)
    } else {
        level
    }
}

/// Premultiplied ARGB alpha byte for black at a retained level: opacity is
/// `1 - level / 100`, rounded to the nearest byte, floor applied.
#[must_use]
pub fn alpha_byte(level: Level) -> u8 {
    let opacity = 100 - u32::from(floored(level).percent());
    u8::try_from((opacity * 255 + 50) / 100).unwrap_or(u8::MAX)
}

/// One native-endian `wl_shm` ARGB8888 pixel: black, so the colour channels
/// of the premultiplied value are zero and only alpha is set.
#[must_use]
pub fn black_pixel(alpha: u8) -> [u8; 4] {
    (u32::from(alpha) << 24).to_ne_bytes()
}

/// Bytes for one buffer of that size, or `None` when it is empty, oversized
/// or would overflow.
#[must_use]
pub fn buffer_bytes(width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return None;
    }
    usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?
        .checked_mul(4)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Surface {
    /// No surface: full brightness, or dropped after `closed`.
    Absent,
    /// Created, waiting for the first configure.
    Awaiting,
    /// Drawn at this size and alpha byte.
    Shown { size: (u32, u32), alpha: u8 },
}

#[derive(Debug)]
struct Entry {
    handle: Handle,
    id: OutputId,
    name: String,
    surface: Surface,
    /// Times the compositor closed (or a bad configure ended) this output's
    /// surface while the output stayed.
    closes: u8,
}

impl Entry {
    fn usable(&self) -> bool {
        self.closes <= MAX_RECREATES_AFTER_CLOSE
    }

    fn bytes(&self) -> usize {
        match self.surface {
            Surface::Shown { size, .. } => buffer_bytes(size.0, size.1).unwrap_or(0),
            _ => 0,
        }
    }
}

#[derive(Debug)]
struct Pending<T> {
    ticket: T,
    id: OutputId,
    waiting: Vec<Handle>,
}

/// What an output looks like to callers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputView {
    pub id: OutputId,
    pub name: String,
    /// False once the compositor has closed its surface too often.
    pub writable: bool,
    /// The retained level now applied (after the floor), `FULL` if never set.
    pub level: Level,
}

/// The overlay state for one session; `T` is the caller's reply ticket.
#[derive(Debug)]
pub struct Overlay<T> {
    entries: Vec<Entry>,
    levels: HashMap<OutputId, Level>,
    pending: Vec<Pending<T>>,
}

impl<T> Default for Overlay<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            levels: HashMap::new(),
            pending: Vec::new(),
        }
    }
}

fn unavailable<T>(reason: &str) -> Result<T, BackendError> {
    Err(BackendError::Unavailable(reason.into()))
}

impl<T> Overlay<T> {
    /// The retained level for an identity: the last one applied, else full.
    #[must_use]
    pub fn level(&self, id: &OutputId) -> Level {
        self.levels.get(id).copied().unwrap_or(Level::FULL)
    }

    /// Outputs present now, one per identity (two identical monitors that
    /// share an EDID identity share one entry, as they share saved settings).
    #[must_use]
    pub fn outputs(&self) -> Vec<OutputView> {
        let mut views: Vec<OutputView> = Vec::new();
        for entry in &self.entries {
            if let Some(view) = views.iter_mut().find(|view| view.id == entry.id) {
                view.writable |= entry.usable();
                continue;
            }
            views.push(OutputView {
                id: entry.id.clone(),
                name: entry.name.clone(),
                writable: entry.usable(),
                level: self.level(&entry.id),
            });
        }
        views
    }

    /// How many surfaces exist or are being created.
    #[must_use]
    pub fn surfaces(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.surface != Surface::Absent)
            .count()
    }

    /// A new `wl_output`, identified. Returns `None` (and tracks nothing) if
    /// [`MAX_OUTPUTS`] are already tracked or the handle is already known.
    pub fn output_added(
        &mut self,
        handle: Handle,
        id: OutputId,
        name: String,
    ) -> Option<Vec<Action<T>>> {
        if self.entries.len() >= MAX_OUTPUTS || self.index(handle).is_some() {
            return None;
        }
        let dimmed = self.level(&id) < Level::FULL;
        self.entries.push(Entry {
            handle,
            id,
            name,
            surface: if dimmed {
                Surface::Awaiting
            } else {
                Surface::Absent
            },
            closes: 0,
        });
        Some(if dimmed {
            vec![Action::Create(handle)]
        } else {
            Vec::new()
        })
    }

    /// The identity of a known output changed (for example its EDID became
    /// readable after it was first announced): its surface now follows the
    /// level retained for the new identity.
    pub fn output_identified(&mut self, handle: Handle, id: OutputId) -> Vec<Action<T>> {
        let Some(index) = self.index(handle) else {
            return Vec::new();
        };
        if self.entries[index].id == id {
            return Vec::new();
        }
        let name = self.entries[index].name.clone();
        let mut actions = self.output_removed(handle);
        actions.extend(self.output_added(handle, id, name).unwrap_or_default());
        actions
    }

    /// A `wl_output` went away (unplug, or a monitor dropped during sleep).
    /// Its level stays retained under its identity for when it returns.
    pub fn output_removed(&mut self, handle: Handle) -> Vec<Action<T>> {
        let Some(index) = self.index(handle) else {
            return Vec::new();
        };
        let entry = self.entries.remove(index);
        let mut actions = Vec::new();
        if entry.surface != Surface::Absent {
            actions.push(Action::Destroy(handle));
        }
        self.fail_waiting_on(handle, "the output was removed", &mut actions);
        actions
    }

    /// The compositor sent `closed` for this output's surface. Respect it:
    /// drop the surface; do not recreate it until the user sets a new level.
    pub fn closed(&mut self, handle: Handle) -> Vec<Action<T>> {
        let Some(index) = self.index(handle) else {
            return Vec::new();
        };
        if self.entries[index].surface == Surface::Absent {
            let mut actions = Vec::new();
            self.fail_waiting_on(handle, "the compositor closed the overlay", &mut actions);
            return actions;
        }
        self.drop_surface(index, "the compositor closed the overlay")
    }

    /// Drop the surface at `index` as the compositor's doing: destroyed,
    /// counted towards giving up on the output, and every write waiting on
    /// it failed with `reason`.
    fn drop_surface(&mut self, index: usize, reason: &str) -> Vec<Action<T>> {
        let entry = &mut self.entries[index];
        let handle = entry.handle;
        entry.surface = Surface::Absent;
        entry.closes = entry.closes.saturating_add(1);
        let mut actions = vec![Action::Destroy(handle)];
        self.fail_waiting_on(handle, reason, &mut actions);
        actions
    }

    /// A configure for this output's surface (SCTK has acknowledged it).
    pub fn configure(&mut self, handle: Handle, width: u32, height: u32) -> Vec<Action<T>> {
        let Some(index) = self.index(handle) else {
            return Vec::new();
        };
        let level = self.level(&self.entries[index].id);
        let entry = &self.entries[index];
        let mut actions = Vec::new();
        if entry.surface == Surface::Absent {
            return actions; // Stale: the surface was already destroyed.
        }
        if level == Level::FULL {
            // Cannot happen through set_level (it destroys at full); be safe.
            self.entries[index].surface = Surface::Absent;
            actions.push(Action::Destroy(handle));
            self.resolve_waiting_on(handle, &mut actions);
            return actions;
        }
        let others: usize = self
            .entries
            .iter()
            .filter(|other| other.handle != handle)
            .map(Entry::bytes)
            .sum();
        let fits = buffer_bytes(width, height)
            .and_then(|bytes| bytes.checked_add(others))
            .is_some_and(|total| total <= MAX_TOTAL_BYTES);
        if !fits {
            // Treated like a close: dropped, counted, never drawn.
            return self.drop_surface(index, "the compositor asked for an invalid size");
        }
        let alpha = alpha_byte(level);
        let size = (width, height);
        let entry = &mut self.entries[index];
        if entry.surface == (Surface::Shown { size, alpha }) {
            actions.push(Action::Commit(handle));
        } else {
            entry.surface = Surface::Shown { size, alpha };
            actions.push(Action::Draw {
                output: handle,
                size,
                alpha,
            });
        }
        self.resolve_waiting_on(handle, &mut actions);
        actions
    }

    /// The caller's new retained level for an identity. The floor is applied
    /// first; at full brightness every surface for it is destroyed.
    pub fn set_level(&mut self, id: &OutputId, level: Level, ticket: T) -> Vec<Action<T>> {
        let mut actions = Vec::new();
        if !self.entries.iter().any(|entry| &entry.id == id) {
            actions.push(Action::Reply(ticket, Err(BackendError::UnknownOutput)));
            return actions;
        }
        let level = floored(level);
        // An older request for this identity still waiting for its first draw
        // is overtaken: its level will never be shown.
        let (overtaken, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|pending| &pending.id == id);
        self.pending = kept;
        actions.extend(
            overtaken
                .into_iter()
                .map(|pending| Action::Reply(pending.ticket, Err(BackendError::Superseded))),
        );
        self.levels.insert(id.clone(), level);

        let alpha = alpha_byte(level);
        let mut waiting = Vec::new();
        let mut refused = false;
        for entry in self.entries.iter_mut().filter(|entry| &entry.id == id) {
            let handle = entry.handle;
            match (level == Level::FULL, entry.surface) {
                (true, Surface::Absent) => {}
                (true, _) => {
                    entry.surface = Surface::Absent;
                    actions.push(Action::Destroy(handle));
                }
                (false, Surface::Absent) if !entry.usable() => refused = true,
                (false, Surface::Absent) => {
                    entry.surface = Surface::Awaiting;
                    actions.push(Action::Create(handle));
                    waiting.push(handle);
                }
                (false, Surface::Awaiting) => waiting.push(handle),
                (false, Surface::Shown { alpha: shown, .. }) if shown == alpha => {}
                (false, Surface::Shown { size, .. }) => {
                    entry.surface = Surface::Shown { size, alpha };
                    actions.push(Action::Draw {
                        output: handle,
                        size,
                        alpha,
                    });
                }
            }
        }
        if refused && waiting.is_empty() {
            actions.push(Action::Reply(
                ticket,
                unavailable("the compositor closed this output's overlay too often"),
            ));
        } else if waiting.is_empty() {
            actions.push(Action::Reply(ticket, Ok(())));
        } else {
            self.pending.push(Pending {
                ticket,
                id: id.clone(),
                waiting,
            });
        }
        actions
    }

    /// The compositor connection is gone: every surface with it. Levels are
    /// kept, so a new connection restores the dimming; callers still waiting
    /// are told their level was not applied.
    pub fn disconnected(&mut self) -> Vec<Action<T>> {
        self.entries.clear();
        std::mem::take(&mut self.pending)
            .into_iter()
            .map(|pending| {
                Action::Reply(
                    pending.ticket,
                    unavailable("the compositor connection was lost"),
                )
            })
            .collect()
    }

    fn index(&self, handle: Handle) -> Option<usize> {
        self.entries.iter().position(|entry| entry.handle == handle)
    }

    fn resolve_waiting_on(&mut self, handle: Handle, actions: &mut Vec<Action<T>>) {
        for pending in &mut self.pending {
            pending.waiting.retain(|waiting| *waiting != handle);
        }
        let (done, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|pending| pending.waiting.is_empty());
        self.pending = kept;
        actions.extend(
            done.into_iter()
                .map(|pending| Action::Reply(pending.ticket, Ok(()))),
        );
    }

    fn fail_waiting_on(&mut self, handle: Handle, reason: &str, actions: &mut Vec<Action<T>>) {
        let (failed, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|pending| pending.waiting.contains(&handle));
        self.pending = kept;
        actions.extend(
            failed
                .into_iter()
                .map(|pending| Action::Reply(pending.ticket, unavailable(reason))),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Act = Action<u32>;

    fn id(name: &str) -> OutputId {
        OutputId(name.into())
    }

    fn level(percent: u8) -> Level {
        Level::new(percent).unwrap()
    }

    fn with_output(handle: Handle, name: &str) -> Overlay<u32> {
        let mut overlay = Overlay::default();
        assert_eq!(
            overlay.output_added(handle, id(name), name.into()),
            Some(vec![])
        );
        overlay
    }

    #[test]
    fn full_brightness_by_default_means_no_surface() {
        let overlay = with_output(1, "TV");
        assert_eq!(overlay.surfaces(), 0);
        assert_eq!(overlay.level(&id("TV")), Level::FULL);
        assert_eq!(
            overlay.outputs(),
            vec![OutputView {
                id: id("TV"),
                name: "TV".into(),
                writable: true,
                level: Level::FULL
            }]
        );
    }

    #[test]
    fn dimming_creates_draws_on_configure_and_only_then_replies() {
        let mut overlay = with_output(1, "TV");
        assert_eq!(
            overlay.set_level(&id("TV"), level(50), 7),
            vec![Act::Create(1)]
        );
        assert_eq!(overlay.surfaces(), 1);
        assert_eq!(
            overlay.configure(1, 1920, 1080),
            vec![
                Act::Draw {
                    output: 1,
                    size: (1920, 1080),
                    alpha: 128
                },
                Act::Reply(7, Ok(()))
            ]
        );
    }

    #[test]
    fn full_brightness_destroys_rather_than_making_transparent() {
        let mut overlay = with_output(1, "TV");
        overlay.set_level(&id("TV"), level(50), 1);
        overlay.configure(1, 1920, 1080);
        assert_eq!(
            overlay.set_level(&id("TV"), Level::FULL, 2),
            vec![Act::Destroy(1), Act::Reply(2, Ok(()))]
        );
        assert_eq!(overlay.surfaces(), 0);
        // A late configure for the destroyed surface does nothing.
        assert_eq!(overlay.configure(1, 1920, 1080), vec![]);
        // And full again is a no-op that still succeeds.
        assert_eq!(
            overlay.set_level(&id("TV"), Level::FULL, 3),
            vec![Act::Reply(3, Ok(()))]
        );
    }

    #[test]
    fn level_change_on_a_shown_surface_redraws_at_the_same_size() {
        let mut overlay = with_output(1, "TV");
        overlay.set_level(&id("TV"), level(50), 1);
        overlay.configure(1, 1920, 1080);
        assert_eq!(
            overlay.set_level(&id("TV"), level(80), 2),
            vec![
                Act::Draw {
                    output: 1,
                    size: (1920, 1080),
                    alpha: 51
                },
                Act::Reply(2, Ok(()))
            ]
        );
        // Same level again draws nothing.
        assert_eq!(
            overlay.set_level(&id("TV"), level(80), 3),
            vec![Act::Reply(3, Ok(()))]
        );
    }

    #[test]
    fn resize_redraws_and_same_size_configure_only_commits() {
        let mut overlay = with_output(1, "TV");
        overlay.set_level(&id("TV"), level(50), 1);
        overlay.configure(1, 1920, 1080);
        assert_eq!(overlay.configure(1, 1920, 1080), vec![Act::Commit(1)]);
        assert_eq!(
            overlay.configure(1, 2560, 1440),
            vec![Act::Draw {
                output: 1,
                size: (2560, 1440),
                alpha: 128
            }]
        );
    }

    #[test]
    fn floor_is_applied_at_this_layer() {
        let mut overlay = with_output(1, "TV");
        overlay.set_level(&id("TV"), level(1), 1);
        assert_eq!(overlay.level(&id("TV")), level(ALPHA_FLOOR));
        let actions = overlay.configure(1, 100, 100);
        let Act::Draw { alpha, .. } = actions[0] else {
            panic!("expected a draw, got {actions:?}");
        };
        assert_eq!(alpha, alpha_byte(level(ALPHA_FLOOR)));
        for percent in 1..=100 {
            assert!(alpha_byte(level(percent)) <= alpha_byte(level(ALPHA_FLOOR)));
        }
    }

    #[test]
    fn alpha_bytes_and_pixels() {
        assert_eq!(ALPHA_FLOOR, 10);
        assert_eq!(alpha_byte(Level::FULL), 0);
        assert_eq!(alpha_byte(level(50)), 128);
        assert_eq!(alpha_byte(level(90)), 26);
        assert_eq!(alpha_byte(level(10)), 230);
        assert_eq!(alpha_byte(level(1)), 230);
        assert_eq!(black_pixel(0), 0_u32.to_ne_bytes());
        assert_eq!(black_pixel(0x80), 0x8000_0000_u32.to_ne_bytes());
    }

    #[test]
    fn checked_buffer_sizes() {
        assert_eq!(buffer_bytes(1920, 1080), Some(8_294_400));
        assert_eq!(buffer_bytes(MAX_DIMENSION, 1), Some(65_536));
        for (width, height) in [(0, 1), (1, 0), (u32::MAX, 2), (MAX_DIMENSION + 1, 1)] {
            assert_eq!(buffer_bytes(width, height), None);
        }
    }

    #[test]
    fn invalid_configure_is_dropped_not_drawn() {
        let mut overlay = with_output(1, "TV");
        overlay.set_level(&id("TV"), level(50), 1);
        assert_eq!(
            overlay.configure(1, 0, 1080),
            vec![
                Act::Destroy(1),
                Act::Reply(1, unavailable("the compositor asked for an invalid size"))
            ]
        );
        assert_eq!(overlay.surfaces(), 0);
    }

    #[test]
    fn total_buffer_memory_is_bounded() {
        let mut overlay = Overlay::default();
        for handle in 0..3 {
            overlay.output_added(handle, id(&format!("M{handle}")), String::new());
            overlay.set_level(&id(&format!("M{handle}")), level(50), handle);
        }
        // 16384² × 4 = 1 GiB alone exceeds the bound.
        assert!(matches!(
            overlay.configure(0, MAX_DIMENSION, MAX_DIMENSION)[0],
            Act::Destroy(0)
        ));
        // Two 8192×8192 (256 MiB each) fit exactly; a third does not.
        assert!(matches!(
            overlay.configure(1, 8192, 8192)[0],
            Act::Draw { .. }
        ));
        assert!(matches!(
            overlay.configure(2, 8192, 8192)[0],
            Act::Draw { .. }
        ));
        overlay.set_level(&id("M0"), level(40), 9);
        assert!(matches!(overlay.configure(0, 1, 1)[0], Act::Destroy(0)));
    }

    #[test]
    fn hotplug_keeps_the_level_and_recreates() {
        let mut overlay = with_output(1, "TV");
        overlay.set_level(&id("TV"), level(40), 1);
        overlay.configure(1, 1920, 1080);
        assert_eq!(overlay.output_removed(1), vec![Act::Destroy(1)]);
        assert_eq!(overlay.outputs(), Vec::new());
        // The same monitor comes back as a new wl_output global.
        assert_eq!(
            overlay.output_added(5, id("TV"), "TV".into()),
            Some(vec![Act::Create(5)])
        );
        assert_eq!(
            overlay.configure(5, 1920, 1080),
            vec![Act::Draw {
                output: 5,
                size: (1920, 1080),
                alpha: 153
            }]
        );
        // A different monitor on the same connector starts undimmed.
        assert_eq!(
            overlay.output_added(6, id("OTHER"), "OTHER".into()),
            Some(vec![])
        );
    }

    #[test]
    fn removal_fails_a_waiting_request_and_unknown_outputs_are_errors() {
        let mut overlay = with_output(1, "TV");
        overlay.set_level(&id("TV"), level(40), 1);
        assert_eq!(
            overlay.output_removed(1),
            vec![
                Act::Destroy(1),
                Act::Reply(1, unavailable("the output was removed"))
            ]
        );
        assert_eq!(
            overlay.set_level(&id("TV"), level(40), 2),
            vec![Act::Reply(2, Err(BackendError::UnknownOutput))]
        );
        assert_eq!(overlay.output_removed(1), vec![]);
        assert_eq!(overlay.configure(1, 10, 10), vec![]);
        assert_eq!(overlay.closed(1), vec![]);
    }

    #[test]
    fn disconnect_keeps_levels_for_the_next_connection() {
        let mut overlay = with_output(1, "TV");
        overlay.set_level(&id("TV"), level(60), 1);
        assert_eq!(
            overlay.disconnected(),
            vec![Act::Reply(
                1,
                unavailable("the compositor connection was lost")
            )]
        );
        assert_eq!(overlay.surfaces(), 0);
        assert_eq!(
            overlay.output_added(1, id("TV"), "TV".into()),
            Some(vec![Act::Create(1)])
        );
    }

    #[test]
    fn closed_is_respected_and_retried_only_on_user_intent_boundedly() {
        let mut overlay = with_output(1, "TV");
        overlay.set_level(&id("TV"), level(50), 0);
        overlay.configure(1, 100, 100);
        for attempt in 1..=MAX_RECREATES_AFTER_CLOSE {
            assert_eq!(overlay.closed(1), vec![Act::Destroy(1)]);
            assert_eq!(overlay.surfaces(), 0, "never recreated by itself");
            let ticket = u32::from(attempt);
            assert_eq!(
                overlay.set_level(&id("TV"), level(50), ticket),
                vec![Act::Create(1)]
            );
            overlay.configure(1, 100, 100);
        }
        assert_eq!(overlay.closed(1), vec![Act::Destroy(1)]);
        assert!(!overlay.outputs()[0].writable);
        assert_eq!(
            overlay.set_level(&id("TV"), level(50), 99),
            vec![Act::Reply(
                99,
                unavailable("the compositor closed this output's overlay too often")
            )]
        );
        // Full brightness still succeeds: there is nothing to show.
        assert_eq!(
            overlay.set_level(&id("TV"), Level::FULL, 100),
            vec![Act::Reply(100, Ok(()))]
        );
    }

    #[test]
    fn closed_while_waiting_fails_the_request() {
        let mut overlay = with_output(1, "TV");
        overlay.set_level(&id("TV"), level(50), 1);
        assert_eq!(
            overlay.closed(1),
            vec![
                Act::Destroy(1),
                Act::Reply(1, unavailable("the compositor closed the overlay"))
            ]
        );
    }

    #[test]
    fn a_newer_level_supersedes_one_still_waiting() {
        let mut overlay = with_output(1, "TV");
        assert_eq!(
            overlay.set_level(&id("TV"), level(50), 1),
            vec![Act::Create(1)]
        );
        assert_eq!(
            overlay.set_level(&id("TV"), level(30), 2),
            vec![Act::Reply(1, Err(BackendError::Superseded))]
        );
        assert_eq!(
            overlay.configure(1, 10, 10),
            vec![
                Act::Draw {
                    output: 1,
                    size: (10, 10),
                    alpha: 179
                },
                Act::Reply(2, Ok(()))
            ]
        );
    }

    #[test]
    fn outputs_are_independent() {
        let mut overlay = with_output(1, "LEFT");
        overlay.output_added(2, id("RIGHT"), "RIGHT".into());
        assert_eq!(
            overlay.set_level(&id("LEFT"), level(50), 1),
            vec![Act::Create(1)]
        );
        assert_eq!(overlay.level(&id("RIGHT")), Level::FULL);
        assert_eq!(overlay.surfaces(), 1);
    }

    #[test]
    fn identical_monitors_share_an_identity_and_a_reply_waits_for_both() {
        let mut overlay = with_output(1, "TWIN");
        overlay.output_added(2, id("TWIN"), "TWIN".into());
        assert_eq!(overlay.outputs().len(), 1);
        assert_eq!(
            overlay.set_level(&id("TWIN"), level(50), 1),
            vec![Act::Create(1), Act::Create(2)]
        );
        assert_eq!(overlay.configure(1, 10, 10).len(), 1, "draw, no reply yet");
        assert_eq!(overlay.configure(2, 10, 10)[1], Act::Reply(1, Ok(())));
    }

    #[test]
    fn late_identification_follows_the_new_identity() {
        // The TV was dimmed earlier, then came back before its EDID was readable.
        let mut overlay = with_output(9, "TV");
        overlay.set_level(&id("TV"), level(50), 1);
        overlay.output_removed(9);
        overlay.output_added(1, id("connector:HDMI-A-1"), "HDMI-A-1".into());
        assert_eq!(overlay.surfaces(), 0, "unknown identity: undimmed");
        assert_eq!(overlay.output_identified(1, id("TV")), vec![Act::Create(1)]);
        assert_eq!(overlay.outputs()[0].name, "HDMI-A-1");
        assert_eq!(overlay.output_identified(1, id("TV")), vec![]);
        assert_eq!(overlay.output_identified(42, id("TV")), vec![]);
    }

    #[test]
    fn output_count_is_bounded() {
        let mut overlay = Overlay::<u32>::default();
        for handle in 0..u32::try_from(MAX_OUTPUTS).unwrap() {
            assert!(
                overlay
                    .output_added(handle, id(&handle.to_string()), String::new())
                    .is_some()
            );
        }
        assert!(overlay.output_added(999, id("x"), String::new()).is_none());
        assert!(overlay.output_added(0, id("dup"), String::new()).is_none());
    }
}
