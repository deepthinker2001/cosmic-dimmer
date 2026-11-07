# Frame TV DDC/CI investigation

## Decision and scope

**Current configuration: no working DDC/CI communication demonstrated. Overall
DDC feasibility: undetermined, pending approved configuration/TV experiments.**
Do not interpret this as proof that the TV cannot support DDC/CI. No VCP value,
capabilities string, successful round-trip time, or safe hardware floor has been
obtained. This is a partial investigation, not the issue's final acceptance result.

Measured on November 5, 2026 (America/Denver). Only read queries were issued;
DDC read queries necessarily send protocol request bytes through i2c-dev. No
`setvcp`, TV setting change, package installation, module reload, initramfs
rebuild, reboot, or host configuration edit was performed. The PM reserved
`/dev/i2c-3` exclusively for this spike; re-confirm ownership before resuming.
Do not overlap ddcutil with a ddcci probe or another brightness backend.

## Independently observed baseline

| Item | Observation |
| --- | --- |
| OS | Pop!_OS 24.04 LTS |
| Kernel (`uname -sr`) | Linux 7.2.6-76070206-generic |
| ddcutil | 1.4.1, built with libdrm and USB support |
| Loaded NVIDIA (`/proc/driver/nvidia/version`) | **NVIDIA UNIX Open Kernel Module** 595.104.02 |
| Module metadata (`modinfo -F version nvidia`) | 595.104.02 |
| Active `/proc/driver/nvidia/params` | `RegistryDwords: ""`, `RegistryDwordsPerDevice: ""` |
| Installed parameter names (`modinfo -p nvidia`) | `NVreg_RegistryDwords`, `NVreg_RegistryDwordsPerDevice` exist (charp) |
| `/etc/modprobe.d` search | No match for `RMUseSwI2c`, `RMI2cSpeed`, or `NVreg_RegistryDwords` in readable files |
| HDMI connector | `card0-HDMI-A-1/status`: `connected` |
| Connector's cached EDID | 256 bytes, manufacturer SAM, product `0x7a8d`, valid 128-byte base checksum |
| `/dev/i2c-3` | character device 89:3, `root:i2c`, mode `crw-rw----+`; current user can read and write it |
| User groups | No `i2c` membership; effective access exists, exact ACL mechanism not inspected |

The issue described a proprietary-driver HDMI baseline. The **loaded open kernel
module** is an important discrepancy: do not assume advice for the proprietary
module or a different release applies unchanged. The visible connector `ddc`
path did not resolve to an adapter-number target with `readlink -f`; target
selection also relies on the issue's bus identity and ddcutil identifying a
monitor on that bus. Cached EDID identity is not proof of working DDC/CI.

## Read-only attempts and measured failure latency

Each operation below was a separate process, run sequentially with this template:

```sh
timeout --signal=TERM --kill-after=2s 20s \
  ddcutil --bus 3 --disable-capabilities-cache \
  --sleep-multiplier MULTIPLIER --maxtries TRIES --stats all COMMAND
```

`COMMAND` was `capabilities`, `getvcp 10`, `getvcp 12`, or `getvcp D6`.
VCP numbers are hexadecimal: `10` = brightness/luminance, `12` = contrast,
`D6` = power mode (read only here). `--maxtries` sets write-only, write/read,
and multipart retry limits respectively. The installed help verifies these
options. No `--force-slave-address`, broad bus scan, or manufacturer-code sweep
was used. HOME, XDG_CONFIG_HOME and XDG_CACHE_HOME were pointed to an isolated
scratch directory to avoid user configuration and caches.

| Multiplier | Tries | capabilities | getvcp 10 | getvcp 12 | getvcp D6 |
| --- | --- | --- | --- | --- | --- |
| 1 | 2,2,2 | 64 ms | 57 ms | 61 ms | 44 ms |
| 2 | 4,4,4 | 54 ms | 59 ms | 54 ms | 49 ms |
| 5 | 4,4,4 | 55 ms | 51 ms | 53 ms | 54 ms |

These are **single whole-process failure latencies**, measured with Python's
`time.monotonic()` around `subprocess.run`, including process startup. They are
**not successful VCP round-trip measurements**, percentiles, or timeout sizing
data. All 12 operations exited 1 without hitting the 20-second harness limit.
All reported:

```text
DDC communication failed for monitor on bus /dev/i2c-3
Total successful attempts:          0
Failed due to max tries exceeded:   1
```

For tries `2,2,2`, statistics reported two EIO errors wrapped by retry and one
`DDCRC_RETRIES`; for `4,4,4`, four EIO errors and one `DDCRC_RETRIES`.
Example: multiplier 2 `getvcp 10` reported six ioctl writes (14.678215 ms total),
two ioctl reads (18.635526 ms total), and 56.623059 ms internal elapsed time.
Those calls can include detection/validation and EDID work; they are not evidence
that the requested VCP exchange reached the TV. Failure happens at communication
validation, so there is no basis to declare each requested VCP unsupported.
Increasing sleep/retry settings did not restore communication.

