//! Backend tests: a fixture sysfs tree shaped like this project's host and
//! scripted buses answering with recorded EDID bytes and DDC/CI frames.

use std::sync::Arc;

use super::*;
use crate::{
    edid::tests::{SAMSUNG_FRAME, other_monitor},
    protocol::{
        DDC_ADDRESS, EDID_ADDRESS, get_vcp_request, set_vcp_request,
        tests::REPLY_BRIGHTNESS_50_OF_100,
    },
    sysfs::fixture::Tree,
    transport::fake::{FakeOpener, Gate, Step},
};

const EIO: i32 = 5;
/// Output IDs of the two fixture monitors: `EdidId::entry_name`.
/// The Samsung fingerprints its name and size; the other monitor has no name,
/// so its product code is fingerprinted instead.
const SAM: &str = "SAM-276723A9-00000000";
const DEL: &str = "DEL-BE5A941F-00000007";
const ENXIO: i32 = 6;

/// NVIDIA GPU (no `ddc` links) with the Samsung on `HDMI-A-1` and another
/// monitor on `DP-1`; buses 2..=4 on the GPU; an `SMBus` and a platform
/// adapter that must never be opened.
struct Host {
    tree: Tree,
    opener: FakeOpener,
    backend: DdcBackend,
}

fn edid_steps(edid: Option<&[u8]>) -> Vec<Step> {
    match edid {
        Some(bytes) => vec![
            Step::Write(EDID_ADDRESS, vec![0], None),
            Step::Read(EDID_ADDRESS, Ok(bytes.to_vec())),
        ],
        None => vec![Step::Write(EDID_ADDRESS, vec![0], Some(ENXIO))],
    }
}

fn get_ok() -> [Step; 2] {
    [
        Step::Write(DDC_ADDRESS, get_vcp_request(0x10).to_vec(), None),
        Step::Read(DDC_ADDRESS, Ok(REPLY_BRIGHTNESS_50_OF_100.to_vec())),
    ]
}

fn get_eio() -> Step {
    Step::Write(DDC_ADDRESS, get_vcp_request(0x10).to_vec(), Some(EIO))
}

fn set(value: u16) -> Step {
    Step::Write(DDC_ADDRESS, set_vcp_request(0x10, value).to_vec(), None)
}

fn config(root: &std::path::Path) -> DdcConfig {
    DdcConfig {
        sleep_multiplier: 0.1,
        max_tries: 2,
        call_timeout: Duration::from_millis(500),
        sysfs_root: root.to_path_buf(),
        dev_root: PathBuf::from("/nonexistent"),
        ..DdcConfig::default()
    }
}

impl Host {
    fn new() -> Self {
        Self::with(|c| c)
    }

    fn with(adjust: impl FnOnce(DdcConfig) -> DdcConfig) -> Self {
        let tree = Tree::new();
        let gpu = tree.pci("0000:01:00.0", "0x030000");
        let smbus = tree.pci("0000:00:14.0", "0x0c0500");
        for bus in 2..=4 {
            tree.adapter(&gpu, bus, &format!("NVIDIA i2c adapter {bus} at 1:00.0"));
        }
        tree.adapter(&smbus, 14, "SMBus PIIX4 adapter port 0 at 0b00");
        tree.adapter(
            &tree.root.join("sys/devices/platform/AMDI0010:00"),
            0,
            "Synopsys DesignWare I2C adapter",
        );
        tree.connector(&gpu, "card0-HDMI-A-1", true, &SAMSUNG_FRAME, None);
        tree.connector(&gpu, "card0-DP-1", true, &other_monitor(), None);
        tree.connector(&gpu, "card0-DP-2", false, b"", None);

        let opener = FakeOpener::default();
        let backend =
            DdcBackend::with_opener(adjust(config(&tree.root)), Arc::new(opener.clone())).unwrap();
        Self {
            tree,
            opener,
            backend,
        }
    }

