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
          revopoint-photo-turntable = (build pkgs);
        in
        {
          inherit revopoint-photo-turntable;
          default = revopoint-photo-turntable;
        }
      );

      formatter = forEachSystem ({ pkgs, system }: pkgs.nixfmt-tree);

      devShells = forEachSystem (
        { pkgs, system }:
        {
          default = pkgs.mkShellNoCC {
            inputsFrom = [ (build pkgs) ];
          };
        }
      );
    };
}
