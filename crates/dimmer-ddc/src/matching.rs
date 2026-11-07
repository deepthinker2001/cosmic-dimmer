//! Which I2C bus reaches which connected monitor. A bus is chosen only when
//! the EDID read over that bus is byte-identical to the EDID the DRM
//! connector holds, so a brightness write cannot go to another monitor.

use std::fmt;

use crate::{
    edid::Edid,
    sysfs::{Adapter, Connector},
};

/// An adapter and the EDID read from it over I2C (`None`: nothing valid).
#[derive(Clone, Debug)]
pub struct ProbedBus<'a> {
    pub adapter: &'a Adapter,
    pub edid: Option<Edid>,
}

/// Why a connected monitor has no bus.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatchProblem {
    /// The connector has no EDID, so there is nothing to verify a bus against.
    NoConnectorEdid,
    /// The connector's EDID is corrupt.
    InvalidConnectorEdid,
    /// No bus returned the connector's EDID (unreadable, or a driver that
    /// exposes no i2c-dev bus for it, such as some DP-MST setups).
    NoMatchingBus,
    /// Several buses return the EDID and the kernel does not say which one
    /// is this connector's: identical monitors without serial numbers on a
    /// driver without `ddc` links. Refused rather than guessed.
    Ambiguous(Vec<u32>),
}

impl fmt::Display for MatchProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoConnectorEdid => f.write_str("the connector has no EDID to match a bus to"),
            Self::InvalidConnectorEdid => f.write_str("the connector's EDID is invalid"),
            Self::NoMatchingBus => f.write_str("no I2C bus returned this monitor's EDID"),
            Self::Ambiguous(buses) => write!(
                f,
                "I2C buses {buses:?} all return this monitor's EDID; refusing to guess"
            ),
        }
    }
}

