//! What the kernel says about DRM connectors and I2C adapters, from sysfs.
//! Reading sysfs causes no bus traffic.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// A DRM connector, such as `card0-HDMI-A-1`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Connector {
    /// The sysfs name, `card0-HDMI-A-1`.
    pub sysfs_name: String,
    /// The connector name without the card, `HDMI-A-1`, as Wayland names outputs.
    pub name: String,
    pub connected: bool,
    /// The EDID the kernel read at modeset, if any.
    pub edid: Option<Vec<u8>>,
    /// The bus the driver links as this connector's DDC channel (`ddc`
    /// symlink). Some drivers (NVIDIA's) do not provide it.
    pub ddc_bus: Option<u32>,
    /// Canonical sysfs path, to recognise adapters created under it.
    pub path: PathBuf,
}

/// An I2C adapter, `/sys/bus/i2c/devices/i2c-N`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Adapter {
    pub bus: u32,
    pub name: String,
    /// The adapter hangs off a PCI display controller (class `0x03xxxx`).
    /// Only these are ever opened: `SMBus` controllers, platform I2C (touchpads,
    /// sensors) and memory SPD EEPROMs at 0x50 are never touched.
    pub on_display_controller: bool,
    /// Canonical sysfs path.
    pub path: PathBuf,
}

impl Adapter {
    /// The adapter belongs to `connector`, by the kernel's own links: the
    /// connector's `ddc` symlink names it, or it was created under the
    /// connector (`DisplayPort` AUX channels).
    #[must_use]
    pub fn tied_to(&self, connector: &Connector) -> bool {
        connector.ddc_bus == Some(self.bus) || self.path.starts_with(&connector.path)
    }
}

/// A sysfs tree: `/` on a real system, a fixture directory in tests.
#[derive(Clone, Debug)]
pub struct Sysfs {
    root: PathBuf,
}

impl Sysfs {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Every DRM connector, sorted by sysfs name.
    ///
    /// # Errors
    /// Fails only when `/sys/class/drm` cannot be listed; unreadable
    /// attributes of a single connector are treated as absent.
    pub fn connectors(&self) -> io::Result<Vec<Connector>> {
        let class = self.root.join("sys/class/drm");
        let mut connectors = Vec::new();
        for entry in fs::read_dir(&class)? {
            let entry = entry?;
            let sysfs_name = entry.file_name().to_string_lossy().into_owned();
            let Some(name) = connector_name(&sysfs_name) else {
                continue;
            };
            let dir = entry.path();
            let Ok(status) = fs::read_to_string(dir.join("status")) else {
                continue;
            };
            let edid = fs::read(dir.join("edid")).ok().filter(|e| !e.is_empty());
            let ddc_bus = fs::read_link(dir.join("ddc"))
                .ok()
                .and_then(|target| bus_number(&target.file_name()?.to_string_lossy()));
            connectors.push(Connector {
                name: name.to_owned(),
                sysfs_name,
                connected: status.trim() == "connected",
                edid,
                ddc_bus,
                path: fs::canonicalize(&dir).unwrap_or(dir),
            });
        }
        connectors.sort_by(|a, b| a.sysfs_name.cmp(&b.sysfs_name));
        Ok(connectors)
    }

    /// Every numbered I2C adapter, sorted by bus number.
    ///
    /// # Errors
    /// Fails only when `/sys/bus/i2c/devices` cannot be listed.
    pub fn adapters(&self) -> io::Result<Vec<Adapter>> {
        let devices = self.root.join("sys/bus/i2c/devices");
        let mut adapters = Vec::new();
        for entry in fs::read_dir(&devices)? {
            let entry = entry?;
            let Some(bus) = bus_number(&entry.file_name().to_string_lossy()) else {
                continue; // i2c client devices such as i2c-MSFT8000:00
            };
            let dir = entry.path();
            let name = fs::read_to_string(dir.join("name"))
                .map(|n| n.trim().to_owned())
                .unwrap_or_default();
            let path = fs::canonicalize(&dir).unwrap_or(dir);
            adapters.push(Adapter {
                bus,
                on_display_controller: on_display_controller(&self.root, &path),
                name,
                path,
            });
        }
        adapters.sort_by_key(|a| a.bus);
        Ok(adapters)
    }

