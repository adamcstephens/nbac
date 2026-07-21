# nbac — Nix Builder on Apple Container

`nbac` turns Apple's `container` runtime into an on-demand, idle-shutdown
`aarch64-linux` remote builder for Nix on macOS. It is a ground-up replacement
for [nix-hex-box](https://github.com/robertderose/nix-hex-box), keeping its
best ideas and discarding its architecture.

## Scope

- A single Rust CLI (`nbac`) that owns **all** imperative behavior: machine
  lifecycle, image builds, key management, SSH transport, diagnostics.
- A thin nix-darwin module that is **purely declarative**: it renders a TOML
  config file, installs packages, and declares `nix.buildMachines` plus SSH
  client config. It contains no shell scripts and no activation logic.
- A default Containerfile (Alpine + upstream Nix) built locally; no published
  image artifact anywhere.

Non-goals: Docker API compatibility (no Socktainer), Linux/NixOS hosts,
x86_64 guests, multi-machine fleets.

## Lessons from nix-hex-box

Kept:

- On-demand machine start via SSH `ProxyCommand` — no resident daemon.
- Guest-side idle watchdog that powers the machine off after a period with no
  SSH connections.
- Persistent `container machine` whose guest `/nix` store survives stop/start;
  recreation only on generation change, with a clear warning that the guest
  store is deleted.
- No published ports, no dependence on a stable machine IP.
- Narrow passwordless sudo for `nix-daemon --stdio` only; pinned host key via
  generated `known_hosts`.

Rejected, with reasons:

- **Nix-interpolated bash generating guest shell** (three languages, base64
  payload smuggling, heredocs inside heredocs): unauditable and fragile.
  Replaced by Rust reading a TOML file.
- **Runtime guest bootstrap** (users, sshd config, sudoers, watchdog injected
  post-create, tracked by two separate hashes, five lifecycle transitions on
  cold create): everything except secrets is baked into the image at build
  time; one generation hash covers it all.
- **Pre-built GHCR image on a mutable `latest` tag**: images are always built
  locally from a Containerfile the user can read.
- **`system.activationScripts` running pkg installers and copying scripts to
  `/usr/local/bin`**: activation mutates nothing; `nbac setup` and the lazy
  cold path do the work; binaries live in the Nix profile.
- **Recovery by grepping error strings, mkdir locks with staleness
  heuristics, sleep-retry loops on every status probe**: typed JSON parsing,
  one `flock`-based lock, structured backoff, concurrent probes.
- **argc-generated 1800-line bash parser driven by 18 environment
  variables**: clap.
- **Lix via network installer + plan patching**: pinned upstream Nix static
  tarball with checksum verification in the Containerfile.

## Architecture

```
┌─ nix-darwin module (declarative only) ─────────────────────┐
│ options → /etc/nbac/config.toml (environment.etc)          │
│ systemPackages: nbac, pkgs.container                       │
│ nix.buildMachines + nix.distributedBuilds                  │
│ /etc/ssh/ssh_config.d/ entry for the builder host alias    │
└────────────────────────────────────────────────────────────┘
                        │ reads config.toml
┌─ nbac (Rust, one binary) ──────────────────────────────────┐
│ setup · status · start · stop · reset · ssh · proxy        │
│ doctor · test · gc · logs                                  │
└────────────────────────────────────────────────────────────┘
                        │ drives (typed wrapper, JSON via serde)
┌─ Apple `container` CLI (from nixpkgs) ─────────────────────┐
│ container machine: built from local Containerfile          │
│ image fully baked: user, sshd, sudoers, init, watchdog     │
│ runtime-injected (one exec): SSH keys + idle settings      │
└────────────────────────────────────────────────────────────┘
```

The tool runs standalone from a hand-written `config.toml`; the nix-darwin
module is optional sugar. This keeps `nbac` testable without `darwin-rebuild`.

## Configuration

Single TOML file, default `/etc/nbac/config.toml`, overridable with
`--config`. Everything the tool needs lives here; nothing is interpolated at
module eval time beyond rendering these values.

```toml
[machine]
name = "nbac"
cpus = 4
memory = "6G"

[image]
containerfile = "/nix/store/…-Containerfile"  # or a user path
build_context = "/nix/store/…-context"        # optional
tag_prefix = "nbac-builder"

[ssh]
user = "builder"
port = 22
host_alias = "nbac"

[state]
dir = "/Users/adam/.local/state/nbac"          # keys, known_hosts, generation marker, lock

[idle]
enable = true
timeout_seconds = 300

[runtime]
container_binary = "container"                 # resolved from PATH (Nix profile)
```

The image tag is `{tag_prefix}:{generation}` where `generation` is a hash of
the Containerfile contents, build context hash, and the config values that
are baked into the image. Tag mismatch on the running machine's image →
rebuild image → recreate machine (with an explicit destructive-action notice).
This single value replaces hex-box's `bootstrapVersion` + `machineGeneration`
pair.

## CLI surface

| Command | Behavior |
| --- | --- |
| `nbac setup` | Idempotent preflight: verify `container` availability, generate builder + host keypairs, write `known_hosts`, build image, create machine, boot, readiness check. Everything it does also happens lazily on the proxy cold path. |
| `nbac status` | Concurrent probes (runtime, machine, SSH, remote store), no retry loops, sub-second when healthy. |
| `nbac start` / `stop` | Explicit lifecycle control. |
| `nbac reset` | Destroy and recreate the machine (confirms; deletes guest `/nix`). |
| `nbac ssh [args…]` | Interactive SSH into the builder. |
| `nbac proxy` | Hidden; used as SSH `ProxyCommand`. See hot path below. |
| `nbac doctor [--fix]` | Runtime health, DNS/external reachability, image contract checks; `--fix` applies recovery (restart runtime, re-inject keys). |
| `nbac test` | Trivial `aarch64-linux` derivation built remotely with a deadline. |
| `nbac gc` | `nix-collect-garbage --delete-old` inside the guest. |
| `nbac logs <boot\|idle> [--follow] [--lines N]` | Guest log access. |

Shell completions come from `clap_complete`, packaged by the flake.

### Proxy hot path

The number-one performance complaint about hex-box's `hb`/proxy chain.

- **Fast path**: one `container machine inspect` (serde-parsed). If status is
  `running` and the stored generation marker matches, immediately exec the
  stdio transport. No lock, no start logic, no polling.
- **Cold path**: take the `flock`, then build image if tag missing → create
  machine if missing or generation mismatch → boot → inject keys/settings
  (single guest exec) → wait for sshd with a loop *inside* the guest (one VM
  exec total, not thirty host-side probes) → exec transport.
- Transport: plain TCP (`nc`) to the machine's host-only vmnet address and
  the guest sshd port, with the IP resolved fresh on every connection from
  `machine inspect` — machine IPs change across boots, but nothing persists
  them, so the no-stable-IP property holds. An stdio transport through
  `container machine run … socat` was tried first and rejected: the CLI
  writes progress chatter to stdout under parallel load, corrupting the
  stream (observed as nix "protocol mismatch, got 'started…'"). TCP is also
  ~75 ms faster per connection (~110 ms vs ~185 ms).
- The SSH client config must **not** set `ControlMaster`/`ControlPersist`:
  Nix verifies every connection by reading a `LocalCommand echo started`
  sentinel as the first line of ssh stdout, and config-level multiplexing
  makes that sentinel appear zero times (mux clients) or twice (the
  persisted master) — observed as "failed to start SSH connection" and
  "protocol mismatch, got 'started'". Nix's `SSHMaster` already shares
  connections over its own private control sockets.

## Default image

`images/Containerfile`, built locally, never published:

- `FROM alpine:<pinned>`.
- Install a **pinned upstream Nix release from the official static binary
  tarball**, verified against a checksum recorded in the Containerfile. No
  curl-pipe installers, no plan patching. Nix, not Lix.
- openssh-server, `s6`, and the minimum runtime packages (`iproute2` for the
  watchdog's `ss`, `sudo`, `socat` if the stdio transport needs it).
- Baked at build time: `builder` user, `sshd_config`, sudoers entry
  (`nix-daemon --stdio` only), `nix.conf`, idle watchdog script, `/sbin/init`,
  and the s6 service directories.
- Init is s6: `/sbin/init` is a few lines of sh that populate the scan
  directory and exec `s6-svscan` (correct PID 1: reaping, signals, crash
  restarts). Services are plain-sh run scripts: `nix-daemon`, `sshd`
  (foreground, gated by a `down` file until keys are injected), and the idle
  watchdog.
- Runtime-injected only: `authorized_keys`, pinned SSH host key, and
  `/etc/nbac/runtime.toml` (idle enable/timeout) — so idle-timeout changes do
  not force an image rebuild. The injection exec brings sshd up (or restarts
  it) with `s6-svc -ru`.

Users may point `image.containerfile` at their own file. The documented
contract for custom images: s6 supervision with scan directory
`/run/service` and an `sshd` service the injection exec can `s6-svc -ru`, a
supervised `nix-daemon`, the configured SSH user with the sudo rule, and the
watchdog if idle shutdown is enabled. `nbac doctor` verifies the contract.

## Idle shutdown

Guest-side watchdog baked into the image: poll `ss` for established
connections on the SSH port; after `timeout_seconds` of none, request
shutdown from PID 1 (`s6-svscanctl -t`), whose finish hook powers the VM
off. Reads settings from the runtime-injected file with safe defaults.

## nix-darwin module

Options (roughly mirroring the TOML): `enable`, `machine.{name,cpus,memory}`,
`image.{containerfile,buildContext}`, `ssh.{user,port,hostAlias}`,
`idle.{enable,timeoutSeconds}`, `stateDir`, plus builder-scheduling options
passed straight to `nix.buildMachines` (`systems`, `maxJobs`, `speedFactor`,
`supportedFeatures`, `mandatoryFeatures`, `protocol`).

The module:

- renders `environment.etc."nbac/config.toml"`,
- adds `nbac` (and `containerPackage` when set) to
  `environment.systemPackages`,
- sets `nix.distributedBuilds`, `nix.settings.builders-use-substitutes`, and
  `nix.buildMachines` (SSH key path under `stateDir`),
- writes the SSH client config for the host alias via
  `environment.etc."ssh/ssh_config.d/…"` (declarative data, not a script),
- asserts `aarch64-darwin`.

It does **not** use `system.activationScripts`. After enabling the module the
user runs `nbac setup` once (or just triggers a build and lets the cold path
do it).

Apple `container` is expected to be installed system-wide (Apple's signed
pkg or the Homebrew cask; 1.1.0 at time of writing) and resolved from PATH.
The module's `containerPackage` option (default `null`) can instead install
`pkgs.container` once it is validated end to end.

## Security model

- Builder key and host key generated locally under `stateDir` (0700), never
  in the Nix store.
- Host key pinned via generated `known_hosts`; `StrictHostKeyChecking yes`.
- Guest user has passwordless sudo for exactly one command:
  `nix-daemon --stdio`.
- No published ports; the guest sshd is reachable only on the host-only
  vmnet interface, and connections are initiated from the host. sshd raises
  `MaxStartups` and disables `PerSourcePenalties`: Nix opens bursts of
  parallel connections from one source address, which the OpenSSH defaults
  drop and then lock out.
- Image built locally from a readable Containerfile with checksum-verified
  inputs.

## Dependencies

Approved crates (latest versions checked at add time): `clap` (derive +
`clap_complete`), `serde`, `toml`, `serde_json`, `anyhow`, `thiserror`,
`rustix` (flock), `sha2` (generation hashing), `console` (styled terminal
output). No async runtime — process orchestration is sequential;
`status` concurrency uses `std::thread`.

Nix side: `nixpkgs` (`rustPlatform.buildRustPackage`), `nix-darwin` for
module testing. Target platform `aarch64-darwin` only. Apple `container`
comes from the system install by default (see module section).

## Development phases

1. **Scaffold** — flake (Rust package, dev shell, checks, formatter), Cargo
   skeleton with stubbed subcommands and config types, empty module, flake
   checks (build + clippy + fmt + module eval); a CI workflow is deferred
   until a forge is chosen.
2. **Core** — config loading; typed `container` wrapper (JSON parsing, error
   taxonomy, one shared recovery routine); image build with generation
   tagging; machine lifecycle; key management.
3. **Builder integration** — `proxy` fast/cold paths, key injection,
   `known_hosts`, module rendering TOML + `buildMachines` + ssh config;
   default Containerfile; end-to-end `nix build` of an `aarch64-linux`
   derivation from a clean machine. Benchmark stdio vs machine-IP transport.
4. **Operations** — idle watchdog, `status`/`doctor`/`test`/`reset`/`gc`/
   `logs`, concurrent probes.
5. **Polish** — docs (custom-image contract, migration from nix-hex-box),
   completions packaging, release workflow.

## Validation items

- Confirm `pkgs.container` works end to end without Apple's pkg installer
  (entitlements/codesigning, `container system start`, kernel install on
  first use) before recommending `containerPackage = pkgs.container`.
- ~~Confirm what `container machine` exposes for file injection~~ Resolved
  in phase 3: a single guest exec with the script piped through stdin
  (`machine run --interactive -- sh`). Note: `machine run` re-joins its argv
  and shell-evaluates it in the guest, so quoting does not survive; nbac only
  passes single safe tokens and ships payloads via stdin. Machine IPs work
  but change on every boot (see transport benchmark).
- ~~Confirm `machine set` semantics~~ Resolved in phase 3: cpus/memory are
  adjustable on an existing machine, taking effect at the next restart; no
  recreation needed.
- `machine create` returns before the guest is bootable; a `machine run` in
  that window fails transiently ("Inappropriate ioctl for device"), handled
  by bounded backoff in the container wrapper.
- The `container` runtime is a per-user launchd agent (Mach/XPC service in
  the user's bootstrap namespace), but nix-daemon runs the ProxyCommand as
  root. When nbac runs with euid 0 it wraps every `container` invocation in
  `launchctl asuser <uid> sudo --user #<uid> --set-home`, deriving the uid
  from the state directory's owner.

## Decision log

| Decision | Choice |
| --- | --- |
| Name | `nbac` (nix builder, apple container) |
| Tooling language | Rust |
| Config | TOML rendered by the module, read by the tool |
| Socktainer | Dropped |
| Image | No published artifact; local build from provided or user Containerfile |
| Activation scripts | None; `nbac setup` + lazy cold path |
| Nix implementation in guest | Upstream Nix (static tarball), not Lix |
| Guest init | s6 (`s6-svscan` as PID 1, plain-sh run scripts); considered a bespoke script (bad PID 1) and OpenRC (boot manager we don't need) |
| Transport | TCP via `nc` to the machine IP, resolved per connection from `machine inspect`; stdio via `machine run … socat` rejected (CLI progress chatter corrupts the stream under parallel load) |
| Apple `container` | System-installed binary from PATH; opt-in `containerPackage` module option for `pkgs.container` |
| Module target | nix-darwin only |
| Async runtime | None |
| CI | `nix flake check` only until a forge is chosen |
