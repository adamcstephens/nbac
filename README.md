# nbac

**N**ix **b**uilder on **A**pple **c**ontainer: an on-demand, idle-shutdown
`aarch64-linux` (and optionally `x86_64-linux`, via Rosetta) remote builder
for Nix on macOS, managed by a single Rust CLI and a thin nix-darwin module.

Run `nix build` for an `aarch64-linux` derivation and nbac boots a
lightweight VM through Apple's `container` runtime, builds there, and powers
the VM off again after a few idle minutes. No resident daemon, no published
ports, no manually managed builder.

## How it works

- The nix-darwin module registers a build machine reached through an SSH
  `ProxyCommand` that runs `nbac proxy`.
- On each connection the proxy inspects the machine once: if it is running
  the current image generation, it immediately connects over the host-only
  vmnet network. Otherwise it takes the cold path — build the image, create
  or recreate the machine, boot, inject SSH keys — and then connects.
- A watchdog inside the guest powers the machine off after a configurable
  period with no SSH connections.
- The guest image is built locally from a readable
  [Containerfile](images/Containerfile) (Alpine, upstream Nix from the
  checksum-verified static tarball, s6 as init). Nothing is pulled from a
  registry.
- The guest's `/nix` store persists across stop/start. The machine is
  recreated — deleting that store — only when the image generation changes
  (Containerfile, build context, or baked-in config), with a warning.

See [docs/spec.md](docs/spec.md) for the full design and decision log.

## Requirements

- An Apple silicon Mac (`aarch64-darwin`) running
  [nix-darwin](https://github.com/nix-darwin/nix-darwin).
- Apple's [`container`](https://github.com/apple/container) ≥ 1.1.0,
  installed system-wide (Apple's signed pkg or the Homebrew cask) and on
  `PATH`.

## Setup

Add nbac to your nix-darwin flake:

```nix
{
  inputs.nbac.url = "…";  # this repository

  # inside your darwinSystem modules:
  modules = [
    nbac.darwinModules.default
    {
      services.nbac.enable = true;
    }
  ];
}
```

After `darwin-rebuild switch`, run `nbac setup` once. It verifies the
runtime, generates the builder and host keys, builds the image, creates and
boots the machine, and checks readiness. (Skipping this is fine too — the
first build triggers the same cold path lazily.)

Useful options under `services.nbac`: `machine.{name,cpus,memory}`,
`idle.{enable,timeoutSeconds}`, `image.{containerfile,buildContext}`,
`stateDir`, `rosetta.enable`, `virtualization.enable`, and builder
scheduling (`systems`, `maxJobs`, `speedFactor`, `supportedFeatures`,
`mandatoryFeatures`, `protocol`).

## x86_64-linux via Rosetta

`services.nbac.rosetta.enable` (default off) registers the builder for
`x86_64-linux` too. Apple `container` attaches Rosetta only to machines
whose platform is `linux/amd64`, so the option builds the image and creates
the machine as that platform. The kernel is arm64 either way, and the image
keeps its native aarch64 Nix with `extra-platforms = x86_64-linux`:
`aarch64-linux` builds run at full native speed, `x86_64-linux` builds run
through Rosetta's binfmt handler. Rosetta must be installed on the host
(`softwareupdate --install-rosetta`). Toggling the option changes the image
generation, so the machine is recreated — deleting its guest `/nix` store —
with the usual warning.

Incompatible with `virtualization.enable` for now: `container` (as of 1.2.0)
assumes a custom kernel matches the machine platform and cannot boot an
amd64-platform machine with the (aarch64) KVM kernel; the module asserts
against the combination.

## Nested virtualization

`services.nbac.virtualization.enable` (default off) exposes `/dev/kvm` inside
the builder so it can itself run VMs — needed for `nixos-test`-style
derivations and KVM-accelerated cross builds. It requires Apple silicon M3+
and macOS 15+, and switches the machine to a custom `aarch64-linux` guest
kernel built with `CONFIG_KVM=y` (from the checked-in `config`, the running
builder's kernel config with KVM enabled) that nbac passes to `container
machine create --virtualization --kernel`.

That kernel can only be built on an `aarch64-linux` builder — nbac itself — so
bring nbac up with virtualization off first, then enable it. Because every
nixpkgs kernel build requires the `big-parallel` feature, the builder must
advertise it:

```nix
services.nbac.supportedFeatures = [
  "big-parallel"
];
```

Add that (and enough `machine.{cpus,memory}` to compile a kernel) in one
`darwin-rebuild switch`, then set `virtualization.enable = true` and rebuild
again; the second rebuild schedules the kernel build on the running builder.
Toggling virtualization on an existing machine takes effect after `nbac
reset`.

## Commands

| Command | Behavior |
| --- | --- |
| `nbac setup` | Idempotent preflight: keys, image, machine, boot, readiness. |
| `nbac status` | Concurrent read-only probes of the runtime, machine, guest sshd, and remote nix daemon; exits non-zero when unhealthy. |
| `nbac start` / `nbac stop` | Explicit lifecycle control. |
| `nbac reset` | Destroy and recreate the machine (confirms; deletes the guest `/nix` store). |
| `nbac ssh [args…]` | Interactive SSH into the builder. Guest logs live under `/var/log/nbac`. |
| `nbac completions <shell>` | Shell completion scripts. |

`nbac proxy` also exists, hidden: it is the SSH `ProxyCommand` everything
else hangs off.

## Standalone use

The module is optional sugar. nbac reads one TOML file
(`/etc/nbac/config.toml`, or `--config <path>`):

```toml
[image]
containerfile = "/path/to/Containerfile"

[state]
dir = "/Users/you/.local/state/nbac"

[machine]
cpus = 4
memory = "6G"

[idle]
timeout_seconds = 300
```

`image` and `state` are required; everything else has defaults. Custom
images must honor the contract described in the
[spec](docs/spec.md#default-image) (s6 supervision, an `sshd` service the
key injection can restart, a supervised `nix-daemon`, the watchdog).

## Security

- Keys are generated locally under the state directory (mode 0700), never
  in the Nix store; the guest host key is pinned via a generated
  `known_hosts` with `StrictHostKeyChecking yes`.
- The guest user has passwordless sudo for exactly one command:
  `nix-daemon --stdio`.
- No published ports: the guest sshd is only reachable from the host over
  the host-only vmnet interface.

## Development

```sh
nix develop        # rust toolchain, just, nixfmt
nix flake check    # build, clippy, rustfmt/nixfmt, module eval
```

Inspired by [nix-hex-box](https://github.com/robertderose/nix-hex-box); the
[spec](docs/spec.md) records what was kept and what was deliberately
rejected.
