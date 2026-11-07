# dimmer-ddc: DDC/CI hardware brightness over i2c-dev

The hardware backend (R1, R8, R9, R10) behind `dimmer_core::Backend`. It talks
to monitors directly through the kernel's i2c-dev interface (`/dev/i2c-N`):
no root, no ddcutil, no ddc-hi. Access comes from a udev rule and group
(R4). On the development host, i2c-tools' `60-i2c-tools.rules` already grants
the `i2c` group every bus.

**About the Samsung Frame TV:** it does not answer DDC/CI on this host. All 36
read-only queries failed with EIO at the first I2C write (docs/ddc-findings.md),
and the user reports that a Mac's brightness control fails on it too. The
backend lists the TV, matches its bus, probes it once, and reports it
`Unavailable` ("the display did not answer DDC/CI (EIO)") from then on without
further bus traffic. Nothing here is tuned for the Samsung. **No default
comes from a Samsung measurement:** with no successful exchange there was
nothing to measure. The defaults follow the DDC/CI spec and ddcutil's
conventions, and the backend is for ordinary DDC/CI monitors.

## How it works

| Module | Role | I/O |
| --- | --- | --- |
| `sysfs` | DRM connectors (status, EDID, `ddc` link) and I2C adapters | sysfs reads only |
| `edid` | Validate a 128-byte base block; manufacturer, product, monitor name | none |
| `matching` | Assign each connected connector a bus | none |
| `protocol` | Get/Set VCP Feature frames, reply parsing, level ↔ raw mapping | none |
| `transport` | `Bus` trait; `/dev/i2c-N` with `I2C_SLAVE` and read/write | the only `unsafe` (one ioctl) |
| `exchange` | One exchange: spec delays, bounded tries, a deadline | bus |
| `worker` | One thread per matched display: serialized jobs, coalescing, health | bus |

**Never the wrong bus.** Discovery reads the EDID (address 0x50) of each bus on
a PCI display controller. It never touches SMBus, platform I2C or buses the
kernel ties to an empty connector. A bus is assigned only when its EDID is
byte-identical to the connector's. A bus the kernel links to the connector
(`ddc` symlink, DP AUX) wins; otherwise the match must be unique (NVIDIA
gives no links). Identical monitors that can't be told apart are refused,
not guessed (`MatchProblem::Ambiguous`). Before every job the worker checks,
from sysfs and with no bus traffic, that the connector still holds that EDID
(`DisplayChanged` otherwise).

**Output IDs** follow the convention all backends share (docs/applet.md):
`dimmer_core::EdidId::entry_name()` of the connector's EDID (byte-identical to
the bus's for a matched display), else `connector:<name>`. If two connected
monitors have the same entry name, both get their connector ID, so no ID ever
names two displays. A test pins the ID to dimmer-core's `entry_name` for the
recorded Samsung bytes. Note that the Frame changes its EDID product code when
its input is labelled PC (0x7a8d → 0x7a8c), so it shows up as a new monitor.

**Never blocking.** Every call waits at most `call_timeout`, and the worker
stops retrying at the same deadline. A display's I/O runs on its own thread,
so a slow monitor delays nobody else. A level that arrives while another is
still queued replaces it; the replaced caller gets `BackendError::Superseded`,
never `Ok`. A queued level whose callers have all gone away (timed out or
dropped) is not sent. A bus is opened per job and closed afterwards, so no
file descriptor stays open while idle.

**Failing fast, not hammering.** A failed probe (after `max_tries`), or
`max_write_failures` failed writes in a row, marks the display `Unavailable`.
From then on it answers from cache with no I/O. It is probed again only when
it is hotplugged (the connected set or an EDID changed in sysfs), or on
`rescan()` after `reprobe_backoff`. `outputs()` does bus I/O only after a
hotplug, so calling it often is cheap.

## Configuration (`DdcConfig`)

| Setting | Default | Source |
| --- | --- | --- |
| `brightness_code` | `0x10` | MCCS luminance |
| `sleep_multiplier` | 1.0 (0.1–10) | Scales the DDC/CI 1.1 delays: 40 ms write→read, 50 ms after a set; ddcutil's default multiplier |
| `max_tries` | 4 (1–10) | ddcutil's default write-read try count |
| `call_timeout` | 3 s (≤ 60 s) | Above the worst case at multiplier 5 (1.85 s); must cover `Timing::worst_case` |
| `reprobe_backoff` | 30 s | Limits re-probes of a failed display on `rescan()` |
| `max_write_failures` | 2 | Failed writes in a row before `Unavailable` |

`DdcConfig::validate` refuses values out of range, and `DdcBackend::new` validates.

## Coexistence with cosmic-settings-daemon

The stock daemon (cosmic-settings-daemon) writes VCP 0x10 to every
display it can reach through ddc-hi, including when the laptop backlight
keys are used. This backend cannot take hardware-write ownership from it
without replacing a stock component (R3 forbids that). So:

- Last writer wins on the monitor. This backend never re-asserts a level, so
  the two never fight in a loop.
- Its cached value is only a hint. `read_value()` reads the monitor, and the
  applet should re-read when its popup opens to show what the daemon may have
  set.
- I/O is serialized per bus within this process. Across processes each job
  holds an advisory `flock` on `/dev/i2c-N`, which orders only tools that take
  the same lock. ddc-hi does not take it, so the daemon's exchanges can
  interleave with ours. Corrupt replies fail their checksum and are retried;
  a write that collides fails and is reported, never claimed.
- `I2C_SLAVE` (not `_FORCE`) is used, so a bus owned by a kernel driver such
  as ddcci fails with EBUSY instead of being used behind that driver's back.

## Tests and limits

Tests use no hardware. They run against a fixture sysfs tree shaped like the
development host (NVIDIA with no `ddc` links, AMD with links and AUX buses,
SMBus, platform I2C) and scripted buses. Those buses answer with the recorded
Samsung EDID (serial bytes zeroed) and DDC/CI frames written out byte for
byte from the spec, with checksums worked by hand. No frame is a capture from
a working monitor: none was available.

Not verified: a real DDC/CI exchange with any monitor (none answering was
connected), a real `flock` contention, and DP-MST.
