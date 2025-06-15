{
  description = "Utility for taking photos with the Revpoint Dual Axis Turntable";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
    }:
    let
      allSystems = [
        "aarch64-linux"
        "x86_64-linux"
        "aarch64-darwin"
        "x86_64-darwin"
      ];

      forEachSystem =
        f:
        nixpkgs.lib.genAttrs allSystems (
          system:
          f {
            inherit system;
            pkgs = import nixpkgs {
              inherit system;
              overlays = [
                rust-overlay.overlays.default
                (self: super: {
                  rustToolchain = self.rust-bin.stable.latest.default.override { extensions = [ "rust-src" ]; };
                })
              ];
            };
          }
        );

      build = pkgs: pkgs.callPackage ./build.nix { };
    in
    {
      packages = forEachSystem (
        { pkgs, system }:
        let
          photo-turntable = (build pkgs);
        in
        {
          inherit photo-turntable;
          default = photo-turntable;
        }
      );

      formatter = forEachSystem ({ pkgs, system }: pkgs.nixfmt-tree);

      devShells = forEachSystem (
        { pkgs, system }:
        {
          default = pkgs.mkShellNoCC {
            inputsFrom = [ (build pkgs) ];
            packages = with pkgs; [
              pkg-config
              clippy
            ];
            env = {
              RUST_BACKTRACE = "1";
              RUST_SRC_PATH = "${pkgs.rustToolchain}/lib/rustlib/src/rust/library";
            };
          };
        }
      );
    };
}
