# The panel applet (`crates/cosmic-ext-dimmer`)

A libcosmic panel applet (R3) in the style of the stock cosmic-applets: a panel
icon whose popup lists each display with its controls. Strings are Fluent
(`i18n/en/cosmic-ext-dimmer.ftl`, via `fl!`), as R13 asks.

## What the popup shows

For each display, named as the overlay backend names it (the model from the
compositor and the connector, such as `QN55LS03H (HDMI-A-1)`):

Sliders carry an icon, not a text label: a sun for software, a monitor for
hardware.

- A **software** slider (the alpha overlay), always (R11).
- A **hardware** slider (DDC/CI) only when the hardware backend can write the
  display. Otherwise there is **no hardware slider at all**, not a disabled
  one and no note about it, whatever the reason. The hardware level is held at
  100 % in the model.
- **Move both together** (the lock, R11), shown only when both sliders are.
- Under a slider, nothing while writes succeed, and "Not applied: reason.
  The display is at N%." when one failed, N being the last level the backend
  confirmed. A note on every write would resize the popup while the slider
  moves.

If the overlay backend itself cannot reach the compositor, a display that only
the hardware backend lists gets a muted "Software dimming unavailable" note in
place of the software slider: a slider that changed nothing would mislead.
Which parts a row shows is decided by `parts()` in `view.rs`, which the tests
check for a display with hardware and one without.