    /// Script one scan's EDID reads: bus 2 the other monitor, bus 3 the
    /// Samsung, bus 4 nothing.
    fn script_scan(&self) {
        self.opener.push(2, edid_steps(Some(&other_monitor())));
        self.opener.push(3, edid_steps(Some(&SAMSUNG_FRAME)));
        self.opener.push(4, edid_steps(None));
    }

    fn opens(&self, bus: u32) -> usize {
        self.opener.script(bus).lock().unwrap().opens
    }

    fn all_consumed(&self) -> bool {
        (2..=4).all(|bus| self.opener.remaining(bus) == 0)
    }
}

fn id(name: &str) -> OutputId {
    OutputId(name.into())
}

fn level(percent: u8) -> Level {
    Level::new(percent).unwrap()
}

#[tokio::test]
async fn discovery_matches_by_edid_and_never_opens_other_adapters() {
    let host = Host::new();
    host.script_scan();
    let outputs = host.backend.outputs().await.unwrap();
    assert_eq!(
        outputs,
        [
            Output {
                id: id(DEL),
                name: "DEL 1234 (DP-1)".into()
            },
            Output {
                id: id(SAM),
                name: "SAMSUNG (HDMI-A-1)".into()
            },
        ]
    );
    assert!(host.all_consumed());
    assert_eq!(host.opens(0), 0, "platform I2C never opened");
    assert_eq!(host.opens(14), 0, "SMBus never opened");

    let displays = host.backend.displays();
    let buses: Vec<_> = displays
        .iter()
        .map(|d| (d.id.0.as_str(), d.connector.as_str(), d.bus))
        .collect();
    assert_eq!(
        buses,
        [
            (DEL, "card0-DP-1", Some(2)),
            (SAM, "card0-HDMI-A-1", Some(3))
        ]
    );
    let samsung = displays[1].identity.as_ref().unwrap();
    assert_eq!((samsung.manufacturer(), samsung.serial()), ("SAM", 0));
    // Discovery itself sends no DDC/CI traffic.
    assert!(displays.iter().all(|d| d.health == Health::Unprobed));
}

