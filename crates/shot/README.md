# shot

`shot` is a cross-platform Rust CLI tool that captures a selected monitor or all
connected monitors. Captures are saved as PNG files under
`Pictures/Screenshots` and copied to the system clipboard.

On Linux, it uses Wayland (`wlr-screencopy`) or X11 (RandR / XGetImage). On
Windows, it uses Win32/GDI. When multiple monitors are captured with `--all`,
they are stitched into a single image while preserving their layout.

## Installation

Install from a local checkout with Cargo:

```sh
cargo install --path crates/shot
```

With Nix, enter the development environment:

```sh
nix develop
```

## Usage

When run without arguments, `shot` lets you select a monitor in the terminal if
multiple monitors are available. Navigate with the arrow keys or
`h` / `j` / `k` / `l`, press Enter to capture, or press Esc or `q` to cancel.

```sh
# Select a monitor and capture it
shot

# Select by zero-based index
shot --monitor 1

# Select by Wayland output name or X11/Windows monitor name
shot --monitor HDMI-A-1

# Stitch all monitors into one image
shot --all

# Select a monitor with rofi
shot --rofi
```

`--monitor` accepts either an index or a name. By default, captures are saved to
`~/Pictures/Screenshots/screenshot-<timestamp>.png`. On success, `shot` prints
the output path and clipboard result to standard output.

## Configuration

Configure the output path and clipboard behavior in
`~/.config/shot/config.toml`:

```toml
[output]
path_format = "${pictures_dir}/Screenshots/screenshot-${timestamp}.png"
clipboard = true
```

If `path_format` is omitted, the capture is not saved to a file and is sent only
to the clipboard. An empty value has the same effect, with a warning. If both
`path_format` and `--file` are absent and `clipboard = false`, `shot` reports an
error before starting capture or monitor selection.

`path_format` supports `${pictures_dir}`, `${documents_dir}`,
`${downloads_dir}`, `${home_dir}`, and `${timestamp}`. You can specify
`--file` multiple times; when present, it takes precedence over `path_format`.
`--clipboard` overrides the configuration file. Its value defaults to `true`;
to disable clipboard copying, use `--clipboard false`.

For a full list of options:

```sh
shot --help
```

## Platform requirements

On Linux, `shot` uses Wayland when `WAYLAND_DISPLAY` is set; otherwise, it
uses X11 when `DISPLAY` is set.

- Wayland: Requires a compositor supporting `wlr-screencopy` and a
  `wayland-data-control` environment for clipboard access.
- X11: Requires a running X server and `DISPLAY`.
- Windows: No additional display server configuration is required.
- `--rofi`: Requires the `rofi` command for monitor selection.

## Backend features

The default build enables the Wayland, X11, and Windows features. To enable
only a specific backend, disable the default features:

```sh
# Linux: X11 only
cargo build -p shot --no-default-features --features x11

# Linux: Wayland only
cargo build -p shot --no-default-features --features wayland

# Windows: Win32/GDI only
cargo build -p shot --no-default-features --features windows
```

## Development

Run the following checks after making changes:

```sh
cargo fmt --all --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

Changes to the backends, clipboard, or PNG output should be verified by running
`shot` in an available graphical environment.

## License

This project is licensed under the MIT License. See
[`LICENSE.txt`](../../LICENSE.txt) for the full license text.