    /// Re-read one connector's EDID, to check before a write that the
    /// display behind it is still the one that was matched. No bus traffic.
    #[must_use]
    pub fn connector_edid(&self, sysfs_name: &str) -> Option<Vec<u8>> {
        fs::read(
            self.root
                .join("sys/class/drm")
                .join(sysfs_name)
                .join("edid"),
        )
        .ok()
        .filter(|e| !e.is_empty())
    }
}

/// `card0-HDMI-A-1` -> `HDMI-A-1`; `None` for `card0`, `renderD128`, `version`.
fn connector_name(sysfs_name: &str) -> Option<&str> {
    let rest = sysfs_name.strip_prefix("card")?;
    let (card, name) = rest.split_once('-')?;
    (!card.is_empty() && card.bytes().all(|b| b.is_ascii_digit()) && !name.is_empty())
        .then_some(name)
}

/// `i2c-3` -> 3.
fn bus_number(name: &str) -> Option<u32> {
    name.strip_prefix("i2c-")?.parse().ok()
}

/// Walk up from the adapter to the first ancestor with a PCI `class`
/// attribute; display controllers are base class 0x03.
fn on_display_controller(root: &Path, adapter: &Path) -> bool {
    let stop = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    for dir in adapter.ancestors().skip(1).take(8) {
        if !dir.starts_with(&stop) || dir == stop {
            break;
        }
        if let Ok(class) = fs::read_to_string(dir.join("class")) {
            return u32::from_str_radix(class.trim().trim_start_matches("0x"), 16)
                .is_ok_and(|class| class >> 16 == 0x03);
        }
    }
    false
}

#[cfg(test)]
pub(crate) mod fixture {
    //! A miniature sysfs tree laid out like this project's host: an NVIDIA
    //! GPU whose HDMI connector has no `ddc` link, an AMD iGPU whose
    //! connectors do, an `SMBus` controller and a platform I2C adapter.

    use std::{
        fs,
        os::unix::fs::symlink,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };

    pub struct Tree {
        pub root: PathBuf,
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    impl Tree {
        pub fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "dimmer-ddc-sysfs-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&root);
            for dir in ["sys/class/drm", "sys/bus/i2c/devices"] {
                fs::create_dir_all(root.join(dir)).unwrap();
            }
            Self { root }
        }

        fn write(path: &Path, name: &str, contents: &[u8]) {
            fs::create_dir_all(path).unwrap();
            fs::write(path.join(name), contents).unwrap();
        }

        /// A PCI device with a class attribute; returns its directory.
        pub fn pci(&self, slot: &str, class: &str) -> PathBuf {
            let dir = self.root.join("sys/devices/pci0000:00").join(slot);
            Self::write(&dir, "class", format!("{class}\n").as_bytes());
            dir
        }

        /// An adapter under `parent`, linked from /sys/bus/i2c/devices.
        pub fn adapter(&self, parent: &Path, bus: u32, name: &str) -> PathBuf {
            let dir = parent.join(format!("i2c-{bus}"));
            Self::write(&dir, "name", format!("{name}\n").as_bytes());
            symlink(
                &dir,
                self.root.join(format!("sys/bus/i2c/devices/i2c-{bus}")),
            )
            .unwrap();
            dir
        }

        /// A connector under `gpu`, linked from /sys/class/drm.
        pub fn connector(
            &self,
            gpu: &Path,
            sysfs_name: &str,
            connected: bool,
            edid: &[u8],
            ddc: Option<&Path>,
        ) -> PathBuf {
            let card = sysfs_name.split('-').next().unwrap();
            let dir = gpu.join("drm").join(card).join(sysfs_name);
            let status = if connected {
                "connected\n"
            } else {
                "disconnected\n"
            };
            Self::write(&dir, "status", status.as_bytes());
            Self::write(&dir, "edid", edid);
            if let Some(target) = ddc {
                symlink(target, dir.join("ddc")).unwrap();
            }
            symlink(&dir, self.root.join("sys/class/drm").join(sysfs_name)).unwrap();
            dir
        }