/// Assign a bus to each connector in `connectors` (the connected ones).
/// Returns one result per connector, in order.
#[must_use]
pub fn assign(
    connectors: &[&Connector],
    buses: &[ProbedBus<'_>],
) -> Vec<Result<u32, MatchProblem>> {
    let edids: Vec<Option<Result<Edid, MatchProblem>>> = connectors
        .iter()
        .map(|c| {
            c.edid
                .as_ref()
                .map(|bytes| Edid::parse(bytes).map_err(|_| MatchProblem::InvalidConnectorEdid))
        })
        .collect();

    connectors
        .iter()
        .zip(&edids)
        .map(|(connector, edid)| {
            let edid = match edid {
                None => return Err(MatchProblem::NoConnectorEdid),
                Some(Err(problem)) => return Err(problem.clone()),
                Some(Ok(edid)) => edid,
            };
            let matching = buses.iter().filter(|b| b.edid.as_ref() == Some(edid));

            // Buses the kernel itself links to this connector win; among
            // them the `ddc` link first, then the lowest number (AMD exposes
            // both an i2c hw bus and an AUX bus for one DP connector).
            let mut tied: Vec<&ProbedBus> = matching
                .clone()
                .filter(|b| b.adapter.tied_to(connector))
                .collect();
            tied.sort_by_key(|b| (connector.ddc_bus != Some(b.adapter.bus), b.adapter.bus));
            if let Some(best) = tied.first() {
                return Ok(best.adapter.bus);
            }

            // Otherwise only buses no connector claims, and only when no other
            // connected monitor shows the same EDID.
            let untied: Vec<u32> = matching
                .filter(|b| !connectors.iter().any(|c| b.adapter.tied_to(c)))
                .map(|b| b.adapter.bus)
                .collect();
            let twins = edids
                .iter()
                .filter(|e| matches!(e, Some(Ok(other)) if other == edid))
                .count();
            match untied.as_slice() {
                [] => Err(MatchProblem::NoMatchingBus),
                [bus] if twins == 1 => Ok(*bus),
                _ => Err(MatchProblem::Ambiguous(untied)),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::edid::tests::{SAMSUNG_FRAME, other_monitor};

    fn connector(sysfs_name: &str, edid: Option<&[u8]>, ddc_bus: Option<u32>) -> Connector {
        Connector {
            sysfs_name: sysfs_name.into(),
            name: sysfs_name.split_once('-').unwrap().1.into(),
            connected: true,
            edid: edid.map(<[u8]>::to_vec),
            ddc_bus,
            path: PathBuf::from("/sys/devices/gpu/drm").join(sysfs_name),
        }
    }

    fn adapter(bus: u32, parent: &str) -> Adapter {
        Adapter {
            bus,
            name: format!("adapter {bus}"),
            on_display_controller: true,
            path: PathBuf::from(parent).join(format!("i2c-{bus}")),
        }
    }

    fn probed<'a>(adapter: &'a Adapter, edid: Option<&[u8]>) -> ProbedBus<'a> {
        ProbedBus {
            adapter,
            edid: edid.map(|e| Edid::parse(e).unwrap()),
        }
    }

    #[test]
    fn nvidia_style_bus_found_by_edid_alone() {
        // This host: HDMI-A-1 has no ddc link; i2c-2..6 belong to the GPU.
        let hdmi = connector("card0-HDMI-A-1", Some(&SAMSUNG_FRAME), None);
        let adapters: Vec<_> = (2..=6).map(|n| adapter(n, "/sys/devices/gpu")).collect();
        let buses: Vec<_> = adapters
            .iter()
            .map(|a| probed(a, (a.bus == 3).then_some(&SAMSUNG_FRAME[..])))
            .collect();
        assert_eq!(assign(&[&hdmi], &buses), [Ok(3)]);
    }

    #[test]
    fn kernel_link_wins_and_aux_channel_is_second_choice() {
        let dp = connector("card1-DP-4", Some(&SAMSUNG_FRAME), Some(8));
        let hw = adapter(8, "/sys/devices/gpu");
        let aux = adapter(11, "/sys/devices/gpu/drm/card1-DP-4");
        let both = [
            probed(&aux, Some(&SAMSUNG_FRAME)),
            probed(&hw, Some(&SAMSUNG_FRAME)),
        ];
        assert_eq!(assign(&[&dp], &both), [Ok(8)]);
        let aux_only = [probed(&aux, Some(&SAMSUNG_FRAME)), probed(&hw, None)];
        assert_eq!(assign(&[&dp], &aux_only), [Ok(11)]);
    }

    #[test]
    fn linked_bus_with_another_monitor_is_never_used() {
        // The kernel links bus 8, but bus 8 answers with another EDID (or
        // none): the link alone is not enough.
        let dp = connector("card1-DP-4", Some(&SAMSUNG_FRAME), Some(8));
        let hw = adapter(8, "/sys/devices/gpu");
        let other = other_monitor();
        assert_eq!(
            assign(&[&dp], &[probed(&hw, Some(&other))]),
            [Err(MatchProblem::NoMatchingBus)]
        );
        assert_eq!(
            assign(&[&dp], &[probed(&hw, None)]),
            [Err(MatchProblem::NoMatchingBus)]
        );
    }

    #[test]
    fn a_bus_tied_to_another_connector_is_not_borrowed() {
        // HDMI-A-1 (no link) has the same EDID as DP-4, whose link is bus 8.
        let hdmi = connector("card0-HDMI-A-1", Some(&SAMSUNG_FRAME), None);
        let dp = connector("card1-DP-4", Some(&SAMSUNG_FRAME), Some(8));
        let hw = adapter(8, "/sys/devices/gpu");
        assert_eq!(
            assign(&[&hdmi, &dp], &[probed(&hw, Some(&SAMSUNG_FRAME))]),
            [Err(MatchProblem::NoMatchingBus), Ok(8)]
        );
    }

    #[test]
    fn identical_monitors_without_links_are_refused() {
        let left = connector("card0-HDMI-A-1", Some(&SAMSUNG_FRAME), None);
        let right = connector("card0-DP-1", Some(&SAMSUNG_FRAME), None);
        let a = adapter(2, "/sys/devices/gpu");
        let b = adapter(3, "/sys/devices/gpu");
        let buses = [
            probed(&a, Some(&SAMSUNG_FRAME)),
            probed(&b, Some(&SAMSUNG_FRAME)),
        ];
        let ambiguous = Err(MatchProblem::Ambiguous(vec![2, 3]));
        assert_eq!(
            assign(&[&left, &right], &buses),
            [ambiguous.clone(), ambiguous]
        );
        // Even with only one bus answering, two twins cannot share it.
        let one = [probed(&a, Some(&SAMSUNG_FRAME)), probed(&b, None)];
        assert_eq!(
            assign(&[&left, &right], &one),
            [
                Err(MatchProblem::Ambiguous(vec![2])),
                Err(MatchProblem::Ambiguous(vec![2]))
            ]
        );
    }

    #[test]
    fn two_different_monitors_get_their_own_buses() {
        let other = other_monitor();
        let left = connector("card0-HDMI-A-1", Some(&SAMSUNG_FRAME), None);
        let right = connector("card0-DP-1", Some(&other), None);
        let a = adapter(2, "/sys/devices/gpu");
        let b = adapter(3, "/sys/devices/gpu");
        let buses = [probed(&a, Some(&other)), probed(&b, Some(&SAMSUNG_FRAME))];
        assert_eq!(assign(&[&left, &right], &buses), [Ok(3), Ok(2)]);
    }

    #[test]
    fn connectors_without_usable_edid_are_reported() {
        let none = connector("card0-DP-1", None, None);
        let mut corrupt_bytes = SAMSUNG_FRAME;
        corrupt_bytes[30] ^= 0xff;
        let corrupt = connector("card0-DP-2", Some(&corrupt_bytes), None);
        assert_eq!(
            assign(&[&none, &corrupt], &[]),
            [
                Err(MatchProblem::NoConnectorEdid),
                Err(MatchProblem::InvalidConnectorEdid)
            ]
        );
    }
}
