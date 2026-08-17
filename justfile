default:
    just --list

format:
    cargo fmt
    nixfmt **/*.nix

lint:
    cargo clippy

check:
    nix flake check

run *args:
    cargo run -- {{ args }}

test *args:
    cargo test {{ args }}

bump-nix:
    #!/usr/bin/env bash
    set -euo pipefail
    version="$(curl --silent --show-error --fail \
        'https://nix-releases.s3.amazonaws.com/?prefix=nix/nix-&delimiter=/' \
        | grep --only-matching 'nix/nix-[0-9][0-9.]*/' \
        | sed -e 's|nix/nix-||' -e 's|/||' \
        | sort --version-sort \
        | tail -1)"
    sha="$(curl --silent --show-error --fail --location \
        "https://releases.nixos.org/nix/nix-${version}/nix-${version}-aarch64-linux.tar.xz" \
        | shasum --algorithm 256 \
        | cut -d' ' -f1)"
    tmp="$(mktemp)"
    sed -e "s|^ARG NIX_VERSION=.*|ARG NIX_VERSION=${version}|" \
        -e "s|^ARG NIX_SHA256=.*|ARG NIX_SHA256=${sha}|" \
        images/Containerfile > "$tmp"
    mv "$tmp" images/Containerfile
    echo "nix ${version} ${sha}"
