//! Which monitor a `wl_output` is, as the EDID identity dimmer-core keys
//! settings by ([`EdidId`]).
//!
//! cosmic-comp names each `wl_output` after its DRM connector (`HDMI-A-1`) and
//! fills make and model from the EDID, but no unprivileged protocol it offers
//! carries the EDID's serial number except output management. The kernel
//! publishes the very same EDID blob, world-readable, at
//! `/sys/class/drm/card<N>-<connector>/edid`: the bytes cosmic-comp parsed and
//! the bytes a DDC backend reads over I2C. Joining on the connector name and
//! parsing that file with [`EdidId::from_edid`] therefore gives the overlay the
//! same identity as every other part of the dimmer, without guessing from
//! display names.
//!
//! Nothing read here is trusted blindly: the connector must be connected, the
//! EDID must pass dimmer-core's header and checksum checks, reads are bounded,
//! and if two DRM devices both have a connector of that name with different
//! monitors the match is refused as ambiguous. Every failure falls back to a
//! connector-scoped [`OutputId`] that matches no saved settings, so a wrong
//! guess can never apply another monitor's dimming.

use std::{fs, io::Read, path::Path};

use dimmer_core::{EdidError, EdidId, OutputId};

/// Where the kernel lists DRM connectors.
pub const SYSFS_DRM: &str = "/sys/class/drm";

/// The largest EDID the kernel can hold: 256 blocks of 128 bytes.
const MAX_EDID_BYTES: u64 = 256 * 128;
/// `status` holds one short word (`connected`, `disconnected`, `unknown`).
const MAX_STATUS_BYTES: u64 = 64;

/// What one DRM connector directory says about its monitor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectorInfo {
    pub connected: bool,
    pub edid: Vec<u8>,
}

/// The outcome of matching a connector name to an EDID identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Resolution {
    /// Exactly one monitor with a valid EDID is on that connector.
    Edid(EdidId),
    /// No connected connector of that name has an EDID.
    NoEdid,
    /// Several DRM devices have a connected connector of that name, with
    /// different monitors; which one this `wl_output` is cannot be told.
    Ambiguous,
    /// The EDID is there but is not a valid base block.
    Invalid(EdidError),
}

/// The overlay's [`OutputId`] for a monitor: its EDID entry name, the same one
/// dimmer-core's store uses, so the applet can join the two without a table.
#[must_use]
pub fn edid_output_id(edid: &EdidId) -> OutputId {
    OutputId(edid.entry_name())
}

/// The fallback [`OutputId`] for an output whose EDID could not be read. It is
/// stable only while the monitor stays on that connector and never collides
/// with an EDID-derived ID (those start with three capital letters).
#[must_use]
pub fn connector_output_id(connector: &str) -> OutputId {
    OutputId(format!("connector:{connector}"))
}

