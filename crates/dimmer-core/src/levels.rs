//! The per-monitor dimming policy (R11, R12): a hardware level and an alpha
//! level, an optional lock between them, and floors that keep the screen
//! readable. Pure state; applying levels to backends is the caller's job.
//!
//! Both levels are retained brightness in percent, as [`Level`] is: hardware
//! 100 is the display's full backlight, alpha 100 is no overlay at all. The
//! *effective* brightness is `hardware × alpha / 100`. That product is only a
//! proxy: a DDC brightness percentage is not linear in luminance, and how dark
//! a given display is at a given hardware percent is not known until measured.
//!
//! # Invariant
//!
//! Every [`MonitorLevels`] value, however it was made (defaults, a config file,
//! any sequence of operations), satisfies all of:
//!
//! - `floors.hardware <= hardware <= 100`
//! - `floors.alpha <= alpha <= 100`
//! - `hardware × alpha >= floors.effective × 100`
//!
//! and every floor is at least its [`Floors::HARD_MINIMUM`]. There is no way to
//! construct a value that breaks this, so a UI bug or a bad file cannot ask a
//! backend for a black screen through this type.

use crate::Level;

/// Lower bounds for one monitor's levels, in percent.
///
/// The defaults are **provisional**: the DDC spike has not yet measured how
/// dark the Frame TV (or any display) gets at low hardware levels, so they are
/// deliberately conservative guesses, not measurements. Replace
/// [`Floors::PROVISIONAL_DEFAULT`] (and reconsider [`Floors::HARD_MINIMUM`])
/// once a safe hardware floor has been measured.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Floors {
    hardware: u8,
    alpha: u8,
    effective: u8,
}

impl Floors {
    /// The lowest each floor may be configured to. Enforced here, not only in
    /// the UI: a config file or caller asking for less gets this.
    ///
    /// Provisional (unmeasured): hardware 5 %, alpha 10 % (the overlay is at
    /// most 90 % opaque), effective 5 % of full.
    pub const HARD_MINIMUM: Self = Self {
        hardware: 5,
        alpha: 10,
        effective: 5,
    };

    /// Floors for a monitor with no saved settings.
    ///
    /// Provisional (unmeasured): hardware 10 %, alpha 20 %, effective 10 % of
    /// full. With these, the effective floor is the one that binds when both
    /// sliders are low (10 × 20 / 100 = 2 % < 10 %).
    pub const PROVISIONAL_DEFAULT: Self = Self {
        hardware: 10,
        alpha: 20,
        effective: 10,
    };

    /// Floors from percentages, each raised to its hard minimum and capped at
    /// 100. Never fails: out-of-range input is clamped, not trusted.
    #[must_use]
    pub fn clamped(hardware: u8, alpha: u8, effective: u8) -> Self {
        Self {
            hardware: hardware.clamp(Self::HARD_MINIMUM.hardware, 100),
            alpha: alpha.clamp(Self::HARD_MINIMUM.alpha, 100),
            effective: effective.clamp(Self::HARD_MINIMUM.effective, 100),
        }
    }

    /// The same floors for a display with no hardware level. Its combined
    /// brightness is just its alpha level, so the alpha floor decides alone
    /// and an effective floor the user cannot see never binds.
    #[must_use]
    pub const fn software_only(self) -> Self {
        Self {
            effective: self.alpha,
            ..self
        }
    }

    #[must_use]
    pub const fn hardware(self) -> u8 {
        self.hardware
    }

    #[must_use]
    pub const fn alpha(self) -> u8 {
        self.alpha
    }

    #[must_use]
    pub const fn effective(self) -> u8 {
        self.effective
    }
}

impl Default for Floors {
    fn default() -> Self {
        Self::PROVISIONAL_DEFAULT
    }
}

/// Which levels an operation changed, so the caller writes only those
/// backends. A level that did not change needs no write.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[must_use = "the changed levels must be applied to their backends"]
pub struct Change {
    pub hardware: bool,
    pub alpha: bool,
}

impl Change {
    #[must_use]
    pub const fn any(self) -> bool {
        self.hardware || self.alpha
    }
}

/// One monitor's levels, lock and floors. See the module docs for the
/// invariant every value keeps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MonitorLevels {
    hardware: u8,
    alpha: u8,
    locked: bool,
    floors: Floors,
}

