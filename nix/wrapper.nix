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
    # nix-daemon's PATH has neither install location of Apple `container`
    # (/usr/local/bin for the signed pkg, /opt/homebrew/bin for the cask),
    # and the ProxyCommand runs under the daemon for distributed builds.
    makeWrapper ${lib.getExe nbac-unwrapped} $out/bin/nbac \
      --suffix PATH : /usr/local/bin \
      --suffix PATH : /opt/homebrew/bin

    installShellCompletion --cmd nbac \
      --bash <(COMPLETE=bash $out/bin/nbac) \
      --fish <(COMPLETE=fish $out/bin/nbac) \
      --zsh <(COMPLETE=zsh $out/bin/nbac)
  ''
