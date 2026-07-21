//! Machine lifecycle: image ensure/build, machine ensure/recreate, and the
//! generation marker the proxy fast path checks.

use anyhow::{Context, Result};

use crate::config::Config;
use crate::container::{MachineInfo, MachineStatus, Runtime};
use crate::{generation, ui};

/// Build the image for the current generation unless it already exists.
pub fn ensure_image(runtime: &Runtime, config: &Config) -> Result<String> {
    let tag = generation::image_tag(config)?;
    if runtime.image_exists(&tag)? {
        return Ok(tag);
    }
    ui::step(&format!("building image {tag}"));
    let context = match &config.image.build_context {
        Some(dir) => dir.clone(),
        None => config
            .image
            .containerfile
            .parent()
            .context("Containerfile has no parent directory")?
            .to_path_buf(),
    };
    let build_args = [
        ("SSH_USER", config.ssh.user.clone()),
        ("SSH_PORT", config.ssh.port.to_string()),
    ];
    runtime.build_image(&tag, &config.image.containerfile, &context, &build_args)?;
    Ok(tag)
}

/// Ensure a machine exists for `tag`, is configured per `config`, and is
/// running. Recreates the machine when its image generation differs, which
/// deletes the guest /nix store.
pub fn ensure_machine(runtime: &Runtime, config: &Config, tag: &str) -> Result<()> {
    let name = &config.machine.name;
    match runtime.machine_inspect(name)? {
        None => {
            ui::step(&format!("creating machine {name}"));
            runtime.machine_create(&config.machine, tag)?;
        }
        Some(info) if !reference_matches(&info.image.reference, tag) => {
            ui::warn(&format!(
                "machine {name} was built from {}; recreating it for {tag} deletes its /nix store",
                info.image.reference
            ));
            if info.status == MachineStatus::Running {
                runtime.machine_stop(name)?;
            }
            runtime.machine_delete(name)?;
            ui::step(&format!("creating machine {name}"));
            runtime.machine_create(&config.machine, tag)?;
        }
        Some(info) => {
            reconcile_resources(runtime, config, &info)?;
            if info.status != MachineStatus::Running {
                ui::step(&format!("booting machine {name}"));
                runtime.machine_boot(name)?;
            }
        }
    }
    Ok(())
}

/// Written only after key injection succeeds, so the proxy fast path never
/// short-circuits into a machine without keys.
pub fn write_generation_marker(config: &Config, tag: &str) -> Result<()> {
    std::fs::write(
        config.state.generation_marker(),
        tag.rsplit(':').next().unwrap(),
    )
    .context("cannot write generation marker")
}

/// Apply cpus/memory changes with `machine set` instead of recreating.
fn reconcile_resources(runtime: &Runtime, config: &Config, info: &MachineInfo) -> Result<()> {
    let wanted_memory = parse_memory(&config.machine.memory)?;
    if info.cpus == config.machine.cpus && info.memory == wanted_memory {
        return Ok(());
    }
    ui::step(&format!(
        "updating machine {} to cpus={} memory={}",
        info.id, config.machine.cpus, config.machine.memory
    ));
    runtime.machine_set(&config.machine)?;
    if info.status == MachineStatus::Running {
        ui::warn("cpu/memory changes take effect after the machine restarts");
    }
    Ok(())
}

/// `machine inspect` may report the reference as given at create time or
/// normalized with a registry prefix.
pub fn reference_matches(reference: &str, tag: &str) -> bool {
    reference == tag || reference.ends_with(&format!("/{tag}"))
}

/// Sizes as `container` accepts them: bytes with an optional binary
/// K/M/G/T/P suffix.
fn parse_memory(value: &str) -> Result<u64> {
    let value = value.trim();
    let (number, shift) = match value.chars().last() {
        Some('K' | 'k') => (&value[..value.len() - 1], 10),
        Some('M' | 'm') => (&value[..value.len() - 1], 20),
        Some('G' | 'g') => (&value[..value.len() - 1], 30),
        Some('T' | 't') => (&value[..value.len() - 1], 40),
        Some('P' | 'p') => (&value[..value.len() - 1], 50),
        _ => (value, 0),
    };
    let number: u64 = number
        .parse()
        .with_context(|| format!("invalid memory size {value:?}"))?;
    number
        .checked_mul(1 << shift)
        .with_context(|| format!("memory size {value:?} overflows"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_memory_suffixes() {
        assert_eq!(parse_memory("6G").unwrap(), 6 * 1024 * 1024 * 1024);
        assert_eq!(parse_memory("512M").unwrap(), 512 * 1024 * 1024);
        assert_eq!(parse_memory("1024").unwrap(), 1024);
        assert_eq!(parse_memory("24g").unwrap(), 25769803776);
        assert!(parse_memory("lots").is_err());
    }

    #[test]
    fn matches_plain_and_normalized_references() {
        assert!(reference_matches(
            "nbac-builder:abc123",
            "nbac-builder:abc123"
        ));
        assert!(reference_matches(
            "docker.io/library/nbac-builder:abc123",
            "nbac-builder:abc123"
        ));
        assert!(!reference_matches(
            "ghcr.io/robertderose/nix-hex-box/hexbox-builder:latest",
            "nbac-builder:abc123"
        ));
    }
}
