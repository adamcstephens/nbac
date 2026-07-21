//! Builder and host keypairs under the state directory, never in the Nix
//! store, plus the pinned `known_hosts` derived from the host key.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::config::Config;

pub fn ensure(config: &Config) -> Result<()> {
    let dir = &config.state.dir;
    std::fs::create_dir_all(dir)
        .with_context(|| format!("cannot create state directory {}", dir.display()))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;

    generate_keypair(&config.state.builder_key(), "nbac builder key")?;
    generate_keypair(&config.state.host_key(), "nbac host key")?;

    let host_pub = std::fs::read_to_string(config.state.host_key_pub())?;
    crate::fsutil::replace(
        &config.state.known_hosts(),
        &format!("{} {}", config.ssh.host_alias, host_pub),
    )
}

fn generate_keypair(path: &Path, comment: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    let output = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", comment, "-f"])
        .arg(path)
        .output()
        .context("cannot run ssh-keygen")?;
    if !output.status.success() {
        bail!(
            "ssh-keygen failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}
