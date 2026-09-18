{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.nbac;
  settingsFormat = pkgs.formats.toml { };

  installable = spec: if lib.hasInfix "#" spec then spec else "${cfg.image.packagesFlake}#${spec}";

  configToml = settingsFormat.generate "nbac-config.toml" {
    machine = {
      inherit (cfg.machine) name cpus memory;
    }
    // lib.optionalAttrs cfg.rosetta.enable {
      rosetta = true;
    }
    // lib.optionalAttrs cfg.virtualization.enable {
      virtualization = true;
      kernel = toString cfg.virtualization.kernelPackage;
    };
    image = {
      containerfile = "${cfg.image.containerfile}";
      packages = map installable cfg.image.packages;
    }
    // lib.optionalAttrs (cfg.image.buildContext != null) {
      build_context = "${cfg.image.buildContext}";
    };
    ssh = {
      inherit (cfg.ssh) user port;
      host_alias = cfg.ssh.hostAlias;
    };
    state.dir = cfg.stateDir;
    home = lib.optionalAttrs cfg.home.enable {
      user = cfg.home.user;
    };
    idle = {
      inherit (cfg.idle) enable;
      timeout_seconds = cfg.idle.timeoutSeconds;
    };
    runtime.container_binary =
      if cfg.containerPackage != null then lib.getExe cfg.containerPackage else "container";
  };
