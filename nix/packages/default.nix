{ pkgs, ... }:
let
  cargo = builtins.fromTOML (builtins.readFile ../../Cargo.toml);
  pname = cargo.package.name;
  lockfile = builtins.fromTOML (builtins.readFile ../../Cargo.lock);
  package = builtins.head (builtins.filter (package: package.name == pname) lockfile.package);
in
pkgs.rustPlatform.buildRustPackage {
  inherit pname;
  inherit (package) version;
  src = ../../.;
  cargoLock = {
    lockFile = ../../Cargo.lock;
    outputHashes = {
      "shell-surface-0.1.0" = "sha256-GwfvexzP2rXtpI0txXHNlKnZ/VNot9kieEY/FMnIimo=";
    };
  };

  nativeBuildInputs = [
    pkgs.pkg-config
  ];

  buildInputs = [
    pkgs.libpulseaudio
    pkgs.libxkbcommon
    pkgs.wayland
  ];
}