All of the popup's padding and spacing comes from the theme
(`cosmic::theme::spacing()` and libcosmic's applet helpers), so it follows
COSMIC Settings → Desktop → Appearance → **Interface density** like the stock
applets: tighter when Compact, wider when Spacious. libcosmic reloads the theme when the setting changes, so an open
popup follows it without a restart. Only content sizes are fixed: the 16 px
slider icons and the width of the level text, which density does not change.

A **Settings** section (collapsed) holds the floors and the brightness
shortcuts toggle. Under it, always shown, **Decrease brightness key** and
**Increase brightness key** buttons show each combination: click one and press a new one (Esc cancels). The change per key press appears under them. Keys set while
the toggle is off are kept and used when it goes on. The hardware response limit (how long to wait for a display to answer
DDC/CI, unrelated to the screen-off setting in COSMIC's power settings) is not
in the popup: it is the `timeout_ms` setting file, default 2000. Like
the hardware slider, the lowest-hardware-level and lowest-combined-level
settings appear only when a connected display has hardware control; their
saved values are kept either way. A display without hardware control obeys
the software floor alone, because its combined level is its software level.

## Auto dimming

An **Auto dimming** toggle sits under the displays. While it is on, the
applet learns the levels you choose at different times of day and follows
them, one percent at a time:

- **Learning.** Whenever you let go of a slider or press a brightness key, the
  display's levels are remembered for that moment in the sun's cycle,
  replacing anything remembered within about half an hour of it (40
  thousandths of the cycle). Turning auto dimming on gives a display with no
  points yet its current levels as its first point, so nothing changes until
  you choose levels at other times. Up to 24 points per display are kept, saved in the
  display's `auto_points`.
- **Relative to the sun.** A moment is placed by how far it is between sunrise
  and sunset, or between sunset and the next sunrise (`dimmer_core::sun`). A
  level chosen an hour after sunset in summer applies about an hour after
  sunset in winter too. Where the sun does not rise or set (polar day or
  night), local solar time stands in, with sunrise at 06:00 and sunset at
  18:00.
- **Following.** Between remembered points the level eases from one to the
  next along a half cosine (slow at each point, fastest midway), round the
  whole cycle (`dimmer_core::profile`). Every 2 seconds each display moves at
  most 1 % toward that level, within the floors. With the lock on, the
  software level leads and the lock carries the hardware level. A slider being
  dragged is left alone until it is let go. These steps are not saved; the
  remembered points are.
- **Location.** Under Settings, while auto dimming is on, a field takes
  `latitude, longitude` in degrees. Empty, it uses the reference city of the
  system time zone from the tz database's `zone1970.tab` (Denver for
  `America/Denver`); the field's placeholder names the zone. That city can be
  far enough away to move sunrise and sunset by half an hour or so; typed
  coordinates fix that. Nothing is looked up online.
- **Reset turns it off.** `cosmic-ext-dimmer --reset` and the applet's own reset
  turn auto dimming off, so a screen reset for being too dark is not dimmed
  again. The remembered points are kept for when it is turned back on.

## Behaviour

- **Floors (R12).** Every move goes through dimmer-core's `MonitorLevels`, so
  a level never goes below its floor or takes the combined level below the
  effective floor. Each slider's range also starts where the policy would
  stop it now, so the thumb cannot even be dragged lower.
  Floors are applet-wide: changing one applies it to every display. The
  defaults are dimmer-core's provisional ones.
- **Never blocking (R10).** Every backend call (discovery and each write) runs
  as a task on libcosmic's executor thread, bounded by the hardware response limit
  (default 2 s, 250 ms to 10 s). The slider shows the requested level at once,
  and the result comes back as a message. Writes are coalesced per display and
  channel: at most one is in flight, and while it runs only the newest level is
  kept, so a drag over a slow display sends the latest value next rather than
  a backlog. `Superseded` from a backend is not shown as a failure.
- **No hardware, hardware at 100.** When a display has no hardware output, its
  hardware level is held at 100 % in the model and the lock is off, because
  the floors assume both levels are real. The same applies to a display
  without an alpha output.
- **Saved levels.** Each display's levels are saved per EDID identity through
  dimmer-core's store (`~/.config/cosmic/io.github.deepthinker2001.CosmicExtDimmer/v1/monitor-…`)
  when a slider is released, on lock, reset and brightness keys. They are
  restored when the applet starts. Only levels below 100 % are written then, so
  a display at full keeps whatever its own controls set. A display without a
  readable EDID (its ID is `connector:<name>`) is not saved: a connector is not
  a monitor, and another monitor's levels must never be applied to it.
- **Settings.** As in the stock applets, each setting is its own file in the
  same folder (`CosmicConfigEntry`): `hardware_floor`, `alpha_floor`,
  `effective_floor`, `timeout_ms`, `key_step`, `raise_binding`,
  `lower_binding`, `auto_dimming` and `location`. A missing or unreadable one
  takes its default without affecting the others, and every value is clamped
  on load, so no file can disable a floor. The applet watches them, so a
  change made elsewhere (`--reset`, a hand edit) applies at once. Settings
  saved by earlier versions in a single `settings` file are moved over the
  first time no per-setting file exists; the old file is left in place and
  no longer read.
- **Rediscovery.** Displays are discovered at start and again each time the
  popup opens. Known displays keep their levels and pending writes.

## The backend seam (`src/backend.rs`)

`dimmer_core::Backend` returns `impl Future`, so it cannot be a trait object.
`AnyBackend` is an enum over the mock, `dimmer_ddc::DdcBackend` and
`dimmer_overlay::OverlayBackend`, and `Backends::system()` builds the real
pair:

- **alpha:** `OverlayBackend::spawn()`, the overlay backend, on its own thread and Wayland connection (see
  [overlay-backend.md](overlay-backend.md));
- **hardware:** `DdcBackend`. A display it does not list (the Frame TV, which
  has no DDC/CI) has no hardware slider, and its hardware level is held at
  100 %.

Set `COSMIC_EXT_DIMMER_MOCK=1` to run on in-memory mocks instead: `MOCK-1`
with hardware and alpha, and `MOCK-2` with alpha only, as a display that does
not answer DDC. Nothing real is changed then, and nothing is saved, since the
mock IDs are not EDID identities.

**How the two backends' outputs are joined:** by `OutputId`. The overlay
backend's ID for a display is `EdidId::entry_name()` from the connector's EDID
in `/sys/class/drm` (`connector:<name>` without one). So **the DDC backend
must use the same ID for the same display**: the EDID it reads over I2C gives
the same `EdidId`. The applet parses the ID back into the `EdidId` that keys
saved levels (only an exact `entry_name` form is accepted). The popup label is
the alpha backend's `Output::name`, else the hardware backend's. A display that
only the hardware backend lists still appears, with "Software dimming
unavailable".

## Terminal recovery: `cosmic-ext-dimmer --reset`

For a screen too dark to use the applet. It never starts the applet UI and
needs no root:

1. It calls the running applet's `Reset` on the session bus (3 s timeout). The
   applet sets every display to 100 %, writing both levels even if its model
   already says 100. That removes every overlay, sets hardware to 100 %, and
   saves.
2. It sets every saved `monitor-*` entry under
   `$XDG_CONFIG_HOME/cosmic/<APP_ID>/v1/` (or `~/.config/...`) to 100 % and
   100 %, keeping each one's lock, floors and auto-dimming points, and turns
   auto dimming off (the `auto_dimming` setting), so neither the next start
   nor auto dimming can dim again. The other settings are left alone.
3. **Only when no applet answered**, it writes hardware 100 % itself to every
   display the DDC backend lists, bounded by the hardware response limit from settings.
   A running applet does its own hardware writes, and two writers would
   contend on the bus. Overlays need nothing then: they belong to the applet's
   Wayland connection and go with its process.

It prints one line per step and exits 0 when everything was reset, 1
otherwise. That includes "the applet did not answer". In that case the line
gives the last resort: `pkill -f 'cosmic-ext-dimmer$'`. Ending the process
makes the compositor remove its overlays. (The pattern matches any command
line that ends in `cosmic-ext-dimmer`, such as an editor open on a file of that
name; `pgrep -af 'cosmic-ext-dimmer$'` lists what it would end.) cosmic-panel
does not restart an applet killed by a signal, so
it comes back at the next login or when it is re-added. `--help` and
`--version` print and exit. Any other argument starts the applet as before,
since cosmic-panel starts applets with no arguments.

## Brightness keys (R12) and their limits

On Wayland only the compositor sees global keys. cosmic-comp takes its
bindings from the `com.system76.CosmicSettings.Shortcuts` config (the
`defaults` entry shipped in `/usr/share/cosmic`, and the user's `custom`
entry), and a custom binding can only **spawn a command**. A standalone applet
cannot add a new kind of action or receive key events, so:

- The running applet serves **`Raise` and `Lower`** (and `Reset`, for
  `--reset`) on the session bus. The
  bus name is `io.github.deepthinker2001.CosmicExtDimmer`, the object path
  `/io/github/deepthinker2001/CosmicExtDimmer`, and the interface has the
  same name as the bus. `Raise` lifts the lowest level of every display
  (dimmer-core's `raise`, which works whatever the sliders or the lock say).
  `Lower` drops the highest available level, stopping at the floors. The step
  is the change-per-key-press setting.
- **Brightness shortcuts** (a toggle in Settings, opt-in, off until turned on)
  writes two custom bindings, Super + F1 and F2 until changed: Super + F2 runs
  `busctl --user call <bus name> <object path> <interface> Raise`, and
  Super + F1 runs the same call with `Lower`. They use F-keys because a
  desktop keyboard may have no dedicated brightness keys. This is the same
  `busctl` pattern the stock brightness keys use for cosmic-settings-daemon.
  Capture takes letters, digits, F-keys, arrows and the navigation keys. F-keys
  and brightness keys can stand alone; the others need Super, Ctrl or Alt.
  A lone F-key stops reaching applications while it is bound. A combination COSMIC
  already binds cannot be captured, because the compositor keeps it from the
  popup, so the applet's own two bindings are released while a capture waits
  and written back when it ends, after the release has finished, so a popup
closed mid-capture (for example by a fullscreen window) cannot leave them
off. If the applet dies during a capture, turn the
  toggle on again; the saved keys come back. A change takes effect at once, is remembered while the toggle is off,
  and one made in COSMIC Settings shows here when the popup next opens. The
  writer:
  - never replaces a binding the user's custom shortcuts or the defaults
    already use (it says which one is taken and writes nothing);
  - reads the custom shortcuts **strictly** and refuses when they hold an
    action this build of `cosmic-settings-config` does not know. The lenient
    reader COSMIC uses would turn such an action into `Disable`, and writing
    the result back would lose the user's binding;
  - is idempotent.

  Turning the toggle off removes the bindings that run the applet's commands,
  on whatever keys they sit, under the same strict read. A key rebound to
  something else stays. The toggle is re-read each time the popup opens.

**Limits, stated plainly:**

- The **plain brightness keys stay with the stock daemon**. Taking them over
  would replace stock behaviour (a laptop backlight, and the daemon's own DDC
  writes, which go to every display it reaches over DDC/CI). A custom binding
  runs one command, so the plain keys cannot do both without a wrapper
  script. So with only the plain keys, R12's "brightness-up always works" holds
  for hardware where the stock daemon reaches the display, but **not for the
  software overlay**. Super + F2 is the applet's guaranteed way
  back up, as long as the applet runs.
- The keys reach the applet only while it runs and owns the bus name. A second
  instance (an applet added to both panel and dock) cannot take the name; it
  shows "Brightness keys unavailable" in Settings and the first instance
  handles the keys.
- Reset and recovery must not depend on the applet alone. The overlay backend
  has to clear its surfaces if the applet exits, and
  `--reset` covers the saved levels and hardware.

## Languages

The applet follows the desktop language (`LANG` and the related variables,
which COSMIC's Region & Language settings set), like the stock applets, and
falls back to English. Translations are Fluent files in
`crates/cosmic-ext-dimmer/i18n/<language>/`: English, plus German, Spanish,
French, Italian, Brazilian Portuguese, Dutch, Polish, Swedish, Japanese and
Simplified Chinese. A regional variant without its own folder, such as
Mexican Spanish, uses the language's (`es`). The desktop entry carries the
applet's name and description in the same languages.

The translations were drafted with AI and have not yet been checked by native
speakers; corrections are welcome. Two tests in `src/localize.rs` check that
every translation loads and has every English message with the same
variables, so a new message cannot be forgotten in one language. Backend
details (DDC or Wayland errors) stay English in the log; the popup shows a
translated reason instead.

## Other limits

- **Saved levels follow the EDID, not the connector.** The identity is the
  manufacturer, the serial number and a fingerprint of the monitor name,
  screen size and serial number text. When the EDID names the monitor, the
  product code is left out. The Frame TV changes its product code with its
  settings (0x7a08 with Game Mode on, 0x7a8d with it off, and 0x7a8c when it
  fell back to 4K at 30 Hz after a crash). The fingerprint keeps it one
  monitor: on 2026-11-07, turning Game Mode off kept the saved level.
  A display that changes anything else in its identity still looks like a
  new monitor and starts at full brightness, which is safe. The TV did this
  when its HDMI input was first labelled "PC": serial 0 became 0x01000e00.
  The old entry stays on disk for when the change is undone.
- **Screenshots and recordings come out dimmed.** COSMIC captures the
  composited screen, overlay included, for full-screen, region and window
  shots alike.
- **The lock screen is not dimmed.** Nothing may draw above it; the dimming
  returns after unlocking.
- **The mouse cursor is not dimmed by software dimming.** The compositor
  draws the cursor above every surface, the overlay included. Hardware
  dimming dims it with the rest of the screen.
- **Settings saved before the fingerprint are not read.** Entries named
  `monitor-<PNP>-<4-digit product>-<serial>` came from the earlier identity,
  and the monitor starts at full brightness until it is set again.

## Not verified yet

- The applet runs in the live panel on the Frame TV, including the sliders,
  floors, saved levels, shortcuts and key capture, and through the lock
  screen, hotplug, screenshots and sleep/wake
  and a resolution change ([overlay-backend.md](overlay-backend.md)). Still
  to check: a display that answers DDC/CI.
- Keyboard and screen-reader access: the sliders and the toggler are iced and
  libcosmic widgets, and libcosmic's `a11y` feature is off in the cosmic-applets
  feature set this workspace pins. So the sliders carry icons but no
  accessible names.