/// Whether a `/sys/class/drm` entry name is `card<digits>-<connector>`.
/// Compared, never used to build a path, so a hostile connector name such as
/// `../x` matches nothing.
fn is_connector_dir(dir: &str, connector: &str) -> bool {
    let Some(rest) = dir.strip_prefix("card") else {
        return false;
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0 && rest[digits..].strip_prefix('-') == Some(connector)
}

/// Match a connector name against connector directories (name, contents).
#[must_use]
pub fn resolve(
    connector: &str,
    candidates: impl IntoIterator<Item = (String, ConnectorInfo)>,
) -> Resolution {
    if connector.is_empty() {
        return Resolution::NoEdid;
    }
    let mut found: Option<Result<EdidId, EdidError>> = None;
    for (dir, info) in candidates {
        if !is_connector_dir(&dir, connector) || !info.connected || info.edid.is_empty() {
            continue;
        }
        let parsed = EdidId::from_edid(&info.edid);
        match &found {
            None => found = Some(parsed),
            Some(previous) if *previous == parsed => {}
            Some(_) => return Resolution::Ambiguous,
        }
    }
    match found {
        None => Resolution::NoEdid,
        Some(Ok(id)) => Resolution::Edid(id),
        Some(Err(error)) => Resolution::Invalid(error),
    }
}

/// [`resolve`] against the live sysfs tree under `root` (normally
/// [`SYSFS_DRM`]). Small blocking reads of kernel attributes; call it on the
/// overlay's worker thread, never the UI thread. Unreadable entries are skipped.
#[must_use]
pub fn resolve_sysfs(root: &Path, connector: &str) -> Resolution {
    let Ok(entries) = fs::read_dir(root) else {
        return Resolution::NoEdid;
    };
    let candidates = entries.filter_map(Result::ok).filter_map(|entry| {
        let dir = entry.file_name().into_string().ok()?;
        if !is_connector_dir(&dir, connector) {
            return None;
        }
        let path = entry.path();
        let status = read_bounded(&path.join("status"), MAX_STATUS_BYTES)?;
        let edid = read_bounded(&path.join("edid"), MAX_EDID_BYTES)?;
        Some((
            dir,
            ConnectorInfo {
                connected: status.trim_ascii() == b"connected",
                edid,
            },
        ))
    });
    resolve(connector, candidates)
}

fn read_bounded(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(limit)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A synthetic EDID base block (made-up serial; no real device's data).
    pub(crate) fn sample_edid(product: u16, serial: u32) -> Vec<u8> {
        let mut edid = vec![0u8; 128];
        edid[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        // S=19, A=1, M=13, five bits each.
        let packed: u16 = (0x13 << 10) | (0x01 << 5) | 0x0D;
        edid[8..10].copy_from_slice(&packed.to_be_bytes());
        edid[10..12].copy_from_slice(&product.to_le_bytes());
        edid[12..16].copy_from_slice(&serial.to_le_bytes());
        let sum = edid.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        edid[127] = 0u8.wrapping_sub(sum);
        edid
    }

    fn connected(edid: Vec<u8>) -> ConnectorInfo {
        ConnectorInfo {
            connected: true,
            edid,
        }
    }

    #[test]
    fn connector_directory_names() {
        assert!(is_connector_dir("card0-HDMI-A-1", "HDMI-A-1"));
        assert!(is_connector_dir("card12-DP-3", "DP-3"));
        assert!(!is_connector_dir("card0-HDMI-A-11", "HDMI-A-1"));
        assert!(!is_connector_dir("card-HDMI-A-1", "HDMI-A-1"));
        assert!(!is_connector_dir("cardX-HDMI-A-1", "HDMI-A-1"));
        assert!(!is_connector_dir("renderD128", "renderD128"));
        assert!(!is_connector_dir("card0", ""));
        assert!(!is_connector_dir("card0-DP-1", "../card0-DP-1"));
    }

    #[test]
    fn resolves_the_one_connected_monitor() {
        let edid = sample_edid(0x7A8D, 7);
        let resolution = resolve(
            "HDMI-A-1",
            [
                ("card0-DP-1".into(), connected(sample_edid(1, 1))),
                ("card0-HDMI-A-1".into(), connected(edid.clone())),
                ("card1-HDMI-A-2".into(), connected(Vec::new())),
            ],
        );
        let expected = EdidId::from_edid(&edid).unwrap();
        assert_eq!(resolution, Resolution::Edid(expected));
        assert_eq!(
            edid_output_id(&expected),
            OutputId(expected.entry_name()),
            "the overlay's ID is dimmer-core's entry name"
        );
    }

    #[test]
    fn disconnected_or_empty_is_no_edid() {
        let stale = ConnectorInfo {
            connected: false,
            edid: sample_edid(1, 1),
        };
        assert_eq!(
            resolve("HDMI-A-1", [("card0-HDMI-A-1".into(), stale)]),
            Resolution::NoEdid
        );
        assert_eq!(
            resolve(
                "HDMI-A-1",
                [("card0-HDMI-A-1".into(), connected(Vec::new()))]
            ),
            Resolution::NoEdid
        );
        assert_eq!(resolve("", []), Resolution::NoEdid);
    }

    #[test]
    fn same_connector_on_two_gpus_is_ambiguous_unless_same_monitor() {
        let a = sample_edid(1, 1);
        let b = sample_edid(2, 2);
        assert_eq!(
            resolve(
                "DP-1",
                [
                    ("card0-DP-1".into(), connected(a.clone())),
                    ("card1-DP-1".into(), connected(b)),
                ]
            ),
            Resolution::Ambiguous
        );
        assert!(matches!(
            resolve(
                "DP-1",
                [
                    ("card0-DP-1".into(), connected(a.clone())),
                    ("card1-DP-1".into(), connected(a)),
                ]
            ),
            Resolution::Edid(_)
        ));
    }

    #[test]
    fn corrupt_edid_is_invalid_not_another_monitor() {
        let mut edid = sample_edid(1, 1);
        edid[12] ^= 0xFF;
        assert_eq!(
            resolve("DP-1", [("card0-DP-1".into(), connected(edid))]),
            Resolution::Invalid(EdidError::BadChecksum)
        );
    }

    #[test]
    fn fallback_ids_never_look_like_edid_ids() {
        let id = connector_output_id("HDMI-A-1");
        assert!(id.0.starts_with("connector:"));
        assert!(!id.0.as_bytes()[..3].iter().all(u8::is_ascii_uppercase));
    }

    #[test]
    fn sysfs_reader_on_a_fake_tree() {
        let root = std::env::temp_dir().join(format!(
            "dimmer-overlay-sysfs-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let dir = root.join("card0-HDMI-A-1");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("status"), "connected\n").unwrap();
        let edid = sample_edid(0x7A8D, 3);
        fs::write(dir.join("edid"), &edid).unwrap();
        fs::create_dir_all(root.join("card0-DP-1")).unwrap(); // no files: skipped
        let resolution = resolve_sysfs(&root, "HDMI-A-1");
        let missing = resolve_sysfs(&root.join("absent"), "HDMI-A-1");
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            resolution,
            Resolution::Edid(EdidId::from_edid(&edid).unwrap())
        );
        assert_eq!(missing, Resolution::NoEdid);
    }
}