One attempted `ddcutil --bus 3 ... detect` exited 1 in 2 ms with
`detect does not support explicit display option`. This was an invalid invocation,
not a hardware result; it is excluded from the 12 queries above. Unrestricted
`detect` was not retried, to avoid touching buses outside the reservation.

### VCP support conclusion

- `0x10`, `0x12`, `0xD6`: values/ranges unknown; no successful communication.
- Capabilities/MCCS version: unknown.
- Samsung-specific codes: **none verified**. No speculative vendor-code sweep
  while baseline communication fails. Once capabilities work, inspect advertised
  vendor codes and query only reviewed read-safe codes individually. Do not infer
  that another Samsung model's codes apply here.

## NVIDIA configuration experiment — proposed, NOT performed

Candidate supplied by the issue:

```text
options nvidia NVreg_RegistryDwords="RMUseSwI2c=0x01;RMI2cSpeed=100"
```

The intended experiment requests software I2C and an I2C speed setting. This
investigation has **not verified that these registry keys are implemented or
honored by this loaded open-module release**, or that they fix this TV.
`modinfo` exposing the container string parameter is not proof that its individual
keys work. Do not call the candidate an established fix.

Approval-gated procedure (requires a separate explicit user go-ahead):

1. Record current TV/input settings, display topology, active NVIDIA version and
   registry string. Inspect existing module-option definitions in modprobe.d,
   boot configuration and initramfs; preserve any unrelated registry keys.
2. Arrange physical access, the TV remote and a boot recovery path. Review the
   proposed file and rollback before writing anything; do not unload the live GPU.
3. If no conflicting definition exists, add the single line above to a dedicated
   `/etc/modprobe.d/samsung-dimmer-ddc.conf`. Never overwrite an existing file or
   existing registry options blindly. Changes to the actual path require approval.
4. On this Pop!_OS host, proposed rebuild command is
   `sudo update-initramfs -u -k "$(uname -r)"` after verifying that it updates the
   image used by the selected boot entry. Capture exit status/output. Do not
   reboot after a failed rebuild. Reboot itself needs the user's approval.
5. After reboot, read `/proc/driver/nvidia/version` and `params`; confirm the
   expected string is active, re-identify the SAM `0x7a8d` connector/bus (numbers
   can change), reserve that bus again, and repeat the exact read-only matrix.
   A populated string alone does not establish that the keys took effect.
6. Rollback: restore the preserved file/registry definition, or remove only the
   newly created file if it did not exist before; rebuild the same booted-kernel
   initramfs and perform an approved reboot. Verify the original active settings
   and normal display operation. Keep an independently usable recovery boot entry.

No post-parameter result exists yet. If configuration is insufficient and a fix
requires editing NVIDIA, kernel, ddcci-driver-linux or ddcutil code, stop affected
work, report the evidence to the PM, and ask the user to create a separate project
(R14). No such upstream code change has yet been established as necessary.

## TV-side experiments — proposed, NOT performed

Current menu values and firmware version are unknown. Ask the user to report
these before changing anything; exact menus vary with firmware/region. None of
the following is established as enabling DDC on this model:

| Candidate | Controlled experiment, after individual approval |
| --- | --- |
| HDMI input label PC | Record original input label; try PC, repeat read queries, then restore |
| Game Mode | Record Off/On/Auto; change only this setting, re-query, restore |
| Anynet+ (HDMI-CEC) | Record original state; try Off, re-query, restore; CEC is not DDC/CI |
| Eco/energy saving/ambient brightness | Record enabled options; disable one at a time, re-query, restore |
| DDC/CI menu toggle, if present | Record whether such a menu actually exists; do not assume it does |

Keep the HDMI cable/port and all other settings fixed within each comparison.
Record whether an input transition, picture-mode reset, or renegotiation occurred.
Stop if the picture disappears and recover with the remote, not blind VCP writes.

## Timeout and hardware-floor handoff

**No measured successful-operation timeout recommendation is available.** The
observed 44–64 ms failures only describe the current early-error path. The
20-second watchdog was a conservative diagnostic guard, not a proposed UI/backend
latency target. A process deadline does not guarantee immediate cancellation of
an uninterruptible kernel I/O; after any timeout, confirm the worker is gone and
bus use has ended before starting another client.

Once communication succeeds, collect at least 20 sequential samples per supported
read code, plus separate capabilities samples, retaining raw output, settings,
retry counts, exit statuses and monotonic durations. Report min/median/p95/max,
success rate, retry-heavy failures, and cold versus warm discovery separately.
Test disconnected/asleep behavior only with specific approval. Use measured
budgets in an off-UI-thread, per-bus-serialized worker with bounded retries,
coalesced updates and recovery/backoff; do not choose a short timeout from EIOs.

**VCP brightness 0 darkness: unmeasured. Hardware floor: unknown.** No brightness
write was attempted. Never treat zero as safe just because many monitors retain
some light there. A future approved test must first establish reliable brightness
read/write/restore at a visible level, record the original value, turn software
alpha dimming off, keep the remote and a physical observer ready, and agree on
recovery if DDC restoration fails. Lower brightness in approved small steps;
stop before the panel becomes unusable. Testing zero needs a separate explicit
approval and must be skipped if recovery/visibility is uncertain. Record OSD
value, returned VCP value, picture/eco mode, ambient lighting, visible-panel
readability and a luminance measurement if equipment is available. Do not write
`D6`, save settings to NVRAM, or change power mode in this experiment.

