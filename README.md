# Dimmer for COSMIC

Per-display dimming for a Pop!_OS COSMIC desktop, as a standalone panel applet
(`cosmic-ext-dimmer`). It has two kinds of dimming:

- **hardware**, over DDC/CI, for displays that support it;
- **software**, a translucent overlay that the applet draws, for any display.

It installs beside stock COSMIC and replaces none of its components.
Licensed GPL-3.0-only (see [LICENSE](LICENSE)).

**Dimmer for COSMIC is an unofficial, independent project.** It is not made,
endorsed or supported by System76, and is not part of COSMIC. "COSMIC" names
the desktop it runs on. The `cosmic-ext-` prefix follows the COSMIC
community's convention for third-party projects. Please report problems here,
not to System76 or the COSMIC developers.

Until version 0.2.0 it was called COSMIC Dimmer (`cosmic-dimmer`); see
[Moving from cosmic-dimmer](#moving-from-cosmic-dimmer).

## Status, honestly

- **Software dimming** works on stock cosmic-comp and is in daily use on a
  Samsung Frame TV (QN55LS03H) on Pop!_OS 24.04: through the lock screen,
  hotplug, sleep/wake, resolution changes and the TV's own off/on
  ([docs/overlay-backend.md](docs/overlay-backend.md)).
- **Hardware dimming (DDC/CI) has not been tested on a real DDC/CI monitor.**
  It is fully tested against simulated displays, but the only display it was
  developed with, the Frame TV, does not answer DDC/CI
  ([docs/ddc-findings.md](docs/ddc-findings.md)). If you have a monitor that
  does, a report of how it behaves is very welcome. The hardware floor
  defaults are provisional for the same reason.
- **Auto dimming** learns the levels you choose through the day.
- **Languages:** English, German, Spanish, French, Italian, Brazilian
  Portuguese, Dutch, Polish, Swedish, Japanese and Simplified Chinese. The
  translations were drafted with AI and have not been checked by native
  speakers yet; corrections are welcome.
- **Known limits:** the lock screen is not dimmed (nothing may draw above it),
  software dimming does not dim the mouse cursor, and screenshots and
  recordings come out dimmed
  ([docs/applet.md](docs/applet.md#other-limits)).
- **The Samsung network tool** (`crates/dimmer-samsung`) can only **wake and
  power** a Samsung TV; the TV's network API has no brightness control
  ([docs/network-findings.md](docs/network-findings.md)). It is built from
  source and is not part of the package. Live on the Frame, 2026-11-13: after
  pairing (about 4.5 s, one Allow on the remote), `off` (3 s power-key hold)
  powered the TV off and a bare Wake-on-LAN packet brought it back in 35.8 s,
  with no KEY_POWER needed. Details and the overlay timings are in
  [docs/overlay-backend.md](docs/overlay-backend.md#live-check-example-2026-11-13-fv-w7w7aj).
  Not yet verified: the installed applet in the panel (two sliders per
  monitor, lock, reset, hotplug, sleep/wake), hotplug with a second monitor,
  and a comparison with the Mac Mini's behaviour, which has no measurements
  recorded here.

## How it was made

Dimmer for COSMIC was built using AI. The code, tests, documentation and
translations were written with AI assistance, and the code and security
reviews before each change were AI-assisted too. The author set the
requirements, made the design decisions, tested every behaviour on a real
desktop and display, and decides what is released. Every change passes the
same checks as any other contribution: formatting, clippy with pedantic lints
as errors, and the full test suite in CI.

## Install

### With apt (recommended)

Add the signed repository once; updates then arrive with your normal system
updates:

```sh
curl -fsSL https://deepthinker2001.github.io/cosmic-ext-dimmer/key.gpg \
  | sudo gpg --dearmor -o /usr/share/keyrings/cosmic-ext-dimmer.gpg
echo "deb [signed-by=/usr/share/keyrings/cosmic-ext-dimmer.gpg] https://deepthinker2001.github.io/cosmic-ext-dimmer stable main" \
  | sudo tee /etc/apt/sources.list.d/cosmic-ext-dimmer.list
sudo apt update
sudo apt install cosmic-ext-dimmer
```

The signing key's fingerprint is
`20B9 1D4A ACA9 BE9E 19F4 8B8C 4FC4 764D 9918 AC0A`.

### Moving from cosmic-dimmer

Version 0.2.0 renamed the package from `cosmic-dimmer` to `cosmic-ext-dimmer`,
and the apt repository moved with it. The old repository address no longer
works. First remove the old applet from the panel, so it stops. Then swap the
apt source once (the signing key is the same) and install the new package;
apt removes the old one:

```sh
sudo rm /etc/apt/sources.list.d/cosmic-dimmer.list
sudo mv /usr/share/keyrings/cosmic-dimmer.gpg /usr/share/keyrings/cosmic-ext-dimmer.gpg
echo "deb [signed-by=/usr/share/keyrings/cosmic-ext-dimmer.gpg] https://deepthinker2001.github.io/cosmic-ext-dimmer stable main" \
  | sudo tee /etc/apt/sources.list.d/cosmic-ext-dimmer.list
sudo apt update
sudo apt install cosmic-ext-dimmer
```

Then add **Dimmer** to the panel again. At its first start the applet copies
your settings and saved levels from
`~/.config/cosmic/io.github.deepthinker2001.CosmicDimmer/`, and points the
brightness shortcuts it added before at its new name. The old folder is left
in place; delete it once you are happy.

### From a release

Download the `.deb` from the
[releases page](https://github.com/deepthinker2001/cosmic-ext-dimmer/releases),
then:

```sh
sudo apt install ./cosmic-ext-dimmer_*.deb
```

### Building the .deb yourself

```sh
cargo install cargo-deb          # once
just deb                         # or: cargo deb --locked -p cosmic-ext-dimmer
sudo apt install ./target/debian/cosmic-ext-dimmer_*.deb
```

### From source with just

```sh
just build-release               # as yourself
sudo just install                # binary, desktop entry, udev rule, modules-load entry
```

Building needs Rust 1.93 or later and `libxkbcommon-dev` and `libwayland-dev`
(see [docs/workspace.md](docs/workspace.md)).

### Then, for hardware dimming

Join the `i2c` group once, then log out and back in:

```sh
sudo usermod -aG i2c "$USER"
```

The udev rule gives that group access only to the displays' DDC buses (an
allow-list; see docs/install.md for exactly which, and what i2c-tools
already grants).
Nothing runs as root or setuid. Details, and an **optional** NVIDIA module
setting that the installer never applies, are in
[docs/install.md](docs/install.md).

## Add it to the panel

COSMIC Settings → Desktop → Panel (or Dock) → **Configure panel applets** →
**Add applet** → **Dimmer**. If it isn't in the list, close COSMIC Settings
completely and reopen it: it reads the applet list only when it starts, so you
don't need to log out.

## First use

Click the panel icon. Each display has:

- a **software** slider, marked with a sun;
- a **hardware** slider, marked with a monitor, only if the display answers
  DDC/CI;
- when both sliders exist, a lock that moves them together.

Below the displays, **Auto dimming** learns the levels you set at different
times of day and follows them through the day, 1 % at a time, relative to
sunrise and sunset where you live (from your time zone, or coordinates you
type in Settings).

Nothing goes below the floors in Settings, so the screen never goes black.
Levels are saved per display and restored at login. See
[docs/applet.md](docs/applet.md) for details, including the opt-in
brightness shortcuts and how to change their keys.

## Troubleshooting

| Symptom | What to do |
| --- | --- |
| **Screen too dark to use** | In a terminal (`Super` + `T`, or `Ctrl` + `Alt` + `F3`) run `cosmic-ext-dimmer --reset`: everything goes to 100 % and is saved. Last resort: `pkill -f 'cosmic-ext-dimmer$'` (the overlay disappears with the process; the applet returns at next login). See [docs/install.md](docs/install.md#recovery-a-screen-that-is-too-dark). |
| No hardware slider | The display has no working DDC/CI on this connection (the Frame TV's case), or you aren't in the `i2c` group. Software dimming still works. Check DDC/CI in the display's own menu, and try `ddcutil detect`. |
| Permission denied on `/dev/i2c-*` | `id -nG` must list `i2c`. If not, run `usermod` as above and log in again. `ls -l /dev/i2c-*` should show `root i2c` and `crw-rw----` on the GPU's buses. |
| Dimmer missing from "Add applet" | Check the desktop entry: `ls /usr/share/applications/io.github.deepthinker2001.CosmicExtDimmer.desktop`. Then close COSMIC Settings completely and reopen it. |
| NVIDIA card, DDC/CI display fails with I/O errors | See the optional NVIDIA step in [docs/install.md](docs/install.md). It needs an initramfs rebuild and a reboot. |

## Uninstall

Remove it from the panel. Then run `sudo apt remove cosmic-ext-dimmer`, or
`sudo just uninstall` if you installed from source. Your settings stay in
`~/.config/cosmic/io.github.deepthinker2001.CosmicExtDimmer/` until you
delete them. See [docs/install.md](docs/install.md#uninstalling).

## Reporting problems

Open an issue at
<https://github.com/deepthinker2001/cosmic-ext-dimmer/issues>. Reports from
DDC/CI monitors and translation fixes are especially useful. The applet's
log lines are in `journalctl -b | grep CosmicExtDimmer`; they name displays by
model and connector and never include EDID serial numbers.

## Development

```sh
./scripts/check.sh   # what CI runs: fmt, clippy -D warnings, tests, desktop entry
```

[docs/workspace.md](docs/workspace.md) covers the layout, the pinned COSMIC
baseline, the build requirements and what has been verified. [RELEASE.md](RELEASE.md) describes how to publish a new version.

## Changelog

What changed in each version, newest first.

### 0.2.0 (2026-11-08)

- Renamed from COSMIC Dimmer (`cosmic-dimmer`) to Dimmer for COSMIC
  (`cosmic-ext-dimmer`), the COSMIC community's naming for third-party
  projects, and made clear that it is unofficial and not endorsed by System76.
  The App ID is now `io.github.deepthinker2001.CosmicExtDimmer`, and the
  repository and apt address moved; see
  [Moving from cosmic-dimmer](#moving-from-cosmic-dimmer).
- Settings, saved levels and the applet's brightness shortcuts carry over
  from the old name on first start.

### 0.1.1 (2026-11-07)

No change to how the applet works.

- Published through a signed apt repository, so updates now arrive with your
  normal system updates (see [Install](#with-apt-recommended)).
- The changelog moved into this README, which the package installs.
- Documented that software dimming leaves the mouse cursor bright, and that
  the popup follows COSMIC's Interface density setting (Compact, Standard or
  Spacious).

### 0.1.0 (2026-11-07)

First release.

- A standalone COSMIC panel applet that dims each display on its own:
  software dimming through a translucent overlay on any display, and hardware
  dimming over DDC/CI where a display supports it (not yet tested on a real
  DDC/CI monitor).
- Minimum brightness floors, so the screen never goes black, and
  `cosmic-dimmer --reset` to bring every display back to full brightness from
  a terminal.
- Auto dimming: learns the levels you set at different times of day and
  follows them, 1 % at a time, relative to sunrise and sunset where you are.
- Optional brightness shortcuts (Super + F1 / F2 by default, rebindable).
- Levels saved per display, keyed by the display's EDID identity.
- English, German, Spanish, French, Italian, Brazilian Portuguese, Dutch,
  Polish, Swedish, Japanese and Simplified Chinese.
