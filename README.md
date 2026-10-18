# rubar

`rubar` is a configurable status bar for Wayland, written in Rust. It creates a bar at the top of each output and supports workspaces, the focused window, a clock, battery, CPU and memory usage, audio volume, and a system tray.

Workspace and window information is available through Hyprland, Sway, i3, or niri IPC. System metrics use Linux interfaces, audio uses PulseAudio, and tray icons use the StatusNotifierItem protocol.

## Build and run

The Nix development shell provides the Rust toolchain and native build dependencies:

```sh
nix develop
cargo run
```

To build a release binary:

```sh
cargo build --release
./target/release/rubar
```

The Nix flake also exposes a `rubar` package, which you can run with `nix run .`.

When building outside Nix, install a recent Rust toolchain, `pkg-config`, and development libraries for PulseAudio, XCB, and Wayland. Run `rubar` inside a Wayland session.

## Configuration

Create `~/.config/rubar/config.toml` (or `$XDG_CONFIG_HOME/rubar/config.toml` when `XDG_CONFIG_HOME` is set). For example:

```toml
[modules]
left = ["workspace", "window"]
center = ["clock"]
right = ["cpu", "memory", "battery", "volume", "tray"]

[clock]
format = "%a %d %b %H:%M"
# timezone = "Asia/Tokyo"

[style]
height = 30
font_size = 14

[style.colors]
bg = "#24283b"
text = "#c0caf5"
```

The `left`, `center`, and `right` arrays set the widgets and their order. All three arrays are empty by default, so add the widgets you want to display. Missing settings use built-in defaults. Changes to the configuration file are reloaded while the bar is running; if a reload fails, the current settings remain active.

## Development

```sh
cargo fmt --check
cargo check
cargo clippy
cargo test
```

Set `RUST_LOG=debug` when running `rubar` to see configuration and provider diagnostics.
