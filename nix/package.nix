{
  drowse,
  lib,
  manifest,
  stdenv,
}:

drowse.lib.${stdenv.hostPlatform.system}.crate2nix {
  pname = manifest.package.name;
  inherit (manifest.package) version;

  src =
    with lib.fileset;
    toSource {
      root = ../.;
      fileset = unions [
        ../Cargo.toml
        ../Cargo.lock
        ../src
      ];
    };

  meta.mainProgram = "nbac";
}
