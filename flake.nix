{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    utils.url = "github:numtide/flake-utils";
    naersk = {
      url = "github:nix-community/naersk";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, utils, naersk }:
    utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        naersk-lib = pkgs.callPackage naersk { };
        # Libraries eframe/winit dlopen at runtime (wayland/x11 + GL).
        guiLibs = with pkgs; [
          fontconfig libGL libxkbcommon wayland
          libx11 libxcb libxcb-util libxcb-cursor libxcb-image
          libxcb-keysyms libxcb-render-util libxcb-wm
        ];
      in
      {
        packages.default = naersk-lib.buildPackage {
          src = ./.;
          nativeBuildInputs = [ pkgs.pkg-config pkgs.makeWrapper ];
          buildInputs = [ pkgs.udev ] ++ guiLibs;
          postInstall = ''
            wrapProgram $out/bin/little_oil \
              --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath guiLibs}
          '';
        };
        devShells.default = pkgs.mkShell {
          buildInputs = (with pkgs; [
            cargo rustc rustfmt pre-commit rustPackages.clippy
            pkg-config udev bacon rust-analyzer
            # tests/integration/run_sway.sh: headless compositor, capture,
            # virtual-keyboard Ctrl, and a JSON/PPM parser.
            sway grim wtype python3
          ]) ++ guiLibs;
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath guiLibs;
          RUST_SRC_PATH = pkgs.rustPlatform.rustLibSrc;
        };
      }
    );
}