impl Default for MonitorLevels {
    fn default() -> Self {
        Self::new(Floors::default())
    }
}

impl MonitorLevels {
    /// Both levels at full, unlocked.
    #[must_use]
    pub const fn new(floors: Floors) -> Self {
        Self {
            hardware: 100,
            alpha: 100,
            locked: false,
            floors,
        }
    }

    /// Levels from untrusted values (a config file, a previous session),
    /// brought within the floors: each level is clamped to its floor and 100,
    /// then, while the effective brightness is below its floor, the lower level
    /// is raised, exactly as [`Self::raise`] would.
    #[must_use]
    pub fn clamped(hardware: u8, alpha: u8, locked: bool, floors: Floors) -> Self {
        let mut levels = Self {
            hardware,
            alpha,
            locked,
            floors,
        };
        levels.normalize();
        levels
    }

    #[must_use]
    pub const fn hardware(&self) -> Level {
        Level(self.hardware)
    }

    #[must_use]
    pub const fn alpha(&self) -> Level {
        Level(self.alpha)
    }

    #[must_use]
    pub const fn locked(&self) -> bool {
        self.locked
    }

    #[must_use]
    pub const fn floors(&self) -> Floors {
        self.floors
    }

    /// `hardware × alpha / 100`, rounded down, in percent of full.
    #[must_use]
    pub fn effective_percent(&self) -> u8 {
        u8::try_from(self.product() / 100).unwrap_or(100)
    }

    /// The hardware level minus the alpha level: what the lock preserves.
    #[must_use]
    pub fn offset(&self) -> i16 {
        i16::from(self.hardware) - i16::from(self.alpha)
    }

