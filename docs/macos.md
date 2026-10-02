# buds-tui on macOS

The Pixel Buds half of this project runs on macOS. The AirPods half cannot,
for a reason that is a property of the platform rather than a missing port.
This document records what was measured, what was built, and what was left
alone on purpose.

Measured on macOS 26.5.1 (Tahoe), Apple N1 controller, against AirPods Pro 2,
AirPods Pro USB-C and Pixel Buds Pro 2.

## Why the Linux build cannot simply be compiled

Both halves of buds-tui sit on [`bluer`](https://github.com/bluez/bluer), the
BlueZ bindings, which say so themselves:

```
error: BlueR only supports the Linux operating system.
error[E0432]: unresolved imports `libc::AF_BLUETOOTH`, `libc::SOL_BLUETOOTH`, `libc::TIOCINQ`
error[E0425]: cannot find function `accept4` in crate `libc`
```

`AF_BLUETOOTH` sockets and the BlueZ D-Bus API are Linux kernel interfaces
with no macOS equivalent. The replacement is `crates/btmac`, which gets the
same two transports — RFCOMM and L2CAP — out of IOBluetooth instead.

Everything *above* the byte stream turned out to be portable untouched. The
`maestro` crate (pwRPC + protobuf) compiles for `aarch64-apple-darwin` as-is;
it only ever wanted an `AsyncRead + AsyncWrite`.

## Pixel Buds: works

The buds advertise the Maestro service in SDP, and macOS has no use for it, so
the channel is free:

```
- MAESTRO APP  rfcomm=1
>> Maestro service FOUND, rfcomm channel 1
openRFCOMMChannelSync -> 0x00000000 (SUCCESS)
mtu=670
139 bytes received unprompted
```

The buds start streaming Maestro frames the moment the channel opens. Battery,
ANC, EQ and the settings panel all work from this.

## AirPods: AACP is closed, but the features are not

Two separate questions got conflated at first, so take them apart.

### Speaking AACP ourselves: no

AirPods expose an `AAP Server` service, and AACP rides L2CAP PSM `0x1001`.
Three findings, in order of how much they settle:

1. **The channel is normally refused.** With the AirPods connected to the Mac,
   `openL2CAPChannelSync` on PSM `0x1001` returns `0xe00002bc`
   (`kIOReturnError`), consistently, whether or not audio is playing. Pausing
   playback changes nothing — it is being connected at all that matters.

2. **There is a window at connect time.** Dropping the baseband link and
   immediately hammering the PSM wins the channel on attempt 2, about 0.1 s
   after `openConnection`. So macOS is not refusing on principle; it is
   holding the one session AACP allows.

3. **Winning the race buys nothing.** With the channel open and an MTU of 672,
   all three init packets — handshake, feature flags, request notifications —
   are accepted by `writeSync`, and the AirPods answer with **zero bytes**,
   across repeated runs, with a 2 s settle delay and 8 s of listening. By
   contrast the Pixel Buds send 139 bytes unbidden.

AACP permits a single session per device and macOS owns it. A second channel
opens but is inert. Porting `aacp.rs` would not change that.

### Getting the features anyway: yes, partly

The mistake was to stop there. macOS *is* driving that AACP session, and it
publishes both what it learns and a lever to steer it. So rather than driving
the accessory, drive the OS. `crates/apmac` does this:

| Feature | Route | Verified |
| --- | --- | --- |
| Noise mode, read | CoreAudio `lstm` on the AirPods audio device | yes |
| Noise mode, write | CoreAudio `lstm` | yes — switched to NC and back |
| Supported modes | CoreAudio `lsms` bitmask | yes — `0x07`, modes are bit *m-1* |
| Per-bud battery | `system_profiler -json SPBluetoothDataType` | yes — L 99%, R 100% |
| Firmware, serial, RSSI, product id | same report | yes |

Prior art that found these first, and is worth reading:
[pods-control](https://github.com/raulgg/pods-control),
[anc](https://github.com/gustaferiksson/anc) and
[WhatBattery](https://github.com/sudoWright/whatbattery).

Four caveats matter, and the last one bites:

* **`lstm` only exists while the AirPods are the active audio output.** When
  the route moves elsewhere, macOS tears the audio device down completely and
  the property goes with it — battery still reads fine, noise control does not.
* **The properties are undocumented**, so a macOS update can move or remove
  them.
* **Battery fields come and go.** `device_batteryLevelCase` appears only with
  a bud in the case, and a bud's own field disappears when it is not in use,
  so every level is an `Option` rather than a number.
* **A write can be accepted and then quietly dropped.** `lsms` advertises
  `Off` even when the AirPods' own "Off Listening Mode" setting is disabled.
  Writing `Off` then returns success, and a read *immediately* afterwards also
  says `Off`, because macOS answers from a cache holding the value that was
  asked for. A second later it reverts.

  So acceptance cannot be trusted as confirmation, and neither can an
  immediate read-back. `apmac` splits the two: `set_mode` writes and returns
  (what the TUI uses, since its one-second poll shows the truth shortly after,
  and the flash message says "requested"), while `set_mode_confirmed` polls
  until the mode has *held* for two consecutive reads and otherwise reports
  `ModeError::NotApplied`. That is what makes `apmac set off` fail honestly:

  ```
  $ apmac set off
  apmac: macOS accepted Off but the mode did not change —
         enable "Off Listening Mode" in the AirPods settings
  ```

### Still out of reach

Conversation Awareness needs the private `AVOutputDevice` in
`AVRouting.framework`, which pods-control reaches only with an interposition
dylib to satisfy entitlement checks — not something worth shipping here. Ear
detection, stem press configuration, adaptive noise level, personalized
volume, and the rest of the thirty-odd AACP settings have no macOS equivalent
at all, and neither does battery while disconnected (which on Linux comes from
Apple's BLE proximity advertisements).

So the macOS AirPods story is noise control plus battery — the most-used part
of the screen, and not much more.

Reproduce the CoreAudio side with `macos-probe/anc_probe.c`:

```bash
cd macos-probe
clang -framework CoreAudio -framework CoreFoundation anc_probe.c -o anc_probe
./anc_probe            # read lstm / lsms
./anc_probe --set 2    # write a mode
```

## Which screen is in front

`buds-macos` opens on the AirPods screen and brings the Pixel Buds screen
forward when the buds connect, mirroring the Linux build's "the pair that
connected last comes to the front".

The subtlety is that a `Connected` event is not evidence of a new connection.
The Maestro session resets whenever the buds hand processing between each
other — pbpctrl documents this as `os error 104` — and `link.rs` reconnects
each time, so `Disconnected` followed by `Connected` is routine and says
nothing about the buds having gone anywhere. Switching on those would drag the
user off the AirPods screen every minute or two.

`Absent` is the only event that means the buds genuinely left, so that is what
re-arms the switch. The screen therefore comes forward once per real
connection and stays where the user put it in between.

## The main-thread constraint

IOBluetooth will only open a channel from the process's **main** thread, and
only delivers delegate callbacks to a run loop running there. A thread of our
own running its own run loop is not enough — `openRFCOMMChannelSync` still
fails with `kIOReturnError`. This was measured both ways round.

So the macOS binary is arranged inside out from the Linux one: `main` hands
itself to the run loop via `btmac::run_main_loop()`, and the terminal UI plus
the Maestro session run on a thread of their own. `btmac` marshals every
IOBluetooth call onto main and buffers inbound frames behind a condition
variable, which is what lets the Rust side read with an ordinary blocking call
on the blocking pool instead of trying to bridge Objective-C delegates into
async Rust.

Frames are queued rather than concatenated, so L2CAP packet boundaries
survive — AACP needs that, even though it is otherwise unusable here.

## Build and run

Needs the Rust toolchain and protobuf:

```bash
brew install protobuf
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

cargo build --release -p pixelbuds-macos
./target/release/pixelbuds-macos
```

Connect the buds to the Mac first: like the Linux build, it only talks to an
already-connected device, so opening it never pulls the buds off a phone.

Without a terminal, or to check the Bluetooth half by itself:

```bash
./target/release/pixelbuds-macos --probe 10   # print events for 10s, no UI
```

No special permission prompt appeared for a binary run from a terminal;
paired-device enumeration and SDP both worked without one. A bundled `.app`
would need `NSBluetoothAlwaysUsageDescription`.

## What was deliberately not ported

These all hang off the AACP session, so none of them has a caller on macOS.
What `apmac` recovers (noise mode, battery) needs none of them:

| Linux piece | Serves | Status |
| --- | --- | --- |
| `pulse_sinks.rs` (libpulse) | AirPods audio rerouting | not ported — macOS routes to AirPods itself |
| `media_controller.rs` (MPRIS) | stem press controls, iPhone handoff | not ported — both are AACP-driven; macOS handles media keys |
| `ble_monitor.rs` (BlueZ BLE) | battery while disconnected | not ported — no macOS equivalent; see "Still out of reach" |
| `airpods-tui.service` (systemd) | daemon, low-battery notices | not ported — nothing to daemonise without AACP |
| `--waybar` JSON | AirPods status for Waybar | not ported |

`pixelbuds-tui` itself does no audio routing and no media control, so the
Pixel Buds screen needs none of the above either.

The Linux build is untouched by any of this: `btmac` and `pixelbuds-macos` are
separate workspace members, and `crates/btmac` is `#![cfg(target_os = "macos")]`.
