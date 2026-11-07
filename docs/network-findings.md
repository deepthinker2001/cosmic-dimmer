# Samsung network route: wake, power and brightness

## Decision and scope

**Wake and power over the network: feasible, implemented in
`crates/dimmer-samsung`. Pairing is verified live on the TV. The power
commands await an approved live test.**
**A network brightness control for the HDMI picture: none found.** No D-Bus
brightness route is exposed. Hardware brightness of the PC picture stays with
DDC/CI (`docs/ddc-findings.md`), and software alpha dimming stays with the overlay.

This is the project's own tooling, not destined for any upstream (R14).

Measured on November 5, 2026 (America/Denver), from the Pop!_OS host on the
same Wi-Fi LAN. Discovery was **passive**: the mDNS cache, `GET /api/v2/` on
ports 8001 and 8002, and the TLS certificate on 8002. Afterwards, with the
user's approval, one pairing was made, and a token check was run that sends no
key. No remote key, Wake-on-LAN packet, SmartThings call or TV setting change
has been made.

Identifiers seen during discovery (serial number, MAC addresses, device UUIDs,
AirPlay keys) are deliberately **not** recorded here. The tool learns the MAC
at runtime and keeps it only in the user's config file.

## Observed (passive)

| Item | Observation |
| --- | --- |
| mDNS name | `Samsung.local`, DHCP-assigned IPv4 address on the Wi-Fi LAN |
| mDNS services | AirPlay ("AirPlay Remote Video", port 7000, `manufacturer=Samsung`, `model=LS03HE`), Spotify Connect |
| AirPlay firmware string | `fv=p20.T-PTMHAKUC-0100-1241.3` |
| `GET http://<tv>:8001/api/v2/` | HTTP 200 in ~54 ms, JSON device info, `version 2.0.25` |
| `modelName` / `model` | `QN55LS03HEFXZA` / `26_PTM_FTV` (2026 Frame) |
| `OS` / `type` | `Tizen` / `Samsung SmartTV` |
| `PowerState` | `on` at the time (TV on, PC input) |
| `TokenAuthSupport` | `true` (token pairing required on 8002) |
| `FrameTVSupport` | `true` |
| `networkType` | **`wireless`** (relevant to Wake-on-LAN, below) |
| `developerMode` | `0` |
| `isSupport.remote_available` | `true` |
| `https://<tv>:8002/api/v2/` | HTTP 200, same device info, TLS 1.3 (`TLS_AES_256_GCM_SHA384`) |
| 8002 certificate | Subject `CN=Samsung SmartViewSDK Cert. G2`, issuer `CN=SmartViewSDK Root CA G2`, `O=Samsung Electronics, OU=Visual Display Business`, valid 2022-04-07 to 2072-03-25; Samsung's private root, so verification fails ("self-signed certificate in certificate chain") and no IP/hostname matches |

The device info also carries the TV's Wi-Fi MAC (`wifiMac`), which Wake-on-LAN
needs, and other identifiers that the tool doesn't print.

## Tizen local WebSocket remote API

Known protocol, as used by open-source clients. It is **not yet exercised on
this TV**: every step below shows something on the TV or changes its state, so
each needs the user's approval first.

- **Endpoint.** `wss://<tv>:8002/api/v2/channels/samsung.remote.control?name=<base64 client name>&token=<token>`.
  Port 8001 offers the same channel over plain `ws://`. The tool uses **8002
  only**, so the token never crosses the network in the clear.
- **Pairing.** On a connection without a valid token, the TV shows a
  notification asking whether to allow the device, named by the decoded client
  name ("Samsung Dimmer"), with **Allow** / **Deny**. Someone must choose Allow
  with the remote, typically within about 30 seconds. The TV then sends
  `{"event":"ms.channel.connect","data":{"token":"<digits>", ...}}`. Deny sends
  `ms.channel.unauthorized`, and an unanswered prompt sends `ms.channel.timeOut`.
