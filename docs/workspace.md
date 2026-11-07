# Standalone applet workspace

The root `Cargo.toml` is the workspace for the standalone COSMIC applet (R3):
a separately installable panel applet, built against the libcosmic that the
installed Pop!_OS COSMIC (epoch-1.11.0) uses, replacing no stock component.
Nothing in this workspace edits, overrides or installs a stock COSMIC file.

The backend contract and its mock are tested; the overlay (alpha) backend is
in use on a live desktop (see [overlay-backend.md](overlay-backend.md)); the
DDC/CI hardware backend is tested offline only. The applet's design, backend
seam and brightness-key limits are in [applet.md](applet.md). The
requirements that comments and docs cite as R1 to R15 are listed in
[requirements.md](requirements.md).

## Layout

| Path | What it is | State |
| --- | --- | --- |
| `crates/dimmer-core` | Level model, per-monitor policy (`levels.rs`), EDID identity (`edid.rs`), cosmic-config settings store (`store.rs`) and the async `Backend` trait; no hardware or compositor I/O. Mock backend in `mock.rs`. | Implemented, tested; floor defaults provisional |
| `crates/dimmer-ddc` | DDC/CI hardware backend over i2c-dev (R1, R8); see its README. | Implemented, tested offline; no DDC/CI monitor available to verify live (the Frame TV has no DDC) |
| `crates/dimmer-overlay` | Software alpha backend through applet-owned layer-shell overlays (R2). | Implemented, offline-tested; own thread and Wayland connection, EDID identity, lifecycle and live checks in [overlay-backend.md](overlay-backend.md) |
| `crates/cosmic-dimmer` | The libcosmic applet binary, desktop entry, Fluent strings. | In use in the stock panel on a Samsung Frame TV ([applet.md](applet.md)) |
| `scripts/check.sh` | The CI checks, runnable locally. | |
| `.github/workflows/ci.yml` | CI: formatting, clippy, tests, desktop entry validation. | |

`crates/dimmer-samsung` (a tool that wakes and powers a Samsung TV over the
network; see [network-findings.md](network-findings.md)) is its own Cargo
workspace, **excluded** from this one, with its own lock.

## Backend contract (`dimmer-core`)

- `OutputId` names an output within one backend; IDs may go stale on unplug.
- `Level` is retained brightness, whole percent, **1 to 100**: zero is not
  representable. This is only a type-level nonzero floor; the policy is below.
- `Capabilities { kind: Hardware | Software, writable }`.
- `Backend::{outputs, capabilities, set_level}` are all async, because
  discovery can need I/O too (the stock daemon's synchronous discovery blocks
  on slow displays).
- `with_timeout(duration, op)` bounds how long the caller waits (R10) and
  returns `TimedOut`. It drops the future; it **cannot** interrupt blocking I/O
  already running, so a real backend must also run DDC on a worker thread with
  its own transport deadline. A future that blocks inside `poll` is not bounded.
- Errors are explicit (`InvalidLevel`, `UnknownOutput`, `Unsupported`,
  `TimedOut`, `Unavailable`, `Superseded`); a backend must never report a write
  it did not make. `Superseded` means a newer level for the output replaced
  this one before it was written (coalescing), not that the display failed.

## Per-monitor policy (`dimmer-core`, R11, R12)

- `MonitorLevels` holds one monitor's `hardware` and `alpha` levels, a `locked`
  flag and its `Floors`. Every value keeps the invariant: each level between
  its floor and 100, and `hardware × alpha / 100` (the *effective* brightness,
  only a proxy: DDC percent is not linear luminance) at or above the effective
  floor. No constructor or operation can break it, so the UI cannot ask for a
  black screen through this type.
- Unlocked, a slider moves alone and stops at its floor or where the effective
  floor would be crossed. Locked, both move by the same amount and the move
  stops where either would leave its range, so the offset is kept; relocking
  keeps whatever offset the levels have then. Operations return a `Change`
  saying which levels to write.
- `raise(step)` (brightness-up keys, recovery) always lifts the lowest level:
  unlocked the lower one (both on a tie); locked both, each stopping at 100.
- **Floors are provisional, not measured.** The DDC spike has not measured how
  dark the Frame TV gets. Defaults: hardware 10 %, alpha 20 %, effective 10 %;
  configurable down to hard minimums of 5 %, 10 % and 5 %, enforced in the
  crate. Revisit both once a safe hardware floor is measured.
- Settings persist per monitor through cosmic-config (`MonitorStore`, config
  version 1), one key per monitor named from its EDID identity
  (`monitor-<PNP>-<fingerprint>-<serial>`, from `EdidId::from_edid`, which
  checks the base block's checksum; the fingerprint covers the monitor name,
  screen size and serial number text, and the product code only for a
  monitor with no name). Loading never fails: a missing or unparsable
  entry gives full brightness, and every value read is clamped into the
  invariant, with what was changed reported. Two identical monitors that both
  report serial 0 share one entry.

The mock (`MockBackend`) holds independent per-output levels and can delay
every call, which the tests use to prove timeouts on discovery and writes, that
a timed-out write leaves the level unchanged, and that a missing output fails.

## Pinned COSMIC baseline

| Component | Pin | Source of the pin |
| --- | --- | --- |
| cosmic-applets | `328d142faac17c9cc3e00d434d05060411c007ea` | the cosmic-applets of Pop!_OS COSMIC epoch-1.11.0 |
| libcosmic | `430f3d48a97d8779f9c1985473912e594ccca1ae` | that cosmic-applets revision's `Cargo.lock` |
| cosmic-protocols | `c0cff4db14c37ed954983158e4055aa94c7741d9` | that cosmic-applets revision's `[patch]` (`c0cff4d` there; full SHA here) |

