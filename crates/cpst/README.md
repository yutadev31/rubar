# cpst

`cpst` is a small, cross-platform command-line toolkit for interacting with
the system clipboard.

It provides two commands:

- `cpy` copies UTF-8 text or a PNG image from standard input to the system clipboard.
- `pst` writes the system clipboard contents to standard output.

## Installation

Install from the workspace root with Cargo:

```sh
cargo install --path crates/cpst
```

If you have Nix installed, build the package with:

```sh
nix build .#cpst
./result/bin/cpy --help
./result/bin/pst --help
```

You can also enter the development shell with `nix develop`.

## Usage

Copy text to the clipboard:

```sh
printf 'hello, clipboard!\n' | cpy
```

Copy a screenshot on Wayland:

```sh
grim - | cpy
```

Paste the clipboard contents:

```sh
pst
```

Both commands use the regular system clipboard. `cpy` reads all input
from standard input and accepts UTF-8 text or PNG images; `pst` writes
clipboard text without adding a newline.

## Platform support

The system clipboard is supported on platforms supported by
[`arboard`](https://docs.rs/arboard).

On Linux, clipboard access requires a working graphical session and the
appropriate X11 or Wayland runtime. When building from source, the Nix
development shell and CI configuration provide the Wayland development
dependency.

## Development

Enter the Nix development shell from the workspace root if Nix is available:

```sh
nix develop
```

Then format, check, lint, test, and build the package:

```sh
cargo fmt --all --check
cargo check -p cpst --locked
cargo clippy -p cpst --all-targets --all-features --locked -- -D warnings
cargo test -p cpst --locked
cargo build -p cpst --locked
```

## License

This project is licensed under the MIT License. See
[`LICENSE.txt`](../../LICENSE.txt) for the full license text.