- **Using the token.** Later connections append `&token=<token>` and are
  accepted without a prompt. The TV may re-issue a token, which the tool saves.
  The TV keeps the allowed device in its device-connection list (menu path
  varies with firmware, e.g. *Settings → Connection/General → External Device
  Manager → Device Connection Manager → Device List*). Removing it there
  revokes the token, and the next connection prompts again.
- **Keys.** `{"method":"ms.remote.control","params":{"Cmd":"Click"|"Press"|"Release","DataOfCmd":"KEY_POWER","Option":"false","TypeOfRemote":"SendRemoteKey"}}`.
- **Frame power semantics.** On Frame models a short `KEY_POWER` toggles
  between TV and **Art Mode**. Holding it (Press, about 3 s, Release) powers
  fully off; that hold is not yet tested here.
- **Live result: short press (power test 1, approved by the user).** The first
  `power-key` was sent at 0 s, and `PowerState` read `on` at 1.2 s, then
  **`standby`** from 2.4 s on, while the device info kept answering. The second
  `power-key` was sent at 19.1 s: `PowerState` read `standby` at 20.5 s and `on`
  again from 21.8 s. Both key sends succeeded, and the TV returned to the PC
  picture. So `PowerState` does **not** report `on` while the picture is off
  after a short press (an earlier draft of this document assumed it did).
- **User observation (verbatim, relayed by the PM):** "The art mode never came
  on, the tv went off after about 10 seconds and then came back on after about
  the same amount of time."
- **What the tool sent:** exactly two messages, each a single
  `{"Cmd":"Click","DataOfCmd":"KEY_POWER"}`: one at 0 s and one at 19.1 s. No
  Press/Release hold was sent (only `off` holds, and it was not run), and
  nothing else was sent in between. The 1 s polls were passive `GET /api/v2/`.
- **Reading:** the off and on were **the two clicks**, not the TV acting on its
  own. `PowerState` changed 2.4 s after the first click and 2.7 s after the
  second, and nothing else changed in between. The user's "about 10 s" is a
  rough estimate of the same ~19 s gap. The picture also returns some seconds
  after `on`, while the HDMI link re-establishes. **On this TV, as set up now, a
  short `KEY_POWER` goes from the PC picture to a screen-off standby with the
  network still up, and back. It does not go to Art Mode.** Possible reasons,
  unverified:
  - a Frame setting that chooses what the power button does (Art Mode or off)
  - Art Mode being unavailable or disabled
  - **Anynet+ (HDMI-CEC)**, which the user switched off for the DDC work at
    about 8:38 p.m., around the time of this test (exact order unknown). It
    can change how the TV and source react to power changes.

  Re-test if any of these change.
- **No brightness command.** The remote channel sends key presses only.
  Reaching the picture menu with simulated keys would be blind and fragile, and
  it isn't considered.

### Pairing: what the user sees, and what happens to the token

Command: `dimmer-samsung pair` (after `dimmer-samsung setup <tv address>`).

1. The tool prints: `Look at the TV: it will ask whether to allow "Samsung
   Dimmer". Choose Allow.`
2. **On the TV**, within a few seconds, a notification appears asking whether
   to allow the device **"Samsung Dimmer"**, with **Allow** and **Deny**. Choose
   **Allow** with the TV's remote, usually within about 30 seconds. The tool
   waits up to 60 seconds. Nothing else changes on the TV: no power, input,
   picture or setting change.
3. On Allow, the tool prints `Paired. Token saved to <path> (mode 600).`
   On Deny it reports that the TV refused. If nobody answers, it reports that
   no one answered in time. Either way nothing is saved.
4. Afterwards "Samsung Dimmer" is listed in the TV's device-connection list.
   Removing it there revokes the token. `dimmer-samsung forget` deletes the
   local copy.

**The token**:

