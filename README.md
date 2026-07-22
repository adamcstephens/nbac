# nbac

**N**ix **b**uilder on **A**pple **c**ontainer: an on-demand, idle-shutdown
`aarch64-linux` remote builder for Nix on macOS, managed by a single Rust CLI
and a thin nix-darwin module.

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
`stateDir`, and builder scheduling (`systems`, `maxJobs`, `speedFactor`,
`supportedFeatures`, `mandatoryFeatures`, `protocol`).

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
