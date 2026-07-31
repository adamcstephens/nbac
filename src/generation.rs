//! Image generation hashing: one value covering the Containerfile, the build
//! context, and the config values baked into the image at build time.

use std::path::Path;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::config::Config;

pub fn compute(config: &Config) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(b"nbac-generation-v1\0");

    let containerfile = std::fs::read(&config.image.containerfile).with_context(|| {
        format!(
            "cannot read Containerfile {}",
            config.image.containerfile.display()
        )
    })?;
    hash_field(&mut hasher, "containerfile", &containerfile);

    if let Some(context) = &config.image.build_context {
        hash_tree(&mut hasher, context, Path::new(""))
            .with_context(|| format!("cannot hash build context {}", context.display()))?;
    }

    hash_field(&mut hasher, "ssh.user", config.ssh.user.as_bytes());
    hash_field(&mut hasher, "ssh.port", &config.ssh.port.to_be_bytes());
    hash_field(
        &mut hasher,
        "machine.arch",
        config.machine.arch().as_bytes(),
    );

    let digest: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(digest[..12].to_string())
}

pub fn image_tag(config: &Config) -> Result<String> {
    Ok(format!("{}:{}", config.image.tag_prefix, compute(config)?))
}

fn hash_field(hasher: &mut Sha256, name: &str, value: &[u8]) {
    hasher.update(name.as_bytes());
    hasher.update(b"\0");
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn hash_tree(hasher: &mut Sha256, root: &Path, relative: &Path) -> Result<()> {
    let dir = root.join(relative);
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .with_context(|| format!("cannot read directory {}", dir.display()))?
        .collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let rel = relative.join(entry.file_name());
        let path = entry.path();
        let kind = entry.file_type()?;
        let name = rel.to_string_lossy().into_owned();
        if kind.is_dir() {
            hash_field(hasher, "dir", name.as_bytes());
            hash_tree(hasher, root, &rel)?;
        } else if kind.is_symlink() {
            let target = std::fs::read_link(&path)?;
            hash_field(hasher, "symlink", name.as_bytes());
            hash_field(hasher, "target", target.to_string_lossy().as_bytes());
        } else if kind.is_file() {
            let contents =
                std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?;
            hash_field(hasher, "file", name.as_bytes());
            hash_field(hasher, "contents", &contents);
        } else {
            bail!("unsupported file type in build context: {}", path.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(dir: &Path) -> Config {
        let containerfile = dir.join("Containerfile");
        std::fs::write(&containerfile, "FROM alpine:3.22\n").unwrap();
        toml::from_str(&format!(
            r#"
            [image]
            containerfile = "{}"

            [state]
            dir = "{}"
            "#,
            containerfile.display(),
            dir.display()
        ))
        .unwrap()
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("nbac-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn deterministic() {
        let dir = temp_dir("deterministic");
        let config = test_config(&dir);
        assert_eq!(compute(&config).unwrap(), compute(&config).unwrap());
        assert_eq!(compute(&config).unwrap().len(), 12);
    }

    #[test]
    fn containerfile_change_changes_generation() {
        let dir = temp_dir("containerfile");
        let config = test_config(&dir);
        let before = compute(&config).unwrap();
        std::fs::write(dir.join("Containerfile"), "FROM alpine:3.23\n").unwrap();
        assert_ne!(before, compute(&config).unwrap());
    }

    #[test]
    fn baked_config_change_changes_generation() {
        let dir = temp_dir("baked");
        let mut config = test_config(&dir);
        let before = compute(&config).unwrap();
        config.ssh.user = "other".into();
        assert_ne!(before, compute(&config).unwrap());
    }

    #[test]
    fn rosetta_change_changes_generation() {
        let dir = temp_dir("rosetta");
        let mut config = test_config(&dir);
        let before = compute(&config).unwrap();
        config.machine.rosetta = true;
        assert_ne!(before, compute(&config).unwrap());
    }

    #[test]
    fn build_context_contents_change_generation() {
        let dir = temp_dir("context");
        let mut config = test_config(&dir);
        let context = dir.join("context");
        std::fs::create_dir_all(context.join("sub")).unwrap();
        std::fs::write(context.join("sub/file"), "one").unwrap();
        config.image.build_context = Some(context.clone());

        let before = compute(&config).unwrap();
        std::fs::write(context.join("sub/file"), "two").unwrap();
        assert_ne!(before, compute(&config).unwrap());
    }

    #[test]
    fn tag_uses_prefix() {
        let dir = temp_dir("tag");
        let config = test_config(&dir);
        let tag = image_tag(&config).unwrap();
        assert!(tag.starts_with("nbac-builder:"));
    }
}
