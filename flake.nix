{
  inputs.rustdev-nix.url = "github:yutadev31/rustdev-nix";

  outputs =
    inputs:
    inputs.rustdev-nix.lib.mkFlake {
      inherit inputs;
      src = ./.;

      packageAttrs = { pkgs, ... }: {
        cargoLock = {
          outputHashes = {
            "shell-surface-0.1.0" = "sha256-GwfvexzP2rXtpI0txXHNlKnZ/VNot9kieEY/FMnIimo=";
          };
        };

        preCheck = ''
          export FONTCONFIG_FILE="${pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; }}"
        '';

        nativeBuildInputs = [
          pkgs.pkg-config
        ];

        buildInputs = [
          pkgs.libpulseaudio
          pkgs.libxkbcommon
          pkgs.wayland
        ];
      };
    };
}
