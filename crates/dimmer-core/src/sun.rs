//! Where the sun is in its daily cycle, for auto dimming.
//!
//! Auto dimming remembers levels by [`Phase`]: how far the time is between
//! sunrise and sunset, or between sunset and the next sunrise. A level set an
//! hour after sunset in June then applies an hour or so after sunset in
//! December too, even though the clock time is hours apart.
//!
//! Sunrise and sunset come from the sunrise equation (as given on Wikipedia's
//! "Sunrise equation" page), which is accurate to a minute or two away from
//! the poles. Everything is in UTC, so no time zone is involved. Pure
//! arithmetic: no clock, no I/O.

use std::f64::consts::PI;

/// Seconds in a day.
const DAY: f64 = 86_400.0;
/// Julian date of the Unix epoch, 1970-01-01 00:00 UTC.
const UNIX_EPOCH_JD: f64 = 2_440_587.5;
/// Julian date of J2000.0, 2000-01-01 12:00 TT.
const J2000: f64 = 2_451_545.0;
/// The sun's centre is this far below the horizon at sunrise and sunset,
/// allowing for refraction and the sun's radius, in degrees.
const HORIZON: f64 = -0.833;
/// The Earth's axial tilt, in degrees.
const OBLIQUITY: f64 = 23.4397;
/// The sun's mean anomaly at J2000 and its daily motion, in degrees.
const MEAN_ANOMALY_AT_J2000: f64 = 357.5291;
const MEAN_ANOMALY_PER_DAY: f64 = 0.985_600_28;
/// The equation of the centre's terms, in degrees: the sun's apparent
/// position runs ahead of and behind its mean position over the year.
const CENTRE: [f64; 3] = [1.9148, 0.0200, 0.0003];
/// The argument of perihelion, in degrees.
const PERIHELION: f64 = 102.9372;
/// The equation of time's terms, in days: solar noon drifts from mean noon
/// with the orbit's eccentricity and the axial tilt.
const EQUATION_OF_TIME: [f64; 2] = [0.0053, 0.0069];

/// A place on Earth in degrees: latitude north positive, longitude east
/// positive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Location {
    latitude: f64,
    longitude: f64,
}

impl Location {
    /// A location, if both numbers are finite and in range.
    #[must_use]
    pub fn new(latitude: f64, longitude: f64) -> Option<Self> {
        let valid = latitude.is_finite()
            && longitude.is_finite()
            && (-90.0..=90.0).contains(&latitude)
            && (-180.0..=180.0).contains(&longitude);
        valid.then_some(Self {
            latitude,
            longitude,
        })
    }

    #[must_use]
    pub const fn latitude(&self) -> f64 {
        self.latitude
    }

    #[must_use]
    pub const fn longitude(&self) -> f64 {
        self.longitude
    }
}

/// Where a moment falls in the sun's cycle, in thousandths: `0` is sunrise,
/// `1000` sunset, and `1000..2000` the night until the next sunrise.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Phase(u16);

impl Phase {
    pub const SUNRISE: Self = Self(0);
    pub const SUNSET: Self = Self(1000);
    /// One full cycle: day and night.
    pub const CYCLE: u16 = 2000;

    /// A phase, wrapped into one cycle.
    #[must_use]
    pub const fn new(thousandths: u16) -> Self {
        Self(thousandths % Self::CYCLE)
    }

    #[must_use]
    pub const fn thousandths(self) -> u16 {
        self.0
    }

    /// How far forward `later` is from `self`, going round the cycle.
    #[must_use]
    pub const fn until(self, later: Self) -> u16 {
        (later.0 + Self::CYCLE - self.0) % Self::CYCLE
    }

    /// The shorter way round the cycle between two phases.
    #[must_use]
    pub fn distance(self, other: Self) -> u16 {
        self.until(other).min(other.until(self))
    }
}

/// Sunrise and sunset of one solar day, as Unix seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Day {
    Normal {
        rise: f64,
        set: f64,
    },
    /// The sun stays up, or down, all day.
    Polar,
}

fn radians(degrees: f64) -> f64 {
    degrees * PI / 180.0
}

fn degrees(radians: f64) -> f64 {
    radians * 180.0 / PI
}