## After Anynet+ off (2026-11-05, 20:39 MST)

The user turned Anynet+ (HDMI-CEC) off on the TV at about 20:38. Nothing else
was changed: no NVIDIA option, no other TV setting, no host configuration. The
exact matrix above was re-run **once**, unchanged (same harness, `--bus 3`,
`--disable-capabilities-cache`, isolated HOME/XDG directories in a scratch
folder, 20-second watchdog, sequential processes), starting 20:39:31 MST.
Read-only: no `setvcp`, no `detect`, no other bus.

| Multiplier | Tries | capabilities | getvcp 10 | getvcp 12 | getvcp D6 |
| --- | --- | --- | --- | --- | --- |
| 1 | 2,2,2 | EIO, 60 ms | EIO, 48 ms | EIO, 60 ms | EIO, 49 ms |
| 2 | 4,4,4 | EIO, 54 ms | EIO, 49 ms | EIO, 65 ms | EIO, 68 ms |
| 5 | 4,4,4 | EIO, 55 ms | EIO, 61 ms | EIO, 51 ms | EIO, 59 ms |

All 12 exited 1 with `DDC communication failed for monitor on bus /dev/i2c-3`,
`Total successful attempts: 0`, `Failed due to max tries exceeded: 1`; the error
counts are again two EIOs per try budget of 2 and four per budget of 4, plus one
`DDCRC_RETRIES`. No capabilities string or VCP value was obtained.

One detail the earlier table did not record: with multiplier 2, `getvcp 10`
reported **zero** DDC sleep events (`SE_WRITE_TO_READ 0`, `SE_POST_WRITE 0`),
six ioctl writes, two ioctl reads and 46 ms internal elapsed time. ddcutil's
write-to-read sleep is where the sleep multiplier acts; none happened, which
is consistent with the exchange failing at its first I2C write rather than
from too short a delay. That is why changing the multiplier makes no
difference to these timings. It is an inference from ddcutil's counters, not a
bus-level trace.

**Result: turning Anynet+ off did not restore DDC/CI on this host.** The
NVIDIA option experiment and the other TV-side candidates remain untried.

## After input labelled PC (2026-11-05, 21:02 MST)

At about 20:58 the user re-plugged the HDMI cable into the NVIDIA card; the
TV asked whether the source is a PC and the user confirmed, so the input is
now labelled PC. Anynet+ stayed off. No other change.

**Bus re-identified first**, read-only: an EDID read (offset write of one
byte, then a 128-byte read at 0x50) on the NVIDIA adapters `i2c-2` to
`i2c-6` only, no other bus. `i2c-3` returned an EDID byte-identical to
`card0-HDMI-A-1`'s; the other four returned EIO (nothing connected). So the
Samsung is still on **`/dev/i2c-3`**.

**The EDID changed with the PC label:** the product code is now **`0x7a8c`**
(it was `0x7a8d`); manufacturer still SAM. The TV presents a different EDID
for a PC-labelled input. Anything keyed by EDID identity (per-monitor
settings) sees a new monitor after this change.

The exact matrix was then run once more on bus 3, unchanged, starting
21:02:53 MST:

| Multiplier | Tries | capabilities | getvcp 10 | getvcp 12 | getvcp D6 |
| --- | --- | --- | --- | --- | --- |
| 1 | 2,2,2 | EIO, 63 ms | EIO, 54 ms | EIO, 54 ms | EIO, 48 ms |
| 2 | 4,4,4 | EIO, 57 ms | EIO, 59 ms | EIO, 57 ms | EIO, 53 ms |
| 5 | 4,4,4 | EIO, 55 ms | EIO, 58 ms | EIO, 58 ms | EIO, 59 ms |

All 12 exited 1 with `DDC communication failed for monitor on bus /dev/i2c-3`
and `Total successful attempts: 0`; error counts as before (two EIOs per try
budget of 2, four per budget of 4, one `DDCRC_RETRIES` each). Multiplier 2
`getvcp 10` again logged zero write-to-read sleeps. No capabilities string or
VCP value.

**Result: the PC input label did not restore DDC/CI either.** With the Mac
Mini's brightness control also not working on this TV (user report, 20:50),
the evidence points to the TV not implementing DDC/CI. The NVIDIA option
experiment is the only candidate left untried.

## Outstanding acceptance items

- User-approved NVIDIA parameter experiment and verified post-boot DDC outcome.
- User-reported baseline TV settings, then approved one-variable comparisons.
- Successful capabilities/VCP replies, verified vendor codes if any, and repeated
  success/failure latency measurements (or a documented configuration-only no-go).
- Separately approved brightness-floor measurement with reliable recovery.
- Final yes/no feasibility and backend code/timing/floor handoff. Present evidence
  is insufficient for that binary conclusion; do not close the hardware spike on
  the strength of this partial report.