        pub fn set_edid(&self, sysfs_name: &str, edid: &[u8]) {
            fs::write(
                self.root
                    .join("sys/class/drm")
                    .join(sysfs_name)
                    .join("edid"),
                edid,
            )
            .unwrap();
        }

        pub fn set_connected(&self, sysfs_name: &str, connected: bool) {
            let status = if connected {
                "connected\n"
            } else {
                "disconnected\n"
            };
            fs::write(
                self.root
                    .join("sys/class/drm")
                    .join(sysfs_name)
                    .join("status"),
                status,
            )
            .unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{fixture::Tree, *};
    use crate::edid::tests::SAMSUNG_FRAME;

    #[test]
    fn names_parse() {
        assert_eq!(connector_name("card0-HDMI-A-1"), Some("HDMI-A-1"));
        assert_eq!(connector_name("card12-DP-4"), Some("DP-4"));
        assert_eq!(connector_name("card0"), None);
        assert_eq!(connector_name("renderD128"), None);
        assert_eq!(connector_name("version"), None);
        assert_eq!(connector_name("cardX-DP-1"), None);
        assert_eq!(bus_number("i2c-13"), Some(13));
        assert_eq!(bus_number("i2c-MSFT8000:00"), None);
    }

    #[test]
    fn reads_connectors_and_adapters_like_the_host() {
        let tree = Tree::new();
        let nvidia = tree.pci("0000:00:01.1/0000:01:00.0", "0x030000");
        let amd = tree.pci("0000:00:08.1/0000:71:00.0", "0x038000");
        let smbus = tree.pci("0000:00:14.0", "0x0c0500");
        let platform = tree.root.join("sys/devices/platform/AMDI0010:00");
        tree.adapter(&nvidia, 3, "NVIDIA i2c adapter 3 at 1:00.0");
        let amd_hw = tree.adapter(&amd, 8, "AMDGPU DM i2c hw bus 1");
        tree.adapter(&smbus, 14, "SMBus PIIX4 adapter port 0 at 0b00");
        tree.adapter(&platform, 0, "Synopsys DesignWare I2C adapter");
        tree.connector(&nvidia, "card0-HDMI-A-1", true, &SAMSUNG_FRAME, None);
        let dp = tree.connector(&amd, "card1-DP-4", false, b"", Some(&amd_hw));
        tree.adapter(&dp, 11, "AMDGPU DM aux hw bus 1");

        let sysfs = Sysfs::new(&tree.root);
        let connectors = sysfs.connectors().unwrap();
        assert_eq!(connectors.len(), 2);
        let hdmi = &connectors[0];
        assert_eq!(hdmi.name, "HDMI-A-1");
        assert!(hdmi.connected);
        assert_eq!(hdmi.edid.as_deref(), Some(&SAMSUNG_FRAME[..]));
        assert_eq!(hdmi.ddc_bus, None);
        let dp = &connectors[1];
        assert_eq!((dp.name.as_str(), dp.connected), ("DP-4", false));
        assert_eq!((dp.edid.clone(), dp.ddc_bus), (None, Some(8)));

        let adapters = sysfs.adapters().unwrap();
        let summary: Vec<_> = adapters
            .iter()
            .map(|a| (a.bus, a.on_display_controller))
            .collect();
        assert_eq!(
            summary,
            [(0, false), (3, true), (8, true), (11, true), (14, false)]
        );
        let aux = adapters.iter().find(|a| a.bus == 11).unwrap();
        let hw = adapters.iter().find(|a| a.bus == 8).unwrap();
        assert!(aux.tied_to(dp) && hw.tied_to(dp));
        assert!(!aux.tied_to(hdmi));
        assert_eq!(
            sysfs.connector_edid("card0-HDMI-A-1").as_deref(),
            Some(&SAMSUNG_FRAME[..])
        );
        assert_eq!(sysfs.connector_edid("card1-DP-4"), None);
    }

    #[test]
    fn missing_tree_is_an_error_not_a_panic() {
        let sysfs = Sysfs::new("/nonexistent-dimmer-ddc-sysfs");
        assert!(sysfs.connectors().is_err());
        assert!(sysfs.adapters().is_err());
    }
}