The cosmic-applets manifest itself names libcosmic with no revision; only its
lock pins it. Here the pin is in the manifest, as a `[patch]` of the libcosmic
git source with itself (the `//` URL, the same trick cosmic-applets uses for
cosmic-protocols). A plain `rev =` on the dependency is not enough: crates from
other COSMIC repositories (cosmic-panel-config, cosmic-settings-config) depend
on libcosmic's `cosmic-config` by URL, and would pull a second, newer copy from
the default branch. That actually happened and failed to resolve before the patch.

`Cargo.lock` was seeded from the baseline's lock and then pruned by Cargo, so
every shared crate keeps the version the installed panel's applets were built
with. Checked: **all 675** external packages in this lock match the baseline lock
exactly in version and source (libcosmic and cosmic-protocols crates differing only in the patch's
spelling of the same commit). CI uses `--locked`, so the lock cannot drift
silently. `libcosmic` uses the same feature set as cosmic-applets.

## Building and checking

```sh
./scripts/check.sh   # fmt --check, clippy -D warnings, test, desktop entry validation
```

Building `cosmic-dimmer` needs these system packages (Ubuntu/Pop!_OS
names). Found by building the whole dependency graph with `--keep-going`:
they are the only pkg-config probes that fail without them.

| Package | Needed by | On the development host (2026-11-05) |
| --- | --- | --- |
| `libxkbcommon-dev` | smithay-client-toolkit's build script (`xkbcommon.pc`); link-time `libxkbcommon.so` | **Missing** (runtime `libxkbcommon0 1.6.0-1build1` present) |
| `libwayland-dev` | wayland-sys (`wayland-client.pc`) | Present |

`libdbus-1-dev` (installed by cosmic-applets CI for its other applets) is not
needed here: nothing in this graph links libdbus; zbus is pure Rust. The three
backend-free crates build with no system packages at all.

The built binary needs only the runtime `libxkbcommon.so.0`, which every COSMIC
desktop has.

### Toolchain

`rust-version = 1.93`, libcosmic's at this pin and cosmic-applets' toolchain.
CI pins 1.93.1, the version cosmic-applets CI lints with, so the set of clippy
lints does not change under us. There is deliberately no `rust-toolchain.toml`:
it would make every local cargo call try to install 1.93 through rustup, which
offline build environments cannot do. Local runs therefore use whatever stable is
installed (1.99.0 here), and a newer clippy may add lints CI does not have.

## Upstream conventions (R13) and deliberate differences

Following cosmic-applets at the installed baseline:

- **rustfmt:** cosmic-applets has no `rustfmt.toml`, so neither does this
  workspace; it formats with defaults. Upstream CI runs `cargo +nightly fmt`;
  ours runs stable 1.93.1 rustfmt. With no configuration the two agree.
- **clippy:** cosmic-applets CI runs clippy on all targets and features; its
  `just check` adds `-W clippy::pedantic`. The workspace enables `pedantic` as a
  lint group and CI passes `-D warnings`, so pedantic findings fail the build.
  That is stricter than upstream CI. There are no blanket `allow`s; the one
  `#[expect]` (float-to-int casts for Wayland positioner coordinates) is local
  and gives its reason. `unsafe_code` is denied workspace-wide; a future crate
  that truly needs it must opt out in its own manifest, visibly.
- **i18n:** Fluent via `i18n-embed`/`i18n-embed-fl` and an `fl!` macro, English
  fallback embedded, like the stock applets. The catalog file is named after the
  Cargo package: `i18n/en/cosmic-dimmer.ftl` (`fl!` checks it at build time).
- **Desktop entry:** modelled on the stock applets' (`X-CosmicApplet=true`,
  `NoDisplay=true`, `X-HostWaylandDisplay=true`, `Categories=COSMIC;`).
  cosmic-applets validates entries in an Ubuntu 25.10 container because only
  desktop-file-utils 0.28 registers the `COSMIC` category; Ubuntu 24.04/Pop!_OS
  ship 0.27, which rejects it. CI does the same (image pinned by digest);
  `check.sh` validates when it finds 0.28 or later and otherwise prints that it
  skipped. Validated here with the real 0.28-1 package: passes (one hint about
  a main category, which the stock entries also get).
- **CI supply chain:** third-party actions and the container are pinned by
  commit or digest, not tag (cosmic-applets uses tags); the workflow token is
  read-only.

## Verification

`./scripts/check.sh` runs what CI runs: formatting, clippy with `-D warnings`
on all targets and features, every test, and desktop entry validation. Live
results on the Frame TV are recorded in [overlay-backend.md](overlay-backend.md)
and [applet.md](applet.md).

Not verified: the DDC/CI hardware path against a monitor that supports DDC/CI
(none was available; the Frame TV does not answer DDC/CI), and so the hardware
floor defaults, which are provisional.

## Fixed decisions

- **Application ID** `io.github.deepthinker2001.CosmicDimmer`. It names the
  desktop entry, the applet's config namespace and its D-Bus name, so changing
  it would move users' settings and break their brightness-key shortcuts. It
  is one constant (`APP_ID`) plus the desktop file's name; a test checks they
  agree.
- **License:** GPL-3.0-only (the license cosmic-applets uses), in `LICENSE` and
  every manifest; packaging is described in [install.md](install.md).