/// The solar day whose noon falls nearest `day` days after J2000 at
/// `location`.
fn solar_day(location: Location, day: f64) -> Day {
    let mean_noon = day - location.longitude / 360.0;
    let anomaly = (MEAN_ANOMALY_AT_J2000 + MEAN_ANOMALY_PER_DAY * mean_noon).rem_euclid(360.0);
    let m = radians(anomaly);
    let centre = CENTRE[0] * m.sin() + CENTRE[1] * (2.0 * m).sin() + CENTRE[2] * (3.0 * m).sin();
    let ecliptic = radians((anomaly + centre + 180.0 + PERIHELION).rem_euclid(360.0));
    let transit = J2000 + mean_noon + EQUATION_OF_TIME[0] * m.sin()
        - EQUATION_OF_TIME[1] * (2.0 * ecliptic).sin();
    let declination = (ecliptic.sin() * radians(OBLIQUITY).sin()).asin();
    let latitude = radians(location.latitude);
    let cos_hour_angle = (radians(HORIZON).sin() - latitude.sin() * declination.sin())
        / (latitude.cos() * declination.cos());
    // Beyond ±1 the sun never crosses the horizon: always down above 1,
    // always up below -1.
    if cos_hour_angle.abs() > 1.0 {
        return Day::Polar;
    }
    let half_day = degrees(cos_hour_angle.acos()) / 360.0;
    let unix = |julian: f64| (julian - UNIX_EPOCH_JD) * DAY;
    Day::Normal {
        rise: unix(transit - half_day),
        set: unix(transit + half_day),
    }
}

/// Where `unix_seconds` falls in the sun's cycle at `location`.
///
/// Where the sun does not rise or set that day (polar day or night), the
/// cycle falls back to local solar time with sunrise at 06:00 and sunset at
/// 18:00, so auto dimming still follows the clock.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "Unix seconds fit f64 exactly until the year 285 million"
)]
pub fn phase(location: Location, unix_seconds: i64) -> Phase {
    let now = unix_seconds as f64;
    let today = day_number(now);
    // Sunrises and sunsets from yesterday to tomorrow, in time order.
    let mut events = Vec::with_capacity(6);
    for offset in [-1.0, 0.0, 1.0] {
        match solar_day(location, today + offset) {
            Day::Normal { rise, set } => {
                events.push((rise, Phase::SUNRISE));
                events.push((set, Phase::SUNSET));
            }
            Day::Polar => return solar_time_phase(location, now),
        }
    }
    events.sort_by(|a, b| a.0.total_cmp(&b.0));
    let next = events.iter().position(|(time, _)| *time > now);
    match next {
        Some(next) if next > 0 => {
            let (start, from) = events[next - 1];
            let (end, _) = events[next];
            let fraction = ((now - start) / (end - start)).clamp(0.0, 1.0);
            Phase::new(from.0 + thousandths(fraction))
        }
        // Three days of events always bracket a moment inside them.
        _ => solar_time_phase(location, now),
    }
}

/// The whole number of days from J2000 to the solar day around `unix`
/// seconds, as the sunrise equation counts them.
fn day_number(unix: f64) -> f64 {
    (unix / DAY + UNIX_EPOCH_JD - J2000 + 0.0008).round()
}

/// The phase by local mean solar time: sunrise at 06:00, sunset at 18:00.
fn solar_time_phase(location: Location, now: f64) -> Phase {
    let solar_seconds = (now + location.longitude / 360.0 * DAY).rem_euclid(DAY);
    let since_sunrise = (solar_seconds - DAY / 4.0).rem_euclid(DAY);
    Phase::new(thousandths(since_sunrise / DAY * 2.0))
}

/// `fraction` (0 to 2) in thousandths.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to 0..=2000 first"
)]
fn thousandths(fraction: f64) -> u16 {
    (fraction * 1000.0).round().clamp(0.0, 2000.0) as u16
}

#[cfg(test)]
#[expect(
    clippy::cast_precision_loss,
    reason = "test dates are far inside f64's exact range"
)]
mod tests {
    use super::*;

    /// Royal Observatory, Greenwich.
    fn greenwich() -> Location {
        Location::new(51.4769, 0.0).unwrap()
    }

