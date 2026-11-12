//! A display's auto-dimming profile: the levels the user chose at points in
//! the sun's cycle, and the levels in between.
//!
//! Every time the user sets a level while auto dimming is on, the profile
//! remembers it at the current [`Phase`], replacing anything set within
//! [`Profile::NEARBY`] of it. Between two remembered points the level changes
//! along a half-cosine ease (slow leaving a point, fastest midway, slow
//! arriving at the next), all the way round the cycle, so the profile has no
//! jumps and no corners at the points. With one point the level is constant;
//! with none there is no target.
//!
//! Levels here are the ones the user set; [`crate::MonitorLevels`] still
//! applies the floors when a target is used.

use crate::sun::Phase;

/// One remembered choice: the hardware and software levels at a phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Point {
    pub phase: Phase,
    pub hardware: u8,
    pub alpha: u8,
}

/// The levels to aim for at a phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Target {
    pub hardware: u8,
    pub alpha: u8,
}

/// A display's remembered points, in phase order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Profile {
    points: Vec<Point>,
}

impl Profile {
    /// A new choice replaces the points this close to it: 40 thousandths is
    /// about half an hour of a 12-hour day.
    pub const NEARBY: u16 = 40;
    /// At most this many points, so the profile stays small on disk.
    pub const MAX_POINTS: usize = 24;

    /// A profile from saved points: sorted, out-of-range levels clamped to
    /// 1..=100, and only the first point kept where several share a phase.
    #[must_use]
    pub fn from_points(points: impl IntoIterator<Item = Point>) -> Self {
        let mut points: Vec<Point> = points
            .into_iter()
            .map(|point| Point {
                hardware: point.hardware.clamp(1, 100),
                alpha: point.alpha.clamp(1, 100),
                ..point
            })
            .collect();
        points.sort_by_key(|point| point.phase);
        points.dedup_by_key(|point| point.phase);
        points.truncate(Self::MAX_POINTS);
        Self { points }
    }