- **Where.** `$XDG_CONFIG_HOME/samsung-dimmer/samsung-tv.json`, or
  `~/.config/samsung-dimmer/samsung-tv.json` when `XDG_CONFIG_HOME` is unset.
  The directory is mode **0700**. The file is created with mode **0600** and
  replaced atomically on each save. The tool refuses to read the file if group
  or others can read or write it, or if it is a symlink.
- **Never shown.** The token is never printed, logged, written to stderr or put
  in an error message, and nor is the connection URL that carries it. In the
  program it is a redacted type: no `Display`, and `Debug` prints `<redacted>`.
  It is only ever sent to the TV, over TLS on port 8002 to the pinned
  certificate. It never goes over plain `ws://` on 8001.
- **Wiped from memory.** The token type isn't `Clone` and is zeroed when
  dropped. The copies the tool makes are also zeroed on drop: the channel URL,
  the serialized config and the file text read back. This is best effort:
  copies inside libraries (tungstenite's read buffer, the JSON parser's
  intermediate buffers) are out of reach.
- **Opened safely.** The file is opened with `O_NOFOLLOW`, then checked on the
  open handle (regular file, owned by this user, no group/other bits) before it
  is read. It can't be swapped between the check and the read.
- **Never in the repo.** The config lives outside the repository. Tests use
  made-up tokens only, and no real token appears in this document, test output
  or reports.
- The same file holds the TV's address, its MAC (for Wake-on-LAN) and the
  pinned certificate fingerprint, none of which the tool prints.

**Live result (November 5, 2026, approved by the user).** `dimmer-samsung pair`
showed the prompt. The user chose Allow, and the command finished in 6.5 s with
exit 0. The TV issued an 8-digit numeric token. It was stored in the config
file (mode 0600 verified with `stat`) and not printed. The pinned fingerprint
matched the leaf certificate's SHA-256 as computed independently with
`openssl`. `dimmer-samsung check` then reconnected with the token over the
pinned certificate. The TV accepted it without a new prompt. No key was sent,
and the token wasn't in the output. No power or picture command has been sent.

**This pairing is for testing only.** Its token and pin live in a test's
scratch config (`XDG_CONFIG_HOME` pointed at a scratch folder), which is
removed after the test, not the user's `~/.config`. A real install
needs the user to run `dimmer-samsung setup <tv>` and `dimmer-samsung pair` once
more as themselves, and to accept the prompt again. The test entry "Samsung
Dimmer" can then be removed from the TV's device list if a duplicate appears.

### Certificate handling

Because the 8002 certificate can't be verified normally, the tool **pins** it.
At `pair`, while the user is at the TV accepting the prompt, it records the
SHA-256 of the TV's leaf certificate (trust on first use). Afterwards it
refuses any other certificate with an explicit error. Handshake signatures are
still verified against the presented certificate, so an impostor must hold the
TV's private key. If a firmware update replaces the certificate, the error says
so, and `pair --repin` pins the new one after the user re-pairs.

## Wake-on-LAN

- The tool sends the standard magic packet (6 × `FF`, then the MAC 16 times)
  over UDP port 9 to `255.255.255.255` and to the TV's own address.
- **The TV is on Wi-Fi.** Wake-on-LAN over Wi-Fi only works if the TV keeps its
  Wi-Fi radio up in standby. On Samsung TVs this is the *Power On with Mobile*
  (or similarly named network-standby) option under the network/expert settings.
  It is **not verified** on this unit, and its current value is unknown. The
  LAN is a mesh Wi-Fi network, and whether it delivers broadcasts to a dozing
  client is unknown. The unicast copy is a fallback that needs the network to
  still know the TV's MAC.
- When the TV answers its device info again but reports `standby` (network up,
  screen off), `wake` presses `KEY_POWER` once (only with a saved token). The
  state is known at that point, so the toggle can't turn the TV off.
