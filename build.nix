{
  lib,
  makeRustPlatform,
  rust-bin,
  cmake,
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
  pname = "photo-turntable";
  version = "0.1.0";

  src = ./photo-turntable;
  cargoHash = "sha256-ajkQwrXQAsKS7XXpvnaR+OQI7MfPUHjt0/ARfEbXagQ=";

  buildInputs = [
    libgphoto2
  ];

  nativeBuildInputs = [
    pkg-config
    cmake
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