    #[must_use]
    pub fn points(&self) -> &[Point] {
        &self.points
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Remember that the user chose `hardware` and `alpha` at `phase`.
    pub fn learn(&mut self, phase: Phase, hardware: u8, alpha: u8) {
        self.points
            .retain(|point| point.phase.distance(phase) > Self::NEARBY);
        if self.points.len() >= Self::MAX_POINTS {
            // Make room by dropping the point nearest the new choice, which
            // takes its place: the rest of the day keeps the shape the user
            // gave it.
            if let Some(nearest) =
                (0..self.points.len()).min_by_key(|&index| self.points[index].phase.distance(phase))
            {
                self.points.remove(nearest);
            }
        }
        let at = self.points.partition_point(|point| point.phase < phase);
        self.points.insert(
            at,
            Point {
                phase,
                hardware: hardware.clamp(1, 100),
                alpha: alpha.clamp(1, 100),
            },
        );
    }

    /// The levels the profile gives at `phase`, or `None` when it is empty.
    #[must_use]
    pub fn target(&self, phase: Phase) -> Option<Target> {
        let first = *self.points.first()?;
        let last = *self.points.last()?;
        // The points either side of `phase`, going round the cycle.
        let after = self.points.partition_point(|point| point.phase <= phase);
        // Before the first point or after the last, the neighbours are the
        // last and first points, round the end of the cycle.
        let (before, next) = if after == 0 || after == self.points.len() {
            (last, first)
        } else {
            (self.points[after - 1], self.points[after])
        };
        let span = before.phase.until(next.phase);
        if span == 0 {
            // A single point: the level is constant.
            return Some(Target {
                hardware: before.hardware,
                alpha: before.alpha,
            });
        }
        let along = before.phase.until(phase);
        // A half-cosine ease: slow near each point, fastest midway.
        let eased = (1.0 - (std::f32::consts::PI * f32::from(along) / f32::from(span)).cos()) / 2.0;
        let between = |from: u8, to: u8| -> u8 {
            let from = f32::from(from);
            let level = (from + (f32::from(to) - from) * eased).round();
            // Clamped to 1..=100 first, so the cast cannot truncate or wrap.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let level = level.clamp(1.0, 100.0) as u8;
            level
        };
        Some(Target {
            hardware: between(before.hardware, next.hardware),
            alpha: between(before.alpha, next.alpha),
        })
    }
}

/// One step of at most 1 % from `current` toward `target`.
#[must_use]
pub fn step_toward(current: u8, target: u8) -> u8 {
    match current.cmp(&target) {
        std::cmp::Ordering::Less => current + 1,
        std::cmp::Ordering::Greater => current - 1,
        std::cmp::Ordering::Equal => current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(thousandths: u16) -> Phase {
        Phase::new(thousandths)
    }

    fn alpha(profile: &Profile, phase: u16) -> u8 {
        profile.target(at(phase)).unwrap().alpha
    }

    #[test]
    fn an_empty_profile_has_no_target() {
        assert_eq!(Profile::default().target(at(500)), None);
    }

    #[test]
    fn one_point_is_constant() {
        let mut profile = Profile::default();
        profile.learn(at(1200), 90, 40);
        for phase in [0, 500, 1200, 1999] {
            assert_eq!(
                profile.target(at(phase)),
                Some(Target {
                    hardware: 90,
                    alpha: 40
                })
            );
        }
    }

    #[test]
    fn levels_ease_between_points() {
        let mut profile = Profile::default();
        profile.learn(at(500), 100, 100); // midday
        profile.learn(at(1500), 100, 20); // midnight
        // Exact at the points, and the midpoint is the average.
        assert_eq!(alpha(&profile, 500), 100);
        assert_eq!(alpha(&profile, 1500), 20);
        assert_eq!(alpha(&profile, 1000), 60);
        // Round the end of the cycle, from midnight back to midday.
        assert_eq!(alpha(&profile, 0), 60);
        // Slower than a straight line near the points.
        assert!(alpha(&profile, 600) > 92);
        assert!(alpha(&profile, 1400) < 28);
        // Symmetric about the midpoint, and never rising while falling.
        for along in 0..=1000 {
            let early = i32::from(alpha(&profile, 500 + along));
            let late = i32::from(alpha(&profile, 1500 - along));
            assert_eq!(early + late, 120, "asymmetric at {along}");
            if along > 0 {
                assert!(early <= i32::from(alpha(&profile, 499 + along)));
            }
        }
    }

    #[test]
    fn a_rising_ease_is_monotonic_and_clamped() {
        let mut profile = Profile::default();
        profile.learn(at(300), 1, 1);
        profile.learn(at(900), 100, 100);
        let mut previous = alpha(&profile, 300);
        assert_eq!(previous, 1);
        for phase in 301..=900 {
            let current = alpha(&profile, phase);
            assert!((1..=100).contains(&current) && current >= previous);
            previous = current;
        }
        assert_eq!(previous, 100);
    }

    #[test]
    fn neighbouring_targets_make_no_big_jumps() {
        let mut profile = Profile::default();
        profile.learn(at(100), 100, 100);
        profile.learn(at(1100), 100, 10);
        let mut previous = alpha(&profile, 0);
        for phase in 1..Phase::CYCLE {
            let current = alpha(&profile, phase);
            // The ease is steepest midway: pi / 2 times the average slope.
            assert!(previous.abs_diff(current) <= 1, "jump at {phase}");
            previous = current;
        }
    }

    #[test]
    fn step_toward_moves_one_percent_per_step() {
        let mut level = 100;
        for _ in 0..90 {
            let next = step_toward(level, 10);
            assert_eq!(level - next, 1);
            level = next;
        }
        assert_eq!(level, 10);
    }

    #[test]
    fn a_new_choice_replaces_nearby_points() {
        let mut profile = Profile::default();
        profile.learn(at(1000), 100, 50);
        profile.learn(at(1030), 100, 30);
        assert_eq!(profile.points().len(), 1);
        assert_eq!(alpha(&profile, 1000), 30);
        profile.learn(at(1100), 100, 25);
        assert_eq!(profile.points().len(), 2);
        // Near the end of the cycle, nearby wraps round to the start.
        profile.learn(at(1990), 100, 80);
        profile.learn(at(10), 100, 70);
        assert_eq!(
            profile.points().iter().map(|p| p.phase).collect::<Vec<_>>(),
            [at(10), at(1030), at(1100)]
        );
    }

    #[test]
    fn the_profile_stays_bounded() {
        let mut profile = Profile::default();
        for index in 0..100u16 {
            profile.learn(at(index * 50 % Phase::CYCLE), 100, 50);
        }
        assert!(profile.points().len() <= Profile::MAX_POINTS);
        let phases: Vec<_> = profile.points().iter().map(|p| p.phase).collect();
        assert!(phases.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn saved_points_are_cleaned_up() {
        let profile = Profile::from_points([
            Point {
                phase: at(900),
                hardware: 0,
                alpha: 200,
            },
            Point {
                phase: at(100),
                hardware: 50,
                alpha: 50,
            },
            Point {
                phase: at(100),
                hardware: 60,
                alpha: 60,
            },
        ]);
        assert_eq!(
            profile.points(),
            [
                Point {
                    phase: at(100),
                    hardware: 50,
                    alpha: 50
                },
                Point {
                    phase: at(900),
                    hardware: 1,
                    alpha: 100
                },
            ]
        );
    }

    #[test]
    fn steps_are_one_percent() {
        assert_eq!(step_toward(50, 60), 51);
        assert_eq!(step_toward(50, 40), 49);
        assert_eq!(step_toward(50, 50), 50);
    }
}