    /// Lock or unlock. Locking keeps the current offset between the levels;
    /// unlocking and relocking later keeps whatever offset they have then.
    pub fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
    }

    /// Move the hardware slider toward `percent`.
    ///
    /// Unlocked, only hardware moves, stopping at its floor, at 100, and where
    /// the effective floor would be crossed. Locked, alpha moves by the same
    /// amount, and the move stops where *either* level would leave its range,
    /// so the offset is kept exactly.
    pub fn set_hardware(&mut self, percent: u8) -> Change {
        if self.locked {
            self.move_locked(i16::from(percent) - i16::from(self.hardware))
        } else {
            let lowest = self.floors.hardware.max(self.min_for_effective(self.alpha));
            let target = percent.clamp(lowest, 100);
            let changed = target != self.hardware;
            self.hardware = target;
            Change {
                hardware: changed,
                alpha: false,
            }
        }
    }

    /// Move the alpha slider toward `percent`; the mirror of
    /// [`Self::set_hardware`].
    pub fn set_alpha(&mut self, percent: u8) -> Change {
        if self.locked {
            self.move_locked(i16::from(percent) - i16::from(self.alpha))
        } else {
            let lowest = self.floors.alpha.max(self.min_for_effective(self.hardware));
            let target = percent.clamp(lowest, 100);
            let changed = target != self.alpha;
            self.alpha = target;
            Change {
                hardware: false,
                alpha: changed,
            }
        }
    }

    /// Replace the floors (each clamped to its hard minimum when it was made),
    /// raising the levels if they are now below them.
    pub fn set_floors(&mut self, floors: Floors) -> Change {
        let before = *self;
        self.floors = floors;
        self.normalize();
        self.change_from(before)
    }

    /// Brighten for a brightness-up key or recovery: always increases the
    /// lowest level by `step` (at least 1), whatever the sliders and lock say.
    ///
    /// Unlocked, the lower level rises (both on a tie). Locked, both rise by
    /// `step`, each stopping at 100; once the higher one is at 100 the offset
    /// shrinks rather than the raise being refused. Changes nothing only when
    /// both levels are already at 100.
    pub fn raise(&mut self, step: u8) -> Change {
        let step = step.max(1);
        let before = *self;
        let up = |level: u8| level.saturating_add(step).min(100);
        if self.locked || self.hardware == self.alpha {
            self.hardware = up(self.hardware);
            self.alpha = up(self.alpha);
        } else if self.hardware < self.alpha {
            self.hardware = up(self.hardware);
        } else {
            self.alpha = up(self.alpha);
        }
        self.change_from(before)
    }

    fn product(self) -> u16 {
        u16::from(self.hardware) * u16::from(self.alpha)
    }

    fn meets_effective(hardware: i16, alpha: i16, effective: u8) -> bool {
        hardware * alpha >= i16::from(effective) * 100
    }

    /// The lowest value for one level, given the other, that keeps the
    /// effective floor. At most 100 whenever `other` comes from a valid state.
    fn min_for_effective(self, other: u8) -> u8 {
        let needed = u16::from(self.floors.effective) * 100;
        let other = u16::from(other.max(1));
        u8::try_from(needed.div_ceil(other)).unwrap_or(100)
    }

    fn move_locked(&mut self, delta: i16) -> Change {
        let before = *self;
        let (hardware, alpha) = (i16::from(self.hardware), i16::from(self.alpha));
        let lowest =
            (i16::from(self.floors.hardware) - hardware).max(i16::from(self.floors.alpha) - alpha);
        let highest = (100 - hardware).min(100 - alpha);
        // A valid state has lowest <= 0 <= highest, so the clamp cannot panic.
        let mut delta = delta.clamp(lowest.min(0), highest.max(0));
        while delta < 0
            && !Self::meets_effective(hardware + delta, alpha + delta, self.floors.effective)
        {
            delta += 1;
        }
        self.hardware = u8::try_from(hardware + delta).unwrap_or(self.hardware);
        self.alpha = u8::try_from(alpha + delta).unwrap_or(self.alpha);
        self.change_from(before)
    }

    /// Establish the invariant from any field values.
    fn normalize(&mut self) {
        self.hardware = self.hardware.clamp(self.floors.hardware, 100);
        self.alpha = self.alpha.clamp(self.floors.alpha, 100);
        // Terminates: each pass raises a level below 100, and at 100 × 100 any
        // effective floor (at most 100 %) is met.
        while self.product() < u16::from(self.floors.effective) * 100 {
            if self.hardware < self.alpha {
                self.hardware += 1;
            } else {
                self.alpha += 1;
            }
        }
    }

    fn change_from(self, before: Self) -> Change {
        Change {
            hardware: self.hardware != before.hardware,
            alpha: self.alpha != before.alpha,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_invariant(levels: MonitorLevels) {
        let floors = levels.floors();
        let (hardware, alpha) = (levels.hardware().percent(), levels.alpha().percent());
        assert!((floors.hardware()..=100).contains(&hardware), "{levels:?}");
        assert!((floors.alpha()..=100).contains(&alpha), "{levels:?}");
        assert!(
            u16::from(hardware) * u16::from(alpha) >= u16::from(floors.effective()) * 100,
            "{levels:?}"
        );
        assert!(floors.hardware() >= Floors::HARD_MINIMUM.hardware());
        assert!(floors.alpha() >= Floors::HARD_MINIMUM.alpha());
        assert!(floors.effective() >= Floors::HARD_MINIMUM.effective());
    }

    fn levels(hardware: u8, alpha: u8, locked: bool) -> MonitorLevels {
        let levels = MonitorLevels::clamped(hardware, alpha, locked, Floors::default());
        assert_eq!(
            (levels.hardware().percent(), levels.alpha().percent()),
            (hardware, alpha),
            "test setup must be a valid state"
        );
        levels
    }

    #[test]
    fn floors_clamp_to_hard_minimum_and_100() {
        assert_eq!(Floors::clamped(0, 0, 0), Floors::HARD_MINIMUM);
        let software = Floors::clamped(10, 15, 40).software_only();
        assert_eq!(software.alpha(), 15);
        assert_eq!(software.effective(), 15, "the alpha floor decides alone");
        let high = Floors::clamped(200, 255, 101);
        assert_eq!(
            (high.hardware(), high.alpha(), high.effective()),
            (100, 100, 100)
        );
        assert_eq!(Floors::default(), Floors::PROVISIONAL_DEFAULT);
        assert_eq!(
            Floors::clamped(10, 20, 10),
            Floors::PROVISIONAL_DEFAULT,
            "defaults are expressible"
        );
    }

    #[test]
    fn defaults_are_full_and_unlocked() {
        let levels = MonitorLevels::default();
        assert_eq!(levels.hardware(), Level::FULL);
        assert_eq!(levels.alpha(), Level::FULL);
        assert!(!levels.locked());
        assert_eq!(levels.effective_percent(), 100);
    }

    #[test]
    fn unlocked_sliders_clamp_to_their_floors() {
        let mut levels = MonitorLevels::default();
        assert_eq!(
            levels.set_hardware(0),
            Change {
                hardware: true,
                alpha: false
            }
        );
        assert_eq!(levels.hardware().percent(), 10);
        assert_eq!(levels.alpha(), Level::FULL);
        let mut levels = MonitorLevels::default();
        let _ = levels.set_alpha(3);
        assert_eq!(levels.alpha().percent(), 20);
        let _ = levels.set_alpha(250);
        assert_eq!(levels.alpha(), Level::FULL);
    }

    #[test]
    fn effective_floor_limits_the_second_slider() {
        // Alpha at 20 %: hardware may go no lower than 50 (50 × 20 = 10 %).
        let mut levels = levels(100, 20, false);
        let _ = levels.set_hardware(10);
        assert_eq!(levels.hardware().percent(), 50);
        assert_eq!(levels.effective_percent(), 10);
        // A non-divisible case rounds the bound up: alpha 30 needs 34.
        let mut levels = levels_with_alpha(30);
        let _ = levels.set_hardware(1);
        assert_eq!(levels.hardware().percent(), 34);
        assert_invariant(levels);
        // And the mirror: hardware 25 holds alpha at 40.
        let mut levels = self::levels(25, 100, false);
        let _ = levels.set_alpha(1);
        assert_eq!(levels.alpha().percent(), 40);
    }

    fn levels_with_alpha(alpha: u8) -> MonitorLevels {
        levels(100, alpha, false)
    }

    #[test]
    fn unchanged_value_reports_no_change() {
        let mut levels = MonitorLevels::default();
        assert!(!levels.set_hardware(100).any());
        assert!(!levels.set_alpha(100).any());
        let _ = levels.set_hardware(10);
        assert!(!levels.set_hardware(0).any(), "already at the floor");
    }

    #[test]
    fn locked_move_keeps_the_offset() {
        let mut levels = levels(80, 60, true);
        assert_eq!(levels.offset(), 20);
        assert_eq!(
            levels.set_hardware(70),
            Change {
                hardware: true,
                alpha: true
            }
        );
        assert_eq!(
            (levels.hardware().percent(), levels.alpha().percent()),
            (70, 50)
        );
        let _ = levels.set_alpha(70);
        assert_eq!(
            (levels.hardware().percent(), levels.alpha().percent()),
            (90, 70)
        );
        assert_eq!(levels.offset(), 20);
    }

    #[test]
    fn locked_move_stops_at_either_bound_without_breaking_offset() {
        // Up: hardware hits 100 first, so alpha stops 20 below.
        let mut levels = levels(80, 60, true);
        let _ = levels.set_alpha(100);
        assert_eq!(
            (levels.hardware().percent(), levels.alpha().percent()),
            (100, 80)
        );
        // Down: the effective floor stops both, keeping the offset.
        let _ = levels.set_hardware(0);
        assert_eq!(levels.offset(), 20);
        assert_invariant(levels);
        let (hardware, alpha) = (levels.hardware().percent(), levels.alpha().percent());
        // One step further down would cross the effective floor.
        assert!(u16::from(hardware - 1) * u16::from(alpha - 1) < 1000);
        // Down where an individual floor binds first: alpha floor 20.
        let mut levels = self::levels(100, 50, true);
        let _ = levels.set_hardware(0);
        assert!(levels.alpha().percent() >= 20);
        assert_eq!(levels.offset(), 50);
        assert_invariant(levels);
    }

    #[test]
    fn relocking_keeps_the_new_offset() {
        let mut levels = levels(80, 60, true);
        levels.set_locked(false);
        let _ = levels.set_alpha(90);
        assert_eq!(levels.hardware().percent(), 80, "unlocked moves one level");
        levels.set_locked(true);
        assert_eq!(levels.offset(), -10);
        let _ = levels.set_hardware(60);
        assert_eq!(
            (levels.hardware().percent(), levels.alpha().percent()),
            (60, 70)
        );
    }

    #[test]
    fn raise_lifts_the_lowest_level() {
        let mut levels = levels(40, 90, false);
        assert_eq!(
            levels.raise(10),
            Change {
                hardware: true,
                alpha: false
            }
        );
        assert_eq!(
            (levels.hardware().percent(), levels.alpha().percent()),
            (50, 90)
        );
        let mut levels = self::levels(90, 30, false);
        let _ = levels.raise(10);
        assert_eq!(
            (levels.hardware().percent(), levels.alpha().percent()),
            (90, 40)
        );
        let mut tie = self::levels(50, 50, false);
        let _ = tie.raise(5);
        assert_eq!((tie.hardware().percent(), tie.alpha().percent()), (55, 55));
    }

    #[test]
    fn raise_works_when_locked_even_at_the_top() {
        let mut levels = levels(95, 60, true);
        let change = levels.raise(10);
        assert!(change.alpha, "the lowest level rose");
        assert_eq!(
            (levels.hardware().percent(), levels.alpha().percent()),
            (100, 70)
        );
        assert_eq!(levels.offset(), 30, "offset shrinks rather than refusing");
        assert!(levels.locked());
    }

    #[test]
    fn raise_saturates_and_zero_step_still_raises() {
        let mut levels = levels(98, 99, false);
        let _ = levels.raise(0);
        assert_eq!(levels.hardware().percent(), 99);
        let _ = levels.raise(u8::MAX);
        let _ = levels.raise(u8::MAX);
        assert_eq!(levels, MonitorLevels::default());
        assert!(!levels.raise(10).any(), "nothing to raise at full");
    }

    #[test]
    fn repeated_raise_always_reaches_full() {
        let mut levels = MonitorLevels::clamped(0, 0, true, Floors::HARD_MINIMUM);
        assert_invariant(levels);
        for _ in 0..20 {
            let _ = levels.raise(10);
            assert_invariant(levels);
        }
        assert_eq!(
            (levels.hardware(), levels.alpha()),
            (Level::FULL, Level::FULL)
        );
    }

    #[test]
    fn clamped_establishes_the_invariant_from_anything() {
        for floors in [
            Floors::HARD_MINIMUM,
            Floors::PROVISIONAL_DEFAULT,
            Floors::clamped(50, 50, 60),
            Floors::clamped(100, 100, 100),
        ] {
            for hardware in (0..=u8::MAX).step_by(7) {
                for alpha in (0..=u8::MAX).step_by(5) {
                    let levels = MonitorLevels::clamped(hardware, alpha, false, floors);
                    assert_invariant(levels);
                }
            }
        }
    }

    #[test]
    fn no_operation_sequence_breaks_the_invariant() {
        // A small deterministic generator, so the test needs no dependency.
        let mut seed: u32 = 0x9E37_79B9;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let mut levels = MonitorLevels::default();
        for _ in 0..20_000 {
            let value = u8::try_from(next() % 256).unwrap();
            match next() % 6 {
                0 => {
                    let _ = levels.set_hardware(value);
                }
                1 => {
                    let _ = levels.set_alpha(value);
                }
                2 => levels.set_locked(value % 2 == 0),
                3 => {
                    let _ = levels.raise(value % 20);
                }
                4 => {
                    let floors = Floors::clamped(value, value / 2, value / 3);
                    let _ = levels.set_floors(floors);
                }
                _ => {
                    let before = levels.offset();
                    if levels.locked() {
                        let _ = levels.set_hardware(value);
                        assert_eq!(levels.offset(), before, "locked moves keep offset");
                    }
                }
            }
            assert_invariant(levels);
        }
    }

    #[test]
    fn raising_floors_raises_levels() {
        let mut levels = levels(10, 100, false);
        let change = levels.set_floors(Floors::clamped(30, 20, 10));
        assert_eq!(
            change,
            Change {
                hardware: true,
                alpha: false
            }
        );
        assert_eq!(levels.hardware().percent(), 30);
        let mut levels = self::levels(50, 50, false);
        let _ = levels.set_floors(Floors::clamped(10, 20, 40));
        assert_invariant(levels);
        assert!(levels.effective_percent() >= 40);
    }
}
