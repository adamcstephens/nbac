{ lib, ... }:
{
  options.services.nbac = {
    enable = lib.mkEnableOption "nbac, an on-demand aarch64-linux remote builder on Apple container";

    containerPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = null;
      example = lib.literalExpression "pkgs.container";
      description = ''
        Apple `container` package to install. The default of null installs
        nothing and expects a system-wide `container` (Apple's pkg or
        Homebrew) on PATH.
      '';
    };
  };
}
