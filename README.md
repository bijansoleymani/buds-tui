# buds-tui

AirPods **and** Google Pixel Buds Pro / Pro 2 in one terminal UI. The screen
follows what is connected over Bluetooth: the AirPods screen for AirPods, the
Pixel Buds screen for Pixel Buds. With both connected, the pair that
connected last comes to the front, a tab line appears at the top, and `b`
switches between them.

This is [airpods-tui](https://github.com/annoyedmilk/airpods-tui) (tracked as
the `upstream` remote) with the Pixel Buds screen from
[pixelbuds-tui](https://github.com/bijansoleymani/pixelbuds-tui) built in. Everything below describes the
AirPods side, which is unchanged: the daemon, IPC socket, config directory,
Waybar module and battery file are all still airpods-tui's and serve AirPods
only. The Pixel Buds screen talks to the buds directly while the TUI is open.

Build needs `protobuf-compiler` in addition to the dependencies below.

---

# airpods-tui

A terminal UI for managing AirPods on Linux, built for [Omarchy](https://omarchy.org/). Speaks Apple's AACP control channel over Bluetooth to expose battery, noise mode, conversation awareness, stem controls, and the rest of the iOS settings panel from a keyboard-driven TUI.

![airpods-tui](airpods-tui.png)

## Features

- **Battery** per pod, case, and headphone (Max), with color indicators and low-battery desktop notifications at 20% and 10% (via the daemon). The case reports its level whenever a pod sits inside; the last known value is retained (marked "last seen") while you wear both pods
- **Case lid** open/closed next to the case level whenever a pod sits in the case, both while connected and from the proximity broadcasts afterwards
- **Noise control**: Transparency, Adaptive, Noise Cancellation and Off (model-aware; Adaptive
  only on capable devices, Off only while Off Listening Mode is on). Number keys pick the mode
  by its row, so the digits always match what is drawn
- **Settings panel**, dynamically built per model and grouped the way Apple's own
  AirPods settings are, so a row is findable by whatever it is called on the phone:
  - *Audio & Routing*: Conversation Awareness (Pro 2, Pro 3, Pro USB-C, 4 ANC, Max 2),
    Adaptive Noise Level slider (adaptive-capable models), Off Listening Mode,
    Personalized Volume, Microphone (Automatic / Always Right / Always Left AirPod),
    Automatic Ear Detection, Pause Media When Falling Asleep, Connect to This Computer,
    Tone Volume, Enable Charging Case Sounds + Charging Case Sound Volume
  - *Controls & Gestures*: Volume Swipe + Volume Swipe Length, Press Speed, Press & Hold
    (stem-equipped models), press-and-hold action per bud (Noise Control or Siri),
    Hold Cycle membership (which of Off / NC / Transparency / Adaptive the press-and-hold
    cycles through), Crown Direction (AirPods Max), Siri Voice Trigger
  - *Accessibility*: Use One AirPod, Noise Cancellation with One AirPod (any ANC-capable model)
- **Use One AirPod**: a daemon setting, remembered per device. With it on, playback keeps going
  while one bud is in and the other sits in the case; only the last bud out pauses
- **Ear detection** status in the header
- **Autoconnect**: pressing play here while wearing the AirPods connects them to this machine.
  If they were playing here when they went into the case and the connection drops, the daemon
  reconnects as they come out and claims them
- **Stem press media controls** (play/pause, next/prev) wired through MPRIS
- **Device renaming**: sets both the AACP name and the BlueZ alias
- **Volume swipe synced** to system volume via configurable commands; your Volume Swipe on/off choice is remembered per device and re-applied on connect
- **Auto audio rerouting** to the AirPods sink when playback starts or the buds go in your ears
- **Automatic iPhone ↔ Linux handoff**: pauses local media when an Apple device takes audio ownership and reclaims the audio session once the peer stops playing (playback stays paused until you press play). Pressing play here takes the AirPods back from the iPhone
- **Battery and in-ear state without a connection**, decoded from Apple's BLE proximity advertisements: the daemon keeps reporting while the buds sit in the case or belong to your iPhone, where the control channel has nothing to say. Battery comes back per-percent rather than in 10% steps
- **Waybar integration** via JSON output (`--waybar` / `--waybar-watch`)
- **Background daemon** with Unix-socket IPC so the TUI launches instantly
- **28 Apple/Beats models** with per-model capability detection; unknown Apple devices fall back to safe defaults

## Installation

### Arch / Omarchy (AUR)

```bash
yay -S airpods-tui-bin    # prebuilt x86_64 binary, fastest
# or
yay -S airpods-tui-git    # builds from latest main
```

Both packages run an install hook that:

- installs the binary to `/usr/bin/airpods-tui`
- drops the systemd user unit at `/usr/lib/systemd/user/airpods-tui.service`
- adds `DeviceID = bluetooth:004C:0000:0000` under `[General]` in `/etc/bluetooth/main.conf` (removed on uninstall)

The DeviceID makes BlueZ identify itself as an Apple host. Without it, AirPods still pair and play audio (A2DP works fine), but they refuse to open the AACP control channel, which is what every feature in this tool runs over. So plain music playback works without it, but battery, noise mode, settings, ear detection, etc. all stay blank.

### From source

Needs Rust 1.89 or newer. If your distribution ships an older `rustc` (Debian 13
has 1.85), install a current toolchain with [rustup](https://rustup.rs) instead of
pinning dependencies back.

Build dependencies:

```bash
# Arch
sudo pacman -S --needed rust libpulse dbus pkgconf
# Debian / Ubuntu
sudo apt install pkg-config libpulse-dev libdbus-1-dev
```

```bash
git clone https://github.com/annoyedmilk/airpods-tui.git
cd airpods-tui
cargo build --release
sudo install -Dm755 target/release/airpods-tui /usr/bin/airpods-tui
sudo install -Dm644 airpods-tui.service /usr/lib/systemd/user/airpods-tui.service
```

This path does **not** run the install hook, see [Apple DeviceID setup](#apple-deviceid-setup) below.

### Apple DeviceID setup

Required for AACP. Skip if you installed via the AUR, the package hook already did it.

```bash
sudo sed -i '/^\[General\]/a DeviceID = bluetooth:004C:0000:0000' /etc/bluetooth/main.conf
sudo systemctl restart bluetooth
```

If your AirPods were paired *before* adding the DeviceID, forget and re-pair them so they handshake against an Apple-identified host:

```bash
bluetoothctl remove <AIRPODS_MAC>
```

Open the AirPods case, hold the button on the back until the LED flashes white, then re-pair via Bluetooth settings or `bluetoothctl`.

### Enable the daemon

```bash
systemctl --user daemon-reload
systemctl --user enable --now airpods-tui.service
```

The daemon owns the AACP session so the TUI launches instantly via the IPC socket. Logs: `journalctl --user -u airpods-tui`.

### Floating window (Hyprland / Omarchy, optional)

Omarchy launches its own TUIs (bluetui, impala, btop) as centered floating
windows. To get the same for airpods-tui, add to `~/.config/hypr/hyprland.conf`:

```ini
windowrule = float on, match:class org.omarchy.airpods-tui
windowrule = center on, match:class org.omarchy.airpods-tui
windowrule = size 615 486, match:class org.omarchy.airpods-tui
```

The size fits the TUI (82×28 cells) at Omarchy's default terminal font
(JetBrainsMono 9pt, 14px padding); adjust if you use a different font or size.
Don't use Omarchy's `tag +floating-window` shortcut here, because its generic
`size 875 600` rule is applied in a later pass and overrides any
class-matched size.

Launch or focus the window with the stock Omarchy helper:

```bash
omarchy-launch-or-focus-tui airpods-tui
```

### Waybar module (optional)

Add to `~/.config/waybar/config.jsonc` modules list:

```jsonc
"custom/airpods": {
    "exec": "airpods-tui --waybar-watch",
    "return-type": "json",
    "format": "󰎈 {}",
    "on-click": "omarchy-launch-or-focus-tui airpods-tui"
}
```

Add `"custom/airpods"` to your bar's `modules-right` (or wherever you prefer) and restart Waybar:

```bash
omarchy restart waybar
```

The module's `class` is `connected` while the daemon holds a session, and `nearby` when the AirPods are only seen through their broadcasts (in the case, or in use by your phone), so you can style the two apart. The tooltip includes the case lid while it is known.

For scripts that don't want to parse JSON, every battery update is also written to `$XDG_RUNTIME_DIR/airpods-battery.env` as `LEFT=`/`RIGHT=`/`CASE=`/`HEADPHONE=` lines.

## Usage

```
airpods-tui                 # launch TUI
airpods-tui --daemon        # headless background daemon (no TUI)
airpods-tui --waybar        # print one-shot JSON status and exit
airpods-tui --waybar-watch  # persistent JSON output on every change
airpods-tui -d              # debug logging (visible in journalctl)
airpods-tui -v              # show version and exit
```

## Keys

| Key | Action |
|-----|--------|
| `q` / `Ctrl+C` | Quit |
| `Tab` / `Shift+Tab` | Cycle section (Noise Control / Settings) |
| `↑` / `↓` | Navigate rows in current section |
| `←` / `→` | Adjust slider/enum in Settings; switch device tab in Noise Control |
| `Space` / `Enter` | Toggle / select focused row |
| `1` / `2` / `3` | Noise mode shortcut (Transparency / Adaptive / Noise Cancellation) |
| `c` | Toggle Conversation Awareness |
| `r` | Rename device |
| `i` | Show device info popup (model, firmware, serial) |

## Configuration

Optional config at `~/.config/airpods-tui/config.toml`:

```toml
# Pop the volume OSD after a stem swipe ({} receives "+0": display only,
# the volume itself is applied by volume_set_command)
volume_osd_command = ["swayosd-client", "--output-volume", "{}"]

# Apply absolute volume ({} is replaced with a 0.0 to 1.0 fraction)
volume_set_command = ["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "{}"]

# Battery-low desktop notification at 20% and 10%, sent by the daemon
# ({} is replaced with "Left battery: 18%" etc.)
battery_alert_command = ["notify-send", "AirPods", "{}"]

# Optional: restart the audio server when the AirPods' card shows up without
# an A2DP profile. Off by default.
# restart_audio_server = ["systemctl", "--user", "restart", "wireplumber"]

# Optional: card profile used for playback. Unset, the daemon picks the
# highest-priority A2DP profile, the one WirePlumber would choose (AAC on AirPods).
# a2dp_profile = "a2dp-sink-sbc_xq"

# Watch BLE proximity advertisements for state while the control channel is down
ble_scan = true

# Connect the AirPods when you press play here while wearing them (needs ble_scan)
auto_connect = true
```

Set any command to `[]` to disable that integration. `restart_audio_server` and
`a2dp_profile` default to unset.

### Codec

When the buds go in, the daemon switches the card to A2DP and reroutes audio to it.
Releases up to 0.3.2 forced SBC-XQ there. Measured on AirPods Pro 3 with an Intel
AX200, SBC-XQ sent 511 kbps against 285 kbps for AAC, with every frame split across
two HCI packets. WirePlumber remembers that choice, so it can outlive the daemon.
Current releases move the card back to AAC on the next activation; to do it by hand:

```bash
pactl set-card-profile bluez_card.XX_XX_XX_XX_XX_XX a2dp-sink
```

Volume and battery-alert commands run asynchronously with a five-second timeout.
Timed-out commands are terminated and logged; volume updates keep the latest pending
target while a command is running.

### BLE proximity scanning

With `ble_scan = true` (the default) the daemon watches Apple's proximity
advertisements, so battery and in-ear state stay live even when the AACP control
channel is down: buds in the case, or currently owned by your phone.

AirPods address these broadcasts with a private address that rotates every few
minutes, so they can only be attributed to a device whose identity key we hold.
That key arrives over AACP, which means **a device must complete one normal
connection before its advertisements resolve**; until then the daemon logs
`Unattributed proximity advertisement` at debug level, and no scan runs at all
until at least one device has stored keys.

The scan only runs while none of your AirPods is connected to this machine. While
one is, AACP reports the same state with more authority, and an LE scan would
take airtime from the audio stream. It resumes once they disconnect.

Set `ble_scan = false` to skip LE scanning entirely; autoconnect relies on it.

## Dependencies

Runtime:

- **BlueZ**: D-Bus interface to Bluetooth
- **libpulse**: PulseAudio client lib (also used to control PipeWire's pulse compatibility layer)
- **dbus**

Optional:

- **PipeWire + WirePlumber** (`wpctl`) for volume control
- **SwayOSD** (`swayosd-client`) for the volume overlay
- **libnotify** (`notify-send`) for battery alerts

## License

GPL-3.0-or-later
