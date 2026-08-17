{ writers }:
writers.writePython3Bin "bump-nix" { doCheck = false; } (builtins.readFile ./bump-nix.py)
