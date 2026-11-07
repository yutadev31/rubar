# rubar

`rubar` is a configurable status bar for Wayland and X11, written in Rust. It creates a bar at the top of each output and supports workspaces, the focused window, a clock, battery, CPU and memory usage, process counts, audio volume, and a system tray.

Workspace and window information is available through Hyprland, Sway, i3, niri, or MangoWM IPC. System metrics use native Linux, FreeBSD, or NetBSD interfaces, audio uses PulseAudio, and tray icons use the StatusNotifierItem protocol.

On MangoWM, the `workspace` widget shows tags on each monitor and supports clicking a tag to view it. The `window` widget shows the focused client title. Run `rubar` within the MangoWM session so it inherits `MANGO_INSTANCE_SIGNATURE`.

## Supported platforms

`rubar` currently supports Linux and FreeBSD on amd64. The long-term goal is to support platforms that run X11 or Wayland, including the BSD family, illumos, and GNU Hurd.

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

When building outside Nix, install a recent Rust toolchain, `pkg-config`, and development libraries for PulseAudio, Wayland, and xkbcommon. Run `rubar` inside a Wayland or X11 session.

### FreeBSD

Build with Cargo after installing the Rust toolchain, `pkgconf`, and the PulseAudio, Wayland, and xkbcommon libraries:

```sh
cargo build --release
```

The `cpu`, `memory`, `process`, and `battery` widgets select FreeBSD providers automatically. The battery provider reads the aggregate ACPI battery state; on systems without an ACPI battery, it reports the missing battery and leaves the widget empty. FreeBSD memory availability includes free and inactive pages. The volume widget requires a PulseAudio-compatible server, and the tray requires a D-Bus session. Workspace and window widgets require one of the supported window managers or compositors listed above.

### NetBSD

On NetBSD 10 amd64, install the `xbase` distribution set and a recent Rust toolchain, `pkgconf`, PulseAudio, Wayland, and xkbcommon from pkgsrc, then run `cargo build --release`. The `xbase` set provides `libxcb`, which pkgsrc's PulseAudio library requires. Run Rubar in a supported X11 or Wayland session. The volume widget requires a PulseAudio-compatible server and the tray requires a D-Bus session.

The `cpu`, `memory`, and `process` widgets select NetBSD providers automatically. CPU ticks, VM pages, and the process table come from `sysctl`. Memory availability includes free and inactive pages. The `battery` widget is unavailable on NetBSD.

## Configuration

Create `~/.config/rubar/config.toml` (or `$XDG_CONFIG_HOME/rubar/config.toml` when `XDG_CONFIG_HOME` is set). For example:

```toml
[widgets]
left = ["workspace", "window"]
center = ["clock"]
right = ["cpu", "memory", "process", "battery", "volume", "tray"]

[clock]
format = "%a %d %b %H:%M"
# timezone = "Asia/Tokyo"

[style]
height = 30
font_size = 14
widget_spacing = 12

[style.colors]
bg = "#24283b"
text = "#c0caf5"
```

The `left`, `center`, and `right` arrays set the widgets and their order. All three arrays are empty by default, so add the widgets you want to display. Missing settings use built-in defaults. Changes to the configuration file are reloaded while the bar is running; if a reload fails, the current settings remain active.

For existing configurations, rename `[modules]` to `[widgets]` and `style.module_spacing` to `style.widget_spacing`.

The `process` widget shows the total number of processes. It counts PID entries in `/proc` on Linux and queries the process table on FreeBSD and NetBSD. On Linux, a procfs mount with `hidepid=2` (or another mode that hides PID directories) makes the widget unavailable and logs an error instead of displaying a partial count. Its default format is `{count} 󰒓`, refreshed every five seconds. For example:

```toml
[process]
format = "Processes: {count}"
refresh_seconds = 5
# provider = "auto" # or "linux" / "freebsd" / "netbsd" on the corresponding platform
```

## Development

```sh
cargo fmt --check
cargo check
cargo clippy
cargo test
```

Set `RUST_LOG=debug` when running `rubar` to see configuration and provider diagnostics.
