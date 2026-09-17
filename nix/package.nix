{
  craneLib,
  manifest,
}:

craneLib.buildPackage rec {
  pname = manifest.package.name;
  inherit (manifest.package) version;

  src = craneLib.cleanCargoSource ../.;
  strictDeps = true;

  cargoArtifacts = craneLib.buildDepsOnly {
    inherit src strictDeps;
  };

  meta.mainProgram = "nbac";
}
