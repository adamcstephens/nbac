{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable-small";

    flake-parts.url = "github:hercules-ci/flake-parts";

    nix-darwin = {
      url = "github:nix-darwin/nix-darwin";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    drowse = {
      url = "github:adamcstephens/drowse?ref=push-mvosvrtpsrpq";
      inputs = {
        flake-parts.follows = "flake-parts";
        nixpkgs.follows = "nixpkgs";
      };
    };
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
      ];

      flake.darwinModules = rec {
        default = nbac;
        nbac =
          { lib, pkgs, ... }:
          {
            imports = [ ./nix/module.nix ];
            services.nbac.package = lib.mkDefault inputs.self.packages.${pkgs.stdenv.hostPlatform.system}.nbac;
            services.nbac.virtualization.kernelPackage = lib.mkDefault inputs.self.packages.aarch64-linux.nbac-kernel;
          };
      };

      perSystem =
        {
          pkgs,
          lib,
          self',
          system,
          ...
        }:
        let
          manifest = lib.importTOML ./Cargo.toml;
          bump-nix = pkgs.callPackage ./nix/bump-nix.nix { };
        in
        {
          apps.bump-nix.program = "${bump-nix}/bin/bump-nix";

          devShells.default = pkgs.mkShell {
            packages = [
              pkgs.just
              pkgs.nixfmt

              pkgs.cargo
              pkgs.clippy
              pkgs.rustc
              pkgs.rust-analyzer
              pkgs.rustfmt
            ];
          };

          formatter = pkgs.nixfmt;

          packages = rec {
            default = nbac;

            nbac = pkgs.callPackage ./nix/wrapper.nix {
              inherit nbac-unwrapped;
            };

            nbac-unwrapped = pkgs.callPackage ./nix/package.nix {
              inherit manifest;
              inherit (inputs) drowse;
            };
          }
          // lib.optionalAttrs (system == "aarch64-linux") {
            nbac-kernel = pkgs.callPackage ./nix/kernel.nix { };
          };

          checks = {
            build = self'.packages.nbac;

            clippy = self'.packages.nbac-unwrapped.overrideAttrs (old: {
              pname = "nbac-clippy";
              nativeBuildInputs = (old.nativeBuildInputs or [ ]) ++ [ pkgs.clippy ];
              buildPhase = ''
                runHook preBuild
                cargo clippy --all-targets -- --deny warnings
                runHook postBuild
              '';
              doCheck = false;
              installPhase = "touch $out";
              doInstallCheck = false;
            });

            formatting =
              pkgs.runCommand "nbac-formatting"
                {
                  nativeBuildInputs = [
                    pkgs.cargo
                    pkgs.rustfmt
                    pkgs.nixfmt
                  ];
                }
                ''
                  cd ${inputs.self}
                  HOME=$TMPDIR cargo fmt --check
                  nixfmt --check $(find . -name '*.nix')
                  touch $out
                '';
          }
          // lib.optionalAttrs (system == "aarch64-darwin") {
            module-eval =
              let
                darwin = inputs.nix-darwin.lib.darwinSystem {
                  modules = [
                    inputs.self.darwinModules.default
                    {
                      services.nbac = {
                        enable = true;
                        stateDir = "/var/lib/nbac";
                      };
                      nixpkgs.hostPlatform = "aarch64-darwin";
                      system.stateVersion = 6;
                    }
                  ];
                };
              in
              pkgs.runCommand "nbac-module-eval" {
                toplevel = builtins.unsafeDiscardStringContext darwin.config.system.build.toplevel.drvPath;
              } "echo $toplevel > $out";
          };
        };
    };
}