    /// Unix seconds of a UTC date and time (days from civil, Howard Hinnant).
    fn utc(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> i64 {
        let y = if month <= 2 { year - 1 } else { year };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (month + 9) % 12;
        let doy = (153 * mp + 2) / 5 + day - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        days * 86_400 + hour * 3600 + minute * 60
    }

    fn rise_and_set(location: Location, unix: i64) -> (f64, f64) {
        let day = day_number(unix as f64);
        match solar_day(location, day) {
            Day::Normal { rise, set } => (rise, set),
            Day::Polar => panic!("no sunrise or sunset that day"),
        }
    }

    fn assert_near(actual: f64, expected: i64, label: &str) {
        let minutes = (actual - expected as f64).abs() / 60.0;
        assert!(minutes < 4.0, "{label} off by {minutes:.1} minutes");
    }

    #[test]
    fn greenwich_equinox_sunrise_and_sunset() {
        // Published times for 2024-03-20: sunrise 06:02, sunset 18:14 UTC.
        let (rise, set) = rise_and_set(greenwich(), utc(2024, 3, 20, 12, 0));
        assert_near(rise, utc(2024, 3, 20, 6, 2), "sunrise");
        assert_near(set, utc(2024, 3, 20, 18, 14), "sunset");
    }

    #[test]
    fn denver_solstice_sunrise_and_sunset() {
        // Denver on 2024-06-20: sunrise 05:31 MDT, sunset 20:31 MDT (UTC-6).
        let denver = Location::new(39.7392, -104.9903).unwrap();
        let (rise, set) = rise_and_set(denver, utc(2024, 6, 20, 18, 0));
        assert_near(rise, utc(2024, 6, 20, 11, 31), "sunrise");
        assert_near(set, utc(2024, 6, 21, 2, 31), "sunset");
    }

    #[test]
    fn phase_runs_from_sunrise_through_sunset_to_the_next_sunrise() {
        let place = greenwich();
        let at = |hour, minute| phase(place, utc(2024, 3, 20, hour, minute)).thousandths();
        assert!(at(6, 5) < 20, "just after sunrise: {}", at(6, 5));
        let noon = at(12, 8);
        assert!((480..=520).contains(&noon), "solar noon: {noon}");
        let dusk = at(18, 20);
        assert!((1000..1030).contains(&dusk), "just after sunset: {dusk}");
        let midnight = at(0, 8);
        assert!((1450..=1550).contains(&midnight), "midnight: {midnight}");
    }

    #[test]
    fn phase_is_continuous_over_a_day() {
        let place = Location::new(-33.87, 151.21).unwrap(); // Sydney
        let start = utc(2025, 11, 7, 0, 0);
        let mut previous = phase(place, start);
        for minute in 1..=24 * 60 {
            let current = phase(place, start + minute * 60);
            assert!(
                previous.distance(current) <= 5,
                "jump at minute {minute}: {previous:?} -> {current:?}"
            );
            previous = current;
        }
    }

    #[test]
    fn polar_days_follow_solar_time() {
        let svalbard = Location::new(78.22, 15.65).unwrap();
        // Midnight sun in June, polar night in December: both fall back.
        for unix in [utc(2025, 6, 21, 0, 0), utc(2025, 12, 21, 0, 0)] {
            let day = day_number(unix as f64);
            assert_eq!(solar_day(svalbard, day), Day::Polar);
        }
        // 15.65 degrees east is about 01:03 solar time at 00:00 UTC: night.
        let night = phase(svalbard, utc(2025, 6, 21, 0, 0)).thousandths();
        assert!((1500..=1600).contains(&night), "{night}");
        // 11:00 UTC is about 12:03 solar time: the middle of the day.
        let noon = phase(svalbard, utc(2025, 6, 21, 11, 0)).thousandths();
        assert!((490..=520).contains(&noon), "{noon}");
    }

    #[test]
    fn locations_are_validated() {
        assert!(Location::new(91.0, 0.0).is_none());
        assert!(Location::new(0.0, -180.5).is_none());
        assert!(Location::new(f64::NAN, 0.0).is_none());
        assert!(Location::new(-90.0, 180.0).is_some());
    }

    #[test]
    fn phases_wrap_and_measure_the_short_way_round() {
        assert_eq!(Phase::new(2100), Phase::new(100));
        assert_eq!(Phase::new(1900).until(Phase::new(100)), 200);
        assert_eq!(Phase::new(1900).distance(Phase::new(100)), 200);
        assert_eq!(Phase::new(100).distance(Phase::new(1900)), 200);
        assert_eq!(Phase::SUNRISE.distance(Phase::SUNSET), 1000);
    }
}
