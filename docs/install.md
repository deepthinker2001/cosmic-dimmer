# Installing, permissions, recovery and removal

The applet installs **beside** stock COSMIC. It adds its own binary, its own
desktop entry and its own udev rule, all under names nobody else uses. It
replaces, edits and diverts no COSMIC file (R3, R5), and it never runs as root
(R4).

## What gets installed

| File | `.deb` path | `sudo just install` path (prefix `/usr`) |
| --- | --- | --- |
| Applet binary | `/usr/bin/cosmic-ext-dimmer` | `/usr/bin/cosmic-ext-dimmer` |
| Desktop entry (`X-CosmicApplet=true`, so the panel lists it) | `/usr/share/applications/io.github.deepthinker2001.CosmicExtDimmer.desktop` | same |
| udev rule (I2C access, below) | `/usr/lib/udev/rules.d/60-cosmic-ext-dimmer-i2c.rules` | `/etc/udev/rules.d/60-cosmic-ext-dimmer-i2c.rules` |
| Panel icon | `/usr/share/icons/hicolor/scalable/apps/io.github.deepthinker2001.CosmicExtDimmer-symbolic.svg` | same |
| Load `i2c-dev` at boot | `/usr/lib/modules-load.d/cosmic-ext-dimmer.conf` | `/etc/modules-load.d/cosmic-ext-dimmer.conf` |
| README, this file, license | `/usr/share/doc/cosmic-ext-dimmer/` | same |

There are no locale files: the Fluent strings are compiled into the binary, as
in the stock applets.

`just install` puts the udev and modules-load files in `/etc` because udev
does not read `/usr/local`. It refuses to overwrite a file that a package owns,
so it can't clobber the `.deb`'s binary.

The installer does, on the live system:

1. It creates the system group `i2c` if no package has (i2c-tools and ddcutil
   normally have).
2. It loads `i2c-dev` and reloads udev, so the rule applies without a reboot.

Without `just`, the README's "From source without just" lists the same
commands one by one. The Samsung network tool is not installed by either
route (see the README).

It does **not** add anyone to a group, write to `modprobe.d`, rebuild the
initramfs, restart the panel or add the applet to it. Those are your steps.

## I2C access without root (R4, R8)

DDC/CI goes over `/dev/i2c-*`. The rule
(`packaging/udev/60-cosmic-ext-dimmer-i2c.rules`) gives the `i2c` group
read/write on **display DDC buses only**, chosen by an allow-list:

1. **Adapters whose parent is a DRM connector** (`card<N>-<connector>`).
   These are DP AUX channels, which carry DDC/CI to DisplayPort monitors:
   amdgpu's "AMDGPU DM aux hw bus N" and i915's "AUX x/DDI x/PHY x".
2. **Named connector DDC adapters on a display-class PCI device:**
   - "AMDGPU DM i2c hw bus N": the targets of amdgpu connectors' `ddc` links;
   - "i915 gmbus <pin>";
   - "NVIDIA i2c adapter N at <pci>".

Everything else is left out:

- the motherboard's SMBus, where the memory modules' SPD EEPROMs are;
- platform controllers (touchpads, sensors);
- amdgpu's SMU and power-management buses, and nouveau's `nvkm-*` buses;
- any other adapter, including others on a GPU.

udev can't follow a connector's `ddc` link backwards, so the rule matches by
name. **Known residual: NVIDIA.** Its driver creates no `ddc` links and doesn't
expose which I2C port serves which connector. So all its adapters match. On
this host that is five adapters for four connectors, so at least one port
without a connector gets the grant.

**On this host** (2026-11-05: NVIDIA card0 with the open module 595.104.02, and
the AMD iGPU card1), this is the rule evaluated against the real adapters by
`python3 -I packaging/udev/match-report.py packaging/udev/60-cosmic-ext-dimmer-i2c.rules`:

