# Alpha dimming backend (`crates/dimmer-overlay`)

Software ("alpha") dimming for R2/F2: one black wlr-layer-shell surface per
output on the **overlay** layer of stock cosmic-comp, at opacity
`1 - retained level`. The protocol sequence was first proven on stock
cosmic-comp by a standalone prototype, and since the Frame TV turned out to
have no DDC/CI (R1), it is that display's only dimming.

## Where it runs: in the applet process, on its own thread and connection

**Decision: in-process.** `OverlayBackend::spawn()` starts a thread named
`dimmer-overlay` with its own Wayland connection to cosmic-comp. A helper
process was considered and turned down for now.

Why in-process is safe enough for the panel:

- **The panel is already isolated.** cosmic-panel runs each applet as its own
  client process. Nothing the applet does, including aborting, can crash the
  panel. The thing to protect is the applet's own UI.
- **Protocol errors are per connection.** A compositor-side error on the
  overlay's connection (an invalid request, a killed client) ends only that
  connection, not the applet's UI connection. The worker turns it into
  `Status::Unavailable`, answers waiting callers with `Unavailable`, and
  reconnects with backoff (1 s doubling to 30 s).
- **Failure leaves the screen undimmed, never stuck dark.** Closing the
  connection makes the compositor remove every surface (observed in spike runs
  2 and 3). That holds for a crash of the applet too.
