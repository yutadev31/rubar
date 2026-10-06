{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        rust = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
          ];
        };

        devTools = with pkgs; [
          rust
          taplo
          nixd
          nixfmt
          typos-lsp
        ];

        nativeBuildInputs = with pkgs; [
          pkg-config
        ];

        buildInputs = with pkgs; [
          libpulseaudio
          libxcb
          wayland
        ];

        rubar = pkgs.rustPlatform.buildRustPackage {
          pname = "rubar";
          version = "0.1.0";
          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;

            outputHashes = {
              "shell-surface-0.1.0" = "sha256-QUrfvyEuGP1zTxIV6EE+gTD4cu3Y5dl5EApJmJjKx9s=";
            };
          };

          inherit nativeBuildInputs buildInputs;
        };
      in
      {
        packages = {
          inherit rubar;
          default = rubar;
        };

        devShells.default = pkgs.mkShell {
          nativeBuildInputs = nativeBuildInputs ++ devTools;
          inherit buildInputs;
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath buildInputs;
        };
      }
    );
}