- After a short `KEY_POWER` the TV keeps answering on its network but reports
  `standby` (power test 1). `wake` handles that case: when the TV answers
  `standby` on two reads in a row, it presses `KEY_POWER` once, and the
  picture returns as it did in the test.

## SmartThings

SmartThings is Samsung's **cloud** API (`api.smartthings.com`). It needs a
Personal Access Token or OAuth from the user's Samsung account, and the TV must
be registered to that account. Nothing was queried: there is no token, and the
cloud route is the user's decision. Capabilities that Samsung TVs commonly
expose, from public documentation, **unverified for this TV and account**:

| Capability | Use here |
| --- | --- |
| `switch` | `off` works. `on` from full standby is unreliable through the cloud and depends on the same network standby as Wake-on-LAN |
| `audioVolume`, `audioMute`, `tvChannel` | not relevant |
| `mediaInputSource` / `samsungvd.mediaInputSource` | switch to the PC's HDMI input |
| `custom.picturemode` | **picture mode** (e.g. Standard / Movie / Dynamic): a coarse preset, not a level |
| `samsungvd.ambient`, `samsungvd.ambientContent` | Art/Ambient mode content |
| `custom.launchapp` | not relevant |

No documented SmartThings capability sets a TV's **backlight or picture
brightness level**. Picture mode is the closest thing, and it is a preset
switch, not a slider.

## Brightness-like controls: conclusion

| Candidate | Affects the PC picture? | Status |
| --- | --- | --- |
| Remote API keys | only by blind menu navigation | rejected |
| Frame Art Mode channel (`com.samsung.art-app`, `get_brightness`/`set_brightness`) | **no**, Art Mode only | unverified on 2026 firmware; not implemented |
| SmartThings `custom.picturemode` | coarse preset | unverified; not implemented |
| DDC/CI VCP `0x10` | yes | failing on this host (`docs/ddc-findings.md`) |

