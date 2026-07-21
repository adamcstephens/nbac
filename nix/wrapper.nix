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
    makeWrapper ${lib.getExe nbac-unwrapped} $out/bin/nbac --prefix PATH : ${
      lib.makeBinPath [
        # Add runtime tool dependencies here, e.g. pkgs.git
      ]
    }

    installShellCompletion --cmd nbac \
      --bash <(COMPLETE=bash $out/bin/nbac) \
      --fish <(COMPLETE=fish $out/bin/nbac) \
      --zsh <(COMPLETE=zsh $out/bin/nbac)
  ''