#[tokio::test]
async fn repeated_outputs_calls_do_no_bus_io_without_hotplug() {
    let host = Host::new();
    host.script_scan();
    let first = host.backend.outputs().await.unwrap();
    for _ in 0..20 {
        assert_eq!(host.backend.outputs().await.unwrap(), first);
    }
    assert_eq!((host.opens(2), host.opens(3), host.opens(4)), (1, 1, 1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_monitors_are_set_independently_and_at_once() {
    let host = Host::new();
    host.script_scan();
    host.backend.outputs().await.unwrap();
    let gate = Arc::new(Gate::default());
    // DP-1 is held mid-probe; HDMI-A-1 must still complete meanwhile.
    host.opener.push(2, [Step::Gate(gate.clone())]);
    host.opener.push(2, get_ok());
    host.opener.push(2, [set(70)]);
    host.opener.push(3, get_ok());
    host.opener.push(3, [set(25)]);

    let slow = tokio::spawn({
        let backend = host.backend.clone();
        async move { backend.set_level(&id(DEL), level(70)).await }
    });
    gate.wait_until_held();
    assert_eq!(host.backend.set_level(&id(SAM), level(25)).await, Ok(()));
    gate.open();
    assert_eq!(slow.await.unwrap(), Ok(()));

    let values: Vec<_> = host
        .backend
        .displays()
        .into_iter()
        .map(|d| d.health)
        .collect();
    assert_eq!(
        values,
        [
            Health::Ready(VcpValue {
                current: 70,
                maximum: 100
            }),
            Health::Ready(VcpValue {
                current: 25,
                maximum: 100
            }),
        ]
    );
    assert!(host.all_consumed());
}

#[tokio::test]
async fn eio_display_is_unavailable_fast_while_the_other_works() {
    let host = Host::new();
    host.script_scan();
    host.backend.outputs().await.unwrap();
    host.opener.push(3, [get_eio(), get_eio()]);
    host.opener.push(2, get_ok());

    let err = host.backend.capabilities(&id(SAM)).await.unwrap_err();
    let BackendError::Unavailable(reason) = err else {
        panic!("expected Unavailable, got {err:?}");
    };
    assert!(reason.contains("EIO"), "{reason}");
    assert_eq!(
        host.backend.capabilities(&id(DEL)).await,
        Ok(Capabilities {
            kind: LevelKind::Hardware,
            writable: true
        })
    );

    // From now on the Samsung answers from cache: no traffic, no waiting.
    let traffic = host.opener.log(3).len();
    let start = Instant::now();
    for percent in 1..=100 {
        assert!(matches!(
            host.backend.set_level(&id(SAM), level(percent)).await,
            Err(BackendError::Unavailable(_))
        ));
    }
    assert!(host.backend.capabilities(&id(SAM)).await.is_err());
    assert!(start.elapsed() < Duration::from_millis(500));
    assert_eq!(host.opener.log(3).len(), traffic);
}

#[tokio::test]
async fn failed_display_is_reprobed_on_hotplug_and_after_backoff_only() {
    let host = Host::new();
    host.script_scan();
    host.backend.outputs().await.unwrap();
    host.opener.push(3, [get_eio(), get_eio()]);
    assert!(host.backend.capabilities(&id(SAM)).await.is_err());

    // A forced rescan within the backoff re-reads EDIDs but does not
    // re-probe DDC on the failed display.
    host.script_scan();
    host.backend.rescan().await.unwrap();
    assert!(host.backend.capabilities(&id(SAM)).await.is_err());
    assert!(host.all_consumed());

    // Hotplug: the TV is unplugged (one scan sees it gone) and plugged
    // back; it is probed afresh and now answers.
    host.tree.set_connected("card0-HDMI-A-1", false);
    host.opener.push(2, edid_steps(Some(&other_monitor())));
    host.opener.push(3, edid_steps(None));
    host.opener.push(4, edid_steps(None));
    let outputs = host.backend.outputs().await.unwrap();
    assert_eq!(outputs.len(), 1);
    host.tree.set_connected("card0-HDMI-A-1", true);
    host.script_scan();
    host.backend.outputs().await.unwrap();
    host.opener.push(3, get_ok());
    assert!(host.backend.capabilities(&id(SAM)).await.is_ok());
    assert!(host.all_consumed());
}

#[tokio::test]
async fn rescan_after_backoff_reprobes() {
    let host = Host::with(|c| DdcConfig {
        reprobe_backoff: Duration::ZERO,
        ..c
    });
    host.script_scan();
    host.backend.outputs().await.unwrap();
    host.opener.push(3, [get_eio(), get_eio()]);
    assert!(host.backend.capabilities(&id(SAM)).await.is_err());
    host.script_scan();
    host.backend.rescan().await.unwrap();
    host.opener.push(3, get_ok());
    assert!(host.backend.capabilities(&id(SAM)).await.is_ok());
}

#[tokio::test]
async fn monitor_swapped_on_a_connector_moves_to_the_right_bus() {
    let host = Host::new();
    host.script_scan();
    host.backend.outputs().await.unwrap();
    // The two monitors swap connectors: DP-1 now shows the Samsung, whose
    // bus is still 3; HDMI-A-1 the other, on bus 2.
    host.tree.set_edid("card0-HDMI-A-1", &other_monitor());
    host.tree.set_edid("card0-DP-1", &SAMSUNG_FRAME);
    host.script_scan();
    host.backend.outputs().await.unwrap();
    let buses: Vec<_> = host
        .backend
        .displays()
        .into_iter()
        .map(|d| (d.id.0, d.connector, d.bus))
        .collect();
    assert_eq!(
        buses,
        [
            (DEL.into(), "card0-HDMI-A-1".into(), Some(2)),
            (SAM.into(), "card0-DP-1".into(), Some(3))
        ]
    );
}

#[tokio::test]
async fn unmatched_connector_is_listed_but_unavailable() {
    let host = Host::new();
    // Bus 3 does not return the Samsung's EDID (say, an unreadable bus).
    host.opener.push(2, edid_steps(Some(&other_monitor())));
    host.opener.push(3, edid_steps(None));
    host.opener.push(4, edid_steps(None));
    let outputs = host.backend.outputs().await.unwrap();
    assert_eq!(outputs.len(), 2);
    let err = host.backend.capabilities(&id(SAM)).await.unwrap_err();
    assert_eq!(
        err,
        BackendError::Unavailable("no I2C bus returned this monitor's EDID".into())
    );
    assert!(matches!(
        host.backend.set_level(&id(SAM), Level::FULL).await,
        Err(BackendError::Unavailable(_))
    ));
}

#[tokio::test]
async fn unknown_outputs_and_calls_before_a_scan_fail_cleanly() {
    let host = Host::new();
    assert_eq!(
        host.backend.set_level(&id(SAM), Level::FULL).await,
        Err(BackendError::UnknownOutput)
    );
    host.script_scan();
    host.backend.outputs().await.unwrap();
    assert_eq!(
        host.backend.capabilities(&id("HDMI-A-9")).await,
        Err(BackendError::UnknownOutput)
    );
}

#[tokio::test]
async fn a_stuck_display_times_out_the_call_not_the_caller() {
    let host = Host::new();
    host.script_scan();
    host.backend.outputs().await.unwrap();
    let gate = Arc::new(Gate::default());
    host.opener.push(3, [Step::Gate(gate.clone())]);
    host.opener.push(3, get_ok());
    let start = Instant::now();
    assert_eq!(
        host.backend.set_level(&id(SAM), level(10)).await,
        Err(BackendError::TimedOut)
    );
    let waited = start.elapsed();
    assert!(waited >= Duration::from_millis(500) && waited < Duration::from_secs(2));

    // When the display recovers, the abandoned level is not sent late: the
    // probe finishes, then only the next request's read reaches the bus.
    host.opener.push(3, get_ok());
    gate.open();
    assert!(host.backend.read_value(&id(SAM)).await.is_ok());
    assert!(host.all_consumed());
    assert!(
        host.opener.log(3).iter().all(|l| !l.contains("[51, 84,")),
        "no Set VCP after the caller timed out"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn superseded_moves_are_errors_never_successes() {
    let host = Host::new();
    host.script_scan();
    host.backend.outputs().await.unwrap();
    let gate = Arc::new(Gate::default());
    host.opener.push(3, get_ok());
    host.opener
        .push(3, [Step::Gate(gate.clone()), set(10), set(90)]);

    let spawn_set = |percent| {
        let backend = host.backend.clone();
        tokio::spawn(async move { backend.set_level(&id(SAM), level(percent)).await })
    };
    let first = spawn_set(10);
    gate.wait_until_held();
    let middle = spawn_set(50);
    // Let the middle request reach the worker before the last one.
    while host.backend.inner.inventory().displays[SAM]
        .matched
        .as_ref()
        .map(Worker::pending_write)
        .ok()
        .flatten()
        != Some(level(50))
    {
        tokio::task::yield_now().await;
    }
    let last = spawn_set(90);
    while host.backend.inner.inventory().displays[SAM]
        .matched
        .as_ref()
        .map(Worker::pending_write)
        .ok()
        .flatten()
        != Some(level(90))
    {
        tokio::task::yield_now().await;
    }
    gate.open();
    assert_eq!(first.await.unwrap(), Ok(()));
    assert_eq!(middle.await.unwrap(), Err(BackendError::Superseded));
    assert_eq!(last.await.unwrap(), Ok(()));
    assert!(host.all_consumed());
}

#[test]
fn config_defaults_are_valid_and_bad_values_refused() {
    let defaults = DdcConfig::default();
    assert_eq!(defaults.validate(), Ok(()));
    assert_eq!(defaults.brightness_code, 0x10);
    // The default timeout covers the slowest setting the spike tried.
    let slow = DdcConfig {
        sleep_multiplier: 5.0,
        ..DdcConfig::default()
    };
    assert_eq!(slow.validate(), Ok(()));

    for bad in [
        DdcConfig {
            sleep_multiplier: f32::NAN,
            ..DdcConfig::default()
        },
        DdcConfig {
            sleep_multiplier: 0.0,
            ..DdcConfig::default()
        },
        DdcConfig {
            max_tries: 0,
            ..DdcConfig::default()
        },
        DdcConfig {
            max_write_failures: 0,
            ..DdcConfig::default()
        },
        DdcConfig {
            sleep_multiplier: 10.0,
            ..DdcConfig::default()
        },
        DdcConfig {
            call_timeout: Duration::from_secs(600),
            ..DdcConfig::default()
        },
    ] {
        assert!(bad.validate().is_err(), "{bad:?}");
        assert!(DdcBackend::new(bad).is_err());
    }
}

#[test]
fn errors_map_to_the_contract() {
    assert_eq!(
        BackendError::from(DdcError::TimedOut),
        BackendError::TimedOut
    );
    assert_eq!(
        BackendError::from(DdcError::UnsupportedCode(0x10)),
        BackendError::Unsupported
    );
    let eio = DdcError::Io {
        op: "DDC request write",
        errno: Some(EIO),
        detail: "Input/output error (os error 5)".into(),
    };
    assert_eq!(
        BackendError::from(eio),
        BackendError::Unavailable(
            "DDC request write: the display did not answer DDC/CI (EIO)".into()
        )
    );
}

#[test]
fn output_ids_are_dimmer_core_entry_names_of_the_same_bytes() {
    // The pairing contract with the overlay backend and the applet.
    for bytes in [SAMSUNG_FRAME, other_monitor()] {
        let expected = dimmer_core::EdidId::from_edid(&bytes).unwrap().entry_name();
        let tree = Tree::new();
        let gpu = tree.pci("0000:01:00.0", "0x030000");
        tree.connector(&gpu, "card0-HDMI-A-1", true, &bytes, None);
        let connectors = Sysfs::new(&tree.root).connectors().unwrap();
        let connected: Vec<&Connector> = connectors.iter().collect();
        assert_eq!(output_ids(&connected), [OutputId(expected)]);
    }
    assert_eq!(
        dimmer_core::EdidId::from_edid(&SAMSUNG_FRAME)
            .unwrap()
            .entry_name(),
        SAM
    );
}

#[tokio::test]
async fn twins_and_connectors_without_edid_get_connector_ids() {
    let tree = Tree::new();
    let gpu = tree.pci("0000:01:00.0", "0x030000");
    tree.adapter(&gpu, 2, "NVIDIA i2c adapter 1 at 1:00.0");
    tree.connector(&gpu, "card0-DP-1", true, &SAMSUNG_FRAME, None);
    tree.connector(&gpu, "card0-DP-2", true, &SAMSUNG_FRAME, None);
    tree.connector(&gpu, "card0-HDMI-A-1", true, b"", None);
    let opener = FakeOpener::default();
    opener.push(2, edid_steps(Some(&SAMSUNG_FRAME)));
    let backend = DdcBackend::with_opener(config(&tree.root), Arc::new(opener.clone())).unwrap();
    let ids: Vec<_> = backend
        .outputs()
        .await
        .unwrap()
        .into_iter()
        .map(|o| o.id.0)
        .collect();
    assert_eq!(
        ids,
        ["connector:DP-1", "connector:DP-2", "connector:HDMI-A-1"]
    );
    // Identical twins on one bus: refused, not guessed.
    for id in &ids {
        assert!(matches!(
            backend.capabilities(&OutputId(id.clone())).await,
            Err(BackendError::Unavailable(_))
        ));
    }
}
