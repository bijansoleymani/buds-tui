# pixelbuds-macos

> **Vendored copy, modified.** This is
> [pixelbuds-tui](https://github.com/bijansoleymani/pixelbuds-tui) with its
> Bluetooth layer ported to macOS. `src/link.rs` was rewritten against
> `btmac` (IOBluetooth RFCOMM) in place of `bluer` (BlueZ), and `bluer::Address`
> was replaced with `btmac::Address`; `app.rs` and `ui.rs` are unchanged, and
> `main.rs` gained the run-loop thread arrangement macOS requires. Everything
> below is upstream's README and describes the Linux original. See
> `docs/macos.md` in the repository root for the port.

Terminal UI for Google Pixel Buds Pro and Pixel Buds Pro 2: battery for each
bud and the case, noise control, EQ, balance and the device settings.

It talks to the buds over Maestro, the protocol the Pixel Buds Android app
uses, through the `maestro` crate from [qzed/pbpctrl](https://github.com/qzed/pbpctrl).
The device handling follows the daemon in
[alinuxfan/omarchy-pixelbuds](https://github.com/alinuxfan/omarchy-pixelbuds).

Tested on Pixel Buds Pro 2 (firmware release_5.203).

## Build

Needs Rust, D-Bus headers and the protobuf compiler:

```bash
sudo apt install pkg-config libdbus-1-dev protobuf-compiler   # Debian / Ubuntu
cargo build --release
sudo install -Dm755 target/release/pixelbuds-tui /usr/bin/pixelbuds-tui
```

## Usage

```bash
pixelbuds-tui                       # first connected Pixel Buds
pixelbuds-tui --device FC:91:...    # a specific pair
```

The TUI never connects the buds itself, so opening it does not pull them
away from a phone. Connect them as usual and it attaches within a couple of
seconds.

| Key | Action |
|-----|--------|
| `↑` `↓` / `j` `k` | move between rows |
| `←` `→` / `h` `l` | change the value (ANC mode, balance, EQ band, switch) |
| `Enter` / `Space` | toggle a switch, or step to the next ANC mode |
| `1`–`4` | ANC Off / Active / Transparency / Adaptive |
| `r` | on Balance or an EQ band: reset to centered / flat |
| `i` | device address and firmware versions |
| `q` / `Esc` | quit |

The case battery is only reported while a bud sits in the case. After the
buds come out, the last reading stays on screen marked "last seen".

## License

GPL-3.0-or-later, see [LICENSE](LICENSE). Parts of `src/link.rs` are adapted
from pbpctrl and omarchy-pixelbuds under the MIT License; their notices are
in [NOTICE](NOTICE).