- **No panics on the overlay path.** The release profile has `panic = "abort"`,
  so a panic anywhere aborts the applet. cosmic-panel restarts an applet only
  when it exits *with a code* (`should_restart = is_restarting &&
  err_code.is_some()` in cosmic-panel's `wrapper_space.rs`); a SIGABRT has
  none, so an abort would remove the applet until the next login. Every error
  at this boundary is therefore a `Result`: no `unwrap`/`expect`/indexing on the
  worker path, bounded allocations, and bad configures treated as a close.

What a helper process would add: isolation from aborts inside dependencies
(SCTK, wayland-client) and from a worker that hangs. It would cost a second
installed binary, an IPC protocol to secure, and supervision. **Revisit it** if
a live check or a field report shows the overlay thread aborting or hanging the
applet. The worker sits behind a command channel and a snapshot, so it could
move into a process without changing the `Backend` API.

## Which compositor socket: `WAYLAND_DISPLAY`, never `WAYLAND_SOCKET`

An applet does not get the real compositor through the usual variables.
cosmic-panel (`cosmic-panel-bin/src/space/wrapper_space.rs` at the pinned
revision) starts applets with:

- `WAYLAND_SOCKET`: a socket to **the panel's own embedded server**. The
  applet's button and popups live there. A layer surface made there would
  never reach cosmic-comp.
- for `X-HostWaylandDisplay=true` entries (ours has it):
  `X_PRIVILEGED_WAYLAND_SOCKET`, a descriptor that libcosmic's applet-token
  handler (`src/applet/token/wayland_handler.rs`) takes for itself and that
  can carry one connection only, plus the host's `WAYLAND_DISPLAY`.

So the overlay connects to the socket that `WAYLAND_DISPLAY` names (relative
to `XDG_RUNTIME_DIR`, as libwayland does), with `Connection::from_socket`.
`Connection::connect_to_env` would prefer `WAYLAND_SOCKET` and land on the
panel's server, so it is never used. Such a client has no security context,
and cosmic-comp's `client_not_sandboxed` filter, which gates wlr-layer-shell,
admits it (`src/state.rs` at the pinned cosmic-comp).

## Output identity: the same EDID identity as dimmer-core

`OutputId` is `dimmer_core::EdidId::entry_name()`: manufacturer, a fingerprint
(monitor name, screen size and serial number text, or the product code when
the EDID names no monitor) and serial from the EDID base block, the name
dimmer-core's store keys settings by.
The applet can join a monitor's saved levels to its overlay without a table.

Source of the EDID: cosmic-comp names each `wl_output` after its DRM connector
(`HDMI-A-1`) and fills make and model from the EDID. The serial reaches
clients only through output management. The kernel publishes the same blob,
world-readable, at `/sys/class/drm/card<N>-<connector>/edid`; it is also what
a DDC backend reads over I2C. `identity::resolve_sysfs` joins on the connector
name, requires `status` = `connected`, and parses with `EdidId::from_edid`
(header and checksum checked). Reads are bounded (32 KiB EDID, 64 B status).

Not trusted blindly:

- The connector name comes from the compositor. It is only **compared** with
  directory names, never joined into a path, so `../x` matches nothing.
- The same connector name on two DRM devices with different monitors is
  `Ambiguous` and is refused.
- No EDID, an invalid one or an ambiguous match falls back to
  `connector:<name>`. That ID can never equal an EDID-derived one, so another
  monitor's saved level is never applied by mistake. The output is still
  dimmable, keyed by connector. The identity is re-checked on every
  `wl_output` update, in case the EDID becomes readable later.
- Two identical monitors that report the same serial share one identity and
  one level, as they share one settings entry in dimmer-core.
- EDID identities carry serial numbers, so logs show connector names only.

## Lifecycle (`state.rs`, a pure state machine)

| Event | What happens |
| --- | --- |
| Level set to 100 % | Every surface for that identity is **destroyed**. No transparent surface is kept, so there is no compositor cost by default. |
| Level set below 100 % | Surface created if absent: empty input region (`Some(empty)`, never `None`), overlay layer, all anchors, size 0×0, exclusive zone -1, no keyboard, commit with no buffer. Drawn on configure. An existing surface is redrawn at the same size. |
| Level below the floor | Applied at `ALPHA_FLOOR` = `Floors::HARD_MINIMUM.alpha()` (10 %, so at most 90 % opaque), whatever the caller sent. dimmer-core already enforces its floors; this is R12's last line of defence. `snapshot()` shows the level applied. |
| Configure, new size (resolution, scale, rotation) | New buffer at the logical configure size, buffer scale 1 (a constant colour needs no HiDPI detail). |
| Configure, same size | Commit only, which applies the acknowledged configure. |
| Invalid configure (0, over 16384, over 512 MiB in total) | Treated as a close: dropped, counted, never drawn. |
| Output removed (unplug, a monitor dropped in sleep) | Surface destroyed. The level **stays retained under the identity**. |
| Output added | The identity is resolved. If a level below 100 % is retained for it, the surface is recreated: replug and wake restore dimming. |
| Compositor `closed` | Respected: dropped, not recreated by itself. A later level from the user may recreate it, at most 3 times per output; after that the output reports `writable: false`. Never a loop against compositor policy. |
| Connection lost | Every surface goes with it. Levels are kept, callers waiting get `Unavailable`, and reconnection uses backoff. On reconnect the outputs come back and their retained levels are redrawn. |

Replies are honest: `set_level` answers `Ok` only after the new pixels are
committed, or the surface is destroyed at 100 %. A level still waiting for the
first configure is answered `Superseded` if a newer level for the same output
arrives (dimmer-core's `BackendError::Superseded`). It is answered
`Unavailable` if the output, the surface or the connection goes first.

Each draw makes a new buffer from the surface's pool. The previous buffer may
still be held by the compositor, so it is never written to again. SCTK
destroys it once released. There are no frame callbacks and no redraw loop.

## Not handled here (by design)

- **Lock screen:** no session-lock binding, no raising above the lock, no
  input. If cosmic-comp hides overlay surfaces under the lock, dimming is
  simply absent there, which is accepted. The backend keeps
  its surfaces, so dimming should be back after unlock. That is a live check.
- **Sleep/wake:** handled through the events above (outputs leaving and
  returning, `closed`, a lost connection), with no logind dependency. Whether
  cosmic-comp keeps, removes or closes anything across suspend is a live
  check.
- **Combined hardware × alpha floor:** policy in dimmer-core
  (`MonitorLevels`). This layer only sees the alpha level.

## Checks

Offline (no compositor; run by `./scripts/check.sh`): 34 unit tests in the
crate and 2 in the live example, covering every row of the lifecycle table,
identity resolution (connector-name matching, ambiguity, a corrupt EDID, a fake
sysfs tree), socket choice, backoff, pixel and size arithmetic, and the
backend against a missing socket, a server that hangs up and a server that
sends garbage (all `Unavailable`, none a panic).

Live, each run approved separately by the user:
`cargo run -p dimmer-overlay --example overlay_live --locked -- --live-approved --level N --seconds S`.
Without `--live-approved` it connects to nothing. `--level` defaults to 100
(no surface) and is bounded to the floor; `--seconds` defaults to 60
(at most 600). A watchdog thread exits after that much *awake* time
(CLOCK_MONOTONIC, so suspend does not use it up). It prints output names,
whether the EDID matched, levels, surface counts and the connection state. It
prints no EDID identities and captures no screen contents or input.

### Live results

Through the applet in the stock panel, on the Frame TV (HDMI-A-1), the only
connected display, 2026-11-06, reported by the user:

- The panel lists and hosts the applet, with no stock component replaced.
- The overlay dims the whole screen including the panel, down to the floor,
  with the panel still readable.
- Clicks on windows and on the panel pass through while dimmed.
- The saved level comes back when the applet restarts.

On 2026-11-07, same setup:

- **Lock screen:** the lock screen shows at full brightness, because nothing
  may draw above it. After unlocking, the dimming is back and the applet has
  not restarted.
- **TV off and on with the remote:** dimming and panel unaffected (the HDMI
  link never dropped).
- **Hotplug (cable pulled and replugged by the user):** cosmic-panel restarted
  the applet, which came back. The TV returned with a different EDID product
  code, so it read as a new monitor at full brightness. Fixed by the EDID
  fingerprint ([applet.md](applet.md#other-limits)); turning Game Mode off,
  which also changes the product code, then kept the saved level.
- **Screenshots:** full-screen, region and window screenshots from COSMIC's
  tool all come out dimmed, since they capture the composited output. The
  overlay is not offered as a window in the screenshot tool, and appears in
  neither Alt+Tab, the workspaces overview nor the dock.
- **Resolution change (by hand, in COSMIC Settings):** passed, reported by
  the user. cosmic-panel restarted the applet as the output changed, and
  cosmic-comp logged no errors. Never change the mode from a shell: earlier
  that day `cosmic-randr mode --test` applied the mode for real,
  cosmic-comp's page flips on the NVIDIA card then failed, and the session
  froze until a power cycle.

- **Sleep/wake (suspend from the power menu, woken with a key):** the screen
  came back dimmed; the applet kept running, with no restart.

Every lifecycle check has now run on the Frame TV.