in
{
  options.services.nbac = {
    enable = lib.mkEnableOption "nbac, an on-demand aarch64-linux remote builder on Apple container";

    package = lib.mkOption {
      type = lib.types.package;
      description = "The nbac package.";
    };

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

    machine = {
      name = lib.mkOption {
        type = lib.types.str;
        default = "nbac";
        description = "Name of the container machine.";
      };

      cpus = lib.mkOption {
        type = lib.types.ints.positive;
        default = 4;
        description = "Number of virtual CPUs.";
      };

      memory = lib.mkOption {
        type = lib.types.str;
        default = "6G";
        description = "Memory allocation, with an optional K/M/G/T/P suffix.";
      };
    };

    rosetta = {
      enable = lib.mkEnableOption ''
        x86_64-linux builds via Rosetta. The image and machine switch to the
        linux/amd64 platform — the only mode in which `container` attaches
        Rosetta — while the guest Nix stays native aarch64-linux and gains
        `extra-platforms = x86_64-linux`. Toggling recreates the machine,
        which deletes its /nix store'';
    };

    virtualization = {
      enable = lib.mkEnableOption ''
        nested virtualization in the builder VM (KVM). Requires Apple Silicon
        M3+ and macOS 15+, and building the KVM-enabled guest kernel on the
        builder itself — bring nbac up first, then enable this and rebuild.
        The builder must advertise the `big-parallel` feature to schedule the
        kernel build (see `supportedFeatures`)'';

      kernelPackage = lib.mkOption {
        type = lib.types.package;
        defaultText = lib.literalExpression "nbac.packages.aarch64-linux.nbac-kernel";
        description = ''
          aarch64-linux kernel image built with `CONFIG_KVM=y`, passed to
          `container machine create --kernel`. Only realized when
          `virtualization.enable` is set. The flake provides the default.
        '';
      };
    };

    image = {
      containerfile = lib.mkOption {
        type = lib.types.path;
        default = ../images/Containerfile;
        description = "Containerfile the builder image is built from.";
      };

      buildContext = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = ../images;
        description = "Build context directory; null uses the Containerfile's directory.";
      };

      packages = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        example = [
          "git"
          "ripgrep"
        ];
        description = ''
          Extra packages installed into the guest profile
          `/nix/var/nix/profiles/nbac`, on PATH for `nbac ssh` logins.
          Attribute names are resolved against `image.packagesFlake`; a value
          containing `#` is taken as a full flake installable. The image build
          substitutes them inside the guest, so they must build for
          aarch64-linux. Distributed builds do not see them. Changing the list
          rebuilds the image, which recreates the machine and deletes its
          /nix store.
        '';
      };

      packagesFlake = lib.mkOption {
        type = lib.types.str;
        example = "github:nixos/nixpkgs/nixos-unstable-small";
        defaultText = lib.literalExpression ''"github:nixos/nixpkgs/''${nixpkgs.rev}"'';
        description = ''
          Flake reference the bare names in `image.packages` are resolved
          against, evaluated inside the guest. The flake pins this to the
          nixpkgs revision nbac itself is locked to.
        '';
      };
    };

    ssh = {
      user = lib.mkOption {
        type = lib.types.str;
        default = "builder";
        description = "SSH user inside the guest.";
      };

      port = lib.mkOption {
        type = lib.types.port;
        default = 22;
        description = "Guest sshd port; never published, only reached over the stdio transport.";
      };

      hostAlias = lib.mkOption {
        type = lib.types.str;
        default = "nbac";
        description = "SSH host alias used by nix.buildMachines and the client config.";
      };
    };

    home = {
      enable = lib.mkEnableOption ''
        mounting the host user's home directory into the machine. `nbac ssh`
        then logs in as that user rather than the builder account, and the
        guest trusts them as a Nix user. Distributed builds keep using the
        builder account either way. Toggling recreates the machine, which
        deletes its /nix store'';

      user = lib.mkOption {
        type = lib.types.str;
        default = config.system.primaryUser;
        defaultText = lib.literalExpression "config.system.primaryUser";
        description = ''
          Host user whose home is mounted. `container` derives the guest
          account from the user owning the runtime, so this must be the user
          running `nbac`.
        '';
      };
    };

    idle = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Power the machine off after a period with no SSH connections.";
      };

      timeoutSeconds = lib.mkOption {
        type = lib.types.ints.positive;
        default = 300;
        description = "Seconds without SSH connections before powering off.";
      };
    };

    stateDir = lib.mkOption {
      type = lib.types.str;
      default = "${config.system.primaryUserHome}/.local/state/nbac";
      example = "/Users/me/.local/state/nbac";
      description = ''
        Directory for keys, known_hosts, the generation marker, and the lock.
        Must be writable by the user running `nbac setup`.
      '';
    };

    systems = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ "aarch64-linux" ] ++ lib.optional cfg.rosetta.enable "x86_64-linux";
      defaultText = lib.literalExpression ''[ "aarch64-linux" ] ++ lib.optional config.services.nbac.rosetta.enable "x86_64-linux"'';
      description = "Systems the builder is registered for.";
    };

    maxJobs = lib.mkOption {
      type = lib.types.ints.positive;
      default = cfg.machine.cpus;
      defaultText = lib.literalExpression "config.services.nbac.machine.cpus";
      description = "Maximum parallel builds scheduled on the builder.";
    };

    speedFactor = lib.mkOption {
      type = lib.types.ints.positive;
      default = 1;
      description = "Relative speed factor for build scheduling.";
    };

    supportedFeatures = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      example = [
        "big-parallel"
      ];
      description = ''
        Features the builder supports.
      '';
    };

    mandatoryFeatures = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      description = "Features a derivation must require to be scheduled here.";
    };

    protocol = lib.mkOption {
      type = lib.types.str;
      default = "ssh-ng";
      description = ''
        Builder protocol. The guest's sudo rule only covers
        `nix-daemon --stdio`, i.e. ssh-ng.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = pkgs.stdenv.hostPlatform.system == "aarch64-darwin";
        message = "services.nbac only supports aarch64-darwin";
      }
      {
        assertion = !(cfg.rosetta.enable && cfg.virtualization.enable);
        message = ''
          services.nbac.rosetta and services.nbac.virtualization are mutually
          exclusive: `container` (as of 1.2.0) assumes a custom kernel matches
          the machine platform (linux/amd64) and fails to boot the machine.
        '';
      }
    ];

    services.nbac.supportedFeatures = [
      "ca-derivations"
      "recursive-nix"
      "uid-range"
    ];

    environment.systemPackages = [
      cfg.package
    ]
    ++ lib.optional (cfg.containerPackage != null) cfg.containerPackage;

    environment.etc."nbac/config.toml".source = configToml;

    environment.etc."ssh/ssh_config.d/100-nbac.conf".text = ''
      Host ${cfg.ssh.hostAlias}
        User ${cfg.ssh.user}
        Port ${toString cfg.ssh.port}
        IdentityFile ${cfg.stateDir}/builder_ed25519
        IdentitiesOnly yes
        UserKnownHostsFile ${cfg.stateDir}/known_hosts
        StrictHostKeyChecking yes
        ProxyCommand ${lib.getExe cfg.package} --config /etc/nbac/config.toml proxy
    '';

    # `container` reports neither the virtualization flag nor the kernel of an
    # existing machine, so settings that only apply at `machine create` time
    # cannot be reconciled by inspecting it. Deleting the machine is enough:
    # the next build's cold path recreates it from the current config. The
    # plist embeds the config's store path, so nix-darwin reloads the agent
    # exactly when a setting changes; the stamp keeps the login-time
    # RunAtLoad from deleting the guest store every time.
    launchd.user.agents.nbac-reconcile = {
      # Apple `container` installs outside the store, in one of its two
      # installer locations, and agents inherit no useful PATH.
      path = [
        pkgs.coreutils
        "/usr/local/bin"
        "/opt/homebrew/bin"
      ]
      ++ lib.optional (cfg.containerPackage != null) cfg.containerPackage;

      script = ''
        set -e
        stamp='${cfg.stateDir}/spec'
        if [ "$(cat "$stamp" 2>/dev/null)" = '${configToml}' ]; then
          exit 0
        fi

        container system start || true
        if container machine inspect '${cfg.machine.name}' > /dev/null 2>&1; then
          echo 'nbac settings changed; deleting machine ${cfg.machine.name} and its /nix store' >&2
          container machine stop '${cfg.machine.name}' || true
          container machine delete '${cfg.machine.name}'
        fi

        mkdir --parents '${cfg.stateDir}'
        printf '%s' '${configToml}' > "$stamp"
      '';

      serviceConfig.RunAtLoad = true;
    };

    nix.distributedBuilds = true;
    nix.settings.builders-use-substitutes = true;
    nix.buildMachines = [
      {
        hostName = cfg.ssh.hostAlias;
        sshUser = cfg.ssh.user;
        sshKey = "${cfg.stateDir}/builder_ed25519";
        inherit (cfg)
          systems
          maxJobs
          speedFactor
          supportedFeatures
          mandatoryFeatures
          protocol
          ;
      }
    ];
  };
}