**Conclusion:** no network control is known to give a brightness level for the
HDMI picture, so the D-Bus brightness route in the issue is **not built**.
**This rests on public documentation and other clients' behaviour, not on a
test of this TV's firmware.** Nobody has probed the Art Mode channel or
SmartThings here: either needs the user's approval, and SmartThings also needs
their cloud token. Samsung could expose a control on this 2026 firmware that
these sources don't describe. If a later
approved probe finds a usable control (e.g. Art Mode brightness, wanted for the
Frame's art display), it can be added as a D-Bus property under its own name,
never as a substitute for HDMI picture brightness.

## What the Mac Mini most likely uses (to confirm with the user)

- **Dimming.** Mac apps that dim external displays (MonitorControl,
  BetterDisplay, Lunar) use **DDC/CI over HDMI** on Apple Silicon, or fall back
  to software (gamma/overlay) dimming. If the Mac dims this TV through DDC, the
  TV does answer DDC/CI, and the Linux failure lies on the host's NVIDIA/I2C path
  rather than in the TV. That is a strong hint for the DDC investigation. If it
  is software dimming, it says nothing about DDC. **Which app, and whether it
  says "hardware/DDC" or "software", should be asked.**
- **Wake.** macOS doesn't drive HDMI-CEC. The likely mechanisms are the TV's
  own reaction to a source signal returning (input auto-switch / Anynet+ from
  another device), or an AirPlay connection, which the TV advertises and which
  wakes it from network standby. The network route here is the closest Linux
  equivalent that doesn't depend on CEC.

## The tool (`crates/dimmer-samsung`)

A synchronous binary with no async runtime. It is its own Cargo workspace,
excluded from the root workspace, since nothing else needs its network
dependencies.

```sh
CARGO_TARGET_DIR=... cargo test --manifest-path crates/dimmer-samsung/Cargo.toml --locked
```

| Command | What the TV sees |
| --- | --- |
| `dimmer-samsung setup <host>` | nothing (passive `GET /api/v2/`); saves host and MAC |
| `dimmer-samsung info` | nothing (passive) |
| `dimmer-samsung pair [--repin]` | **Allow/Deny prompt** naming "Samsung Dimmer" |
| `dimmer-samsung wake [--timeout S]` | Wake-on-LAN packet; maybe one `KEY_POWER` if it answers in standby; nothing if already on |
| `dimmer-samsung check` | nothing with a valid token (connects, sends no key); a revoked token prompts again |
| `dimmer-samsung power-key` | one `KEY_POWER` click (on this TV now: picture ↔ screen-off standby, network stays up; Art Mode didn't appear) |
| `dimmer-samsung off` | `KEY_POWER` held 3 s (full off); nothing unless it reports `on` |
| `dimmer-samsung forget` | nothing (local only; revoke on the TV's device list too) |

Boundary handling:

- **Token storage.** `$XDG_CONFIG_HOME/samsung-dimmer/samsung-tv.json` (else
  `~/.config/...`), directory 0700. The file is created 0600 and replaced
  atomically. A file readable by group or others, or a symlink, is refused.
  The token is a redacted type (no `Display`, `Debug` prints `<redacted>`). It is
  never printed, logged or put in an error. The channel URL holding it is never
  formatted into a message. Tokens from the TV must be 1–64 alphanumerics, or
  they are refused without echoing them.
- **Deadlines and size caps.** Each exchange has an **overall** deadline, not
  only per-read timeouts. Before every read and write, the socket timeout is
  set to the time left, so a TV (or impostor) that trickles bytes can't stretch
  an exchange. The limits are:
  - 5 s for device info (connect, request and the complete reply)
  - 5 s for TCP+TLS+WebSocket upgrade
  - 60 s for the pairing prompt, 10 s for the connect event otherwise
  - 5 s per key send

  Device info is capped at 64 KiB, chunk sizes are bounded before any
  arithmetic, and WebSocket messages and frames are capped at 64 KiB.
- **Untrusted input.** The host is restricted to an IPv4 address or DNS name,
  so the config can't inject into requests. Strings from the TV are stripped of
  control characters before printing. A malformed MAC is dropped. JSON is
  parsed strictly.
- **Toggle safety, and an unauthenticated power state.** `KEY_POWER` toggles,
  so `wake` and `off` check `PowerState` first. That value comes from the plain
  HTTP endpoint on 8001, which anyone on the LAN could spoof, and it changes
  during transitions. So the tool only acts on **two consistent reads** half a
  second apart (two `standby` reads in a row in `wake`'s loop). A spoofer
  could at worst cause one toggle (the same as one remote press), or block one.
  The key itself only ever goes to the pinned TV over TLS.

Testing: 33 unit tests, all offline. They cover the magic packet and send,
device-info and HTTP/chunked parsing (including huge or overflowing chunk
sizes), config modes/refusals/redaction, the pinning verifier, overall
deadlines against peers that trickle bytes, and the WebSocket
pairing/deny/timeout/401 paths against a local stand-in. An end-to-end TLS test pins a self-signed certificate and is
refused with a different pin. Live, with the user's approval:

- `setup`, `info` and `check` (passive, or sending no key)
- `pair` (the prompt was accepted)
- power test 1: two `power-key` clicks, screen off and back

## Outstanding (each needs the user's approval and presence at the TV)

1. ~~`dimmer-samsung pair`~~: done (see the live result above).
2. ~~`dimmer-samsung power-key` twice~~: done. The short press gives a
   screen-off standby with the network up, not Art Mode (see the live result).
3. `dimmer-samsung off`: confirm a full power-off. Then `dimmer-samsung wake`:
   confirm whether Wi-Fi Wake-on-LAN wakes it, and record the *Power On with
   Mobile* / network-standby setting.
4. Ask which Mac app and mechanism dims and wakes the TV (see above).
5. SmartThings only if the user wants a cloud route and provides a token.
