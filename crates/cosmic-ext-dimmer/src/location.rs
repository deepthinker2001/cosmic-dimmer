//! Where the user is, for sunrise and sunset: typed coordinates, or else the
//! reference city of the system time zone from the tz database's
//! `zone1970.tab` (Denver for `America/Denver`). Both are read locally;
//! nothing is looked up online. A time zone city can be a few hundred
//! kilometres away, which moves sunrise and sunset by up to half an hour or
//! so; typed coordinates fix that.

use std::io::Read;
use std::path::Path;

use dimmer_core::sun::Location;

/// The tz database's table of zones and their reference cities.
const ZONE_TABLE: &str = "/usr/share/zoneinfo/zone1970.tab";
/// The link to the system time zone, into `/usr/share/zoneinfo`.
const LOCALTIME: &str = "/etc/localtime";
/// The table is about 20 KiB; anything far bigger is not it.
const MAX_TABLE_BYTES: u64 = 1 << 20;

/// Where the location came from, for the settings view.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Typed,
    /// The time zone whose reference city is used, such as `America/Denver`.
    TimeZone(String),
}

/// The location to use: the typed one, else the time zone's city.
#[must_use]
pub fn location(typed: Option<Location>) -> Option<(Location, Source)> {
    if let Some(location) = typed {
        return Some((location, Source::Typed));
    }
    let zone = system_zone()?;
    let table = read_bounded(Path::new(ZONE_TABLE))?;
    let location = zone_location(&table, &zone)?;
    Some((location, Source::TimeZone(zone)))
}

/// `latitude, longitude` in decimal degrees, as typed in the settings.
#[must_use]
pub fn parse_coordinates(text: &str) -> Option<Location> {
    let (latitude, longitude) = text.split_once(',')?;
    Location::new(
        latitude.trim().parse().ok()?,
        longitude.trim().parse().ok()?,
    )
}

/// The system time zone's name: `$TZ` when it names a zone, else where
/// `/etc/localtime` links to.
fn system_zone() -> Option<String> {
    if let Ok(zone) = std::env::var("TZ") {
        let zone = zone.trim_start_matches(':');
        if !zone.is_empty() && !zone.starts_with('/') {
            return Some(zone.to_owned());
        }
    }
    let target = std::fs::read_link(LOCALTIME).ok()?;
    zone_from_path(&target.to_string_lossy())
}

/// `America/Denver` from `/usr/share/zoneinfo/America/Denver` or a relative
/// link like `../usr/share/zoneinfo/America/Denver`.
fn zone_from_path(path: &str) -> Option<String> {
    let (_, zone) = path.split_once("zoneinfo/")?;
    (!zone.is_empty()).then(|| zone.to_owned())
}

fn read_bounded(path: &Path) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_TABLE_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

/// The reference city of `zone` in a `zone1970.tab` table. Its lines are
/// `countries<TAB>coordinates<TAB>zone[<TAB>comment]`; `#` starts a comment.
fn zone_location(table: &str, zone: &str) -> Option<Location> {
    table
        .lines()
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| {
            let mut fields = line.split('\t');
            let (_, coordinates, name) = (fields.next()?, fields.next()?, fields.next()?);
            (name == zone).then(|| iso6709(coordinates))?
        })
}

/// ISO 6709 coordinates as the tz database writes them: `±DDMM±DDDMM` or
/// `±DDMMSS±DDDMMSS`.
fn iso6709(text: &str) -> Option<Location> {
    let split = text.get(1..)?.find(['+', '-'])? + 1;
    let (latitude, longitude) = text.split_at(split);
    Location::new(sexagesimal(latitude, 2)?, sexagesimal(longitude, 3)?)
}

/// `±DDMM[SS]` (or `±DDDMM[SS]` with three degree digits) in degrees.
fn sexagesimal(text: &str, degree_digits: usize) -> Option<f64> {
    let sign = match text.as_bytes().first()? {
        b'+' => 1.0,
        b'-' => -1.0,
        _ => return None,
    };
    let digits = &text[1..];
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let part = |range: std::ops::Range<usize>| -> Option<f64> { digits.get(range)?.parse().ok() };
    let degrees = part(0..degree_digits)?;
    let minutes = part(degree_digits..degree_digits + 2)?;
    let seconds = match digits.len() - degree_digits {
        2 => 0.0,
        4 => part(degree_digits + 2..degree_digits + 4)?,
        _ => return None,
    };
    Some(sign * (degrees + minutes / 60.0 + seconds / 3600.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "# tzdb timezone descriptions\n\
        #codes\tcoordinates\tTZ\tcomments\n\
        US\t+394421-1045903\tAmerica/Denver\tMountain (most areas)\n\
        GB,GG,IM,JE\t+513030-0000731\tEurope/London\n\
        AU\t-3352+15113\tAustralia/Sydney\tNew South Wales (most areas)\n";

    fn near(location: Location, latitude: f64, longitude: f64) -> bool {
        (location.latitude() - latitude).abs() < 0.01
            && (location.longitude() - longitude).abs() < 0.01
    }

    #[test]
    fn zone_cities_come_from_the_table() {
        let denver = zone_location(TABLE, "America/Denver").unwrap();
        assert!(near(denver, 39.7392, -104.9842), "{denver:?}");
        let london = zone_location(TABLE, "Europe/London").unwrap();
        assert!(near(london, 51.5083, -0.1253), "{london:?}");
        let sydney = zone_location(TABLE, "Australia/Sydney").unwrap();
        assert!(near(sydney, -33.8667, 151.2167), "{sydney:?}");
        assert_eq!(zone_location(TABLE, "America/Den"), None);
        assert_eq!(zone_location(TABLE, "TZ"), None, "header lines skipped");
    }

    #[test]
    fn malformed_coordinates_are_refused() {
        for text in [
            "",
            "+",
            "+3944",
            "+3944-10459x",
            "39.7-104.9",
            "+394-1045903",
        ] {
            assert_eq!(iso6709(text), None, "{text:?}");
        }
        assert_eq!(iso6709("+9100+00000"), None, "latitude out of range");
    }

    #[test]
    fn zone_names_come_from_the_localtime_link() {
        assert_eq!(
            zone_from_path("/usr/share/zoneinfo/America/Denver").as_deref(),
            Some("America/Denver")
        );
        assert_eq!(
            zone_from_path("../usr/share/zoneinfo/Etc/UTC").as_deref(),
            Some("Etc/UTC")
        );
        assert_eq!(zone_from_path("/etc/somewhere"), None);
        assert_eq!(zone_from_path("/usr/share/zoneinfo/"), None);
    }

    #[test]
    fn typed_coordinates_parse() {
        let location = parse_coordinates(" 39.74 , -104.99 ").unwrap();
        assert!(near(location, 39.74, -104.99));
        for text in [
            "",
            "39.74",
            "39.74 -104.99",
            "north, west",
            "95, 0",
            "0, 200",
        ] {
            assert_eq!(parse_coordinates(text), None, "{text:?}");
        }
    }

    #[test]
    fn typed_coordinates_win_over_the_time_zone() {
        let typed = Location::new(10.0, 20.0).unwrap();
        assert_eq!(location(Some(typed)), Some((typed, Source::Typed)));
    }
}