| Node | Adapter | Parent | DRM `ddc` link from | Our rule |
| --- | --- | --- | --- | --- |
| i2c-0, i2c-1 | Synopsys DesignWare I2C adapter | platform `AMDI0010` | | no |
| i2c-2 | NVIDIA i2c adapter 1 at 1:00.0 | PCI GPU 01:00.0 | (none exposed) | yes (NVIDIA name) |
| i2c-3 | NVIDIA i2c adapter 3 at 1:00.0 | PCI GPU 01:00.0 | (none exposed; the TV's bus) | yes (NVIDIA name) |
| i2c-4 to i2c-6 | NVIDIA i2c adapter 4, 5, 6 at 1:00.0 | PCI GPU 01:00.0 | (none exposed) | yes (NVIDIA name) |
| i2c-7 | AMDGPU DM i2c hw bus 0 | PCI iGPU 71:00.0 | card1-HDMI-A-2 | yes (amdgpu name) |
| i2c-8 to i2c-10 | AMDGPU DM i2c hw bus 1, 2, 3 | PCI iGPU 71:00.0 | card1-DP-4, DP-5, DP-6 | yes (amdgpu name) |
| i2c-11 to i2c-13 | AMDGPU DM aux hw bus 1, 2, 3 | connectors card1-DP-4, 5, 6 | | yes (connector parent) |
| i2c-14 to i2c-16 | SMBus PIIX4 adapter | PCI SMBus 00:14.0 (class 0x0c0500) | | **no** |

This host's iGPU exposes no SMU or other non-DDC GPU buses. So here the result
is the same as a broad "any bus on a GPU" rule would give. The allow-list
makes a difference on hardware that does expose them, such as discrete
amdgpu cards.

Why a script and not `udevadm test`: `udevadm test` evaluates only the
installed rules directories, and it can't load an uninstalled file. The
script parses the rule file itself and applies udev's matching semantics.
Checked against `udevadm test`: for the installed `60-i2c-tools.rules`, the
two agree on all 17 nodes of this host.

> **i2c-tools already grants more.** Pop!_OS's i2c-tools package ships
> `/usr/lib/udev/rules.d/60-i2c-tools.rules`, which gives the `i2c` group
> every I2C bus, SMBus and platform controllers included. `udevadm test`
> shows it applying `GROUP` and `MODE 0660` to i2c-14 (SMBus) and i2c-0
> (platform) here. On such hosts our rule adds nothing, and the distro rule
> is the wider grant. Our rule neither depends on it nor revokes it. Whether
> to keep i2c-tools' rule is your decision.

```sh
sudo usermod -aG i2c "$USER"     # once; then log out and back in (a reboot also works)
id -nG | tr ' ' '\n' | grep -x i2c   # check membership in the new session
ls -l /dev/i2c-*                 # GPU buses: crw-rw---- root i2c
```

Network access to the TV needs no rule at all: it is ordinary unprivileged
TCP and UDP.

## Optional, needs your approval: the NVIDIA I2C module option

**The installer never does this.** It is a kernel-module setting that needs a
reboot. On the Samsung Frame TV it is not expected to help: the TV appears to
have no DDC/CI at all (see [ddc-findings.md](ddc-findings.md)). Consider it
only if a display that *should* answer DDC/CI fails with `EIO` on an NVIDIA
card. Read [ddc-findings.md](ddc-findings.md), "approval-gated procedure",
first. The keys are not verified to be honoured by the NVIDIA **open** kernel
module.

1. Check that nothing already sets the option, and keep a copy of anything that does:
   ```sh
   grep -rn 'NVreg_RegistryDwords' /etc/modprobe.d /usr/lib/modprobe.d
   cat /proc/driver/nvidia/params | grep RegistryDwords
   ```
   If a `NVreg_RegistryDwords` line exists, **merge** the two keys into it
   rather than adding a second line (the last definition wins).
2. Make sure you have a working recovery boot entry, physical access and the
   TV's remote.
3. Create a dedicated file (do not overwrite an existing one):
   ```sh
   echo 'options nvidia NVreg_RegistryDwords="RMUseSwI2c=0x01;RMI2cSpeed=100"' \
     | sudo tee /etc/modprobe.d/samsung-dimmer-ddc.conf
   ```
4. Rebuild the initramfs for the running kernel. On Pop!_OS the NVIDIA module
   loads from the initramfs, so this step is required. Don't reboot if it fails:
   ```sh
   sudo update-initramfs -u -k "$(uname -r)"
   ```
5. Reboot. Then check that the setting is active
   (`grep RegistryDwords /proc/driver/nvidia/params`) and try a read-only
   query, for example `ddcutil --bus N getvcp 10`.
6. **Rollback:** delete `/etc/modprobe.d/samsung-dimmer-ddc.conf`, or restore
   your saved copy. Run the same `update-initramfs` command and reboot.

If a fix turns out to need changes to NVIDIA's driver, the kernel, ddcutil or
ddcci-driver-linux, that work belongs in a separate project (R14).

## Recovery: a screen that is too dark

The floors keep every level above a minimum (R12), but they are provisional
guesses, not measurements. If you can't see the panel well enough to drag a
slider back up, open a terminal (`Super` + `T` on Pop!_OS, or a text console
with `Ctrl` + `Alt` + `F3`) and run:

```sh
cosmic-ext-dimmer --reset
```

It sets every display's hardware and software levels to 100 %. With an
applet running, it asks that applet over the session bus, and the applet makes
its own hardware writes. With none running, it writes hardware 100 % itself.
Either way, it saves 100 % for every display, so the next start doesn't bring
the dark level back. It exits 0 when everything was reset and 1 otherwise,
printing a line per step. It needs no root. `--help` and `--version` exist
too. Details: [applet.md](applet.md), "Terminal recovery".

If that doesn't bring the picture back, end the applet. Its overlay surfaces
disappear with its process:

```sh
pkill -f 'cosmic-ext-dimmer$'
```

cosmic-panel does not restart an applet that was killed. It comes back at the
next login or when it is added to the panel again. The
hardware level that a DDC/CI display holds also lives in the display itself,
so the display's own menu can raise it too.

`Super` + `F2` raises the levels too, while the applet runs, once
the opt-in shortcuts are added in its Settings. See [applet.md](applet.md).

## Uninstalling

Remove the applet from the panel first (COSMIC Settings → Desktop → Panel →
Configure panel applets). Then:

```sh
sudo apt remove cosmic-ext-dimmer    # if installed from the .deb
sudo just uninstall                     # if installed with just (same prefix)
```

Either removes every file listed above and reloads udev. Two things stay:

- **The `i2c` group.** Other packages and your membership use it. To leave
  it: `sudo gpasswd -d "$USER" i2c`.
- **Device permissions,** until the next boot or replug.

Your settings stay in
`~/.config/cosmic/io.github.deepthinker2001.CosmicExtDimmer/`. Delete that
folder to forget them.

## Verification record (2026-11-05)

On the development host, with nothing installed on it:

- `just deb` (cargo-deb 3.8.0) built `cosmic-ext-dimmer_0.1.0-1_amd64.deb`.
  `dpkg-deb -c` lists only the files in the table above. Its control data
  shows `Depends: adduser, libc6 (>= 2.39), libxkbcommon0 (>= 0.5.0)` plus
  the postinst and postrm. The binary unpacked from it answers `--help` and
  `--version`.
- `just rootdir=<scratch> install` placed 7 files, and `uninstall` left none.
  `install` refuses to run without a built binary.
- `udevadm verify` accepts the rule. Its match on every adapter on this host
  is the table under "I2C access without root" (`match-report.py`, checked
  against `udevadm test`).

Not verified: installing the `.deb` or running `just install` on a real
system, the rule applying to live device nodes, and the applet appearing in
the panel's applet list. Those are the user's call.
