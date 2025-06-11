{
  lib,
  makeRustPlatform,
  rust-bin,
  pkg-config,
  libgphoto2,
}:
let
  rustPlatform = makeRustPlatform {
    cargo = rust-bin.stable.latest.default;
    rustc = rust-bin.stable.latest.default;
  };
in
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "revopoint-photo-turntable";
  version = "0.1.0";

  src = ./photo-turntable;
  cargoHash = "sha256-SROiAZvXSpaGA5FPG0jze1Mf3IXCrmz0CFNtrRdqluw=";

  buildInputs = [
    libgphoto2
  ];

  nativeBuildInputs = [
    pkg-config
  ];

  meta = {
    description = "Utility for taking photos with the Revpoint Dual Axis Turntable";
    homepage = "https://github.com/conroy-cheers/photo-turntable";
    license = with lib.licenses; [
      mit
    ];
    maintainers = with lib.maintainers; [ ];
  };
})
