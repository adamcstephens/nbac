{
  installShellFiles,
  lib,
  nbac-unwrapped,
  makeWrapper,
  runCommand,
}:
runCommand "nbac"
  {
    nativeBuildInputs = [
      installShellFiles
      makeWrapper
    ];
    meta.mainProgram = "nbac";
  }
  ''
    mkdir -vp $out/bin/
    # nix-daemon's PATH lacks /usr/local/bin, where Apple `container` lives,
    # and the ProxyCommand runs under the daemon for distributed builds.
    makeWrapper ${lib.getExe nbac-unwrapped} $out/bin/nbac \
      --suffix PATH : /usr/local/bin

    installShellCompletion --cmd nbac \
      --bash <(COMPLETE=bash $out/bin/nbac) \
      --fish <(COMPLETE=fish $out/bin/nbac) \
      --zsh <(COMPLETE=zsh $out/bin/nbac)
  ''
