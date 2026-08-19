# Appended, never prepended: the profile carries nix-daemon too, and the
# ssh-ng builder protocol needs that name to keep resolving to the sudo
# wrapper in /usr/bin.
export PATH="$PATH:/nix/var/nix/profiles/nbac/bin:/nix/var/nix/profiles/default/bin"
