{ lib, ... }:
{
  options.services.nbac = {
    enable = lib.mkEnableOption "nbac, an on-demand aarch64-linux remote builder on Apple container";
  };
}
