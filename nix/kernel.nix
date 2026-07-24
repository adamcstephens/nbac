# Guest kernel with CONFIG_KVM=y for nested virtualization. Built from the
# config copied off the running builder (KVM enabled) against the 6.18 LTS
# source, so it can only be realized on an aarch64-linux builder — nbac itself.
{
  linux_6_18,
  linuxManualConfig,
  runCommand,
}:
let
  kernel = linuxManualConfig {
    inherit (linux_6_18) version src;
    configfile = ./config;
  };
in
# Apple `container --kernel` wants a bare image file; arm64 installs it as Image.
runCommand "nbac-kernel" { } ''
  cp ${kernel}/Image $out
''
