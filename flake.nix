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
          tombi
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

        lockfile = builtins.fromTOML (builtins.readFile ./Cargo.lock);
        rubarPackage = builtins.head (builtins.filter (package: package.name == "rubar") lockfile.package);

        rubar = pkgs.rustPlatform.buildRustPackage {
          pname = "rubar";
          version = rubarPackage.version;
          src = ./.;

          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [
            "--package"
            "rubar"
          ];
          cargoTestFlags = [
            "--package"
            "rubar"
          ];

          inherit nativeBuildInputs buildInputs;

          preCheck = ''
            export FONTCONFIG_FILE="${
              pkgs.makeFontsConf {
                fontDirectories = [ pkgs.dejavu_fonts ];
              }
            }"
          '';
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
