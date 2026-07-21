//! State files are written by whoever runs the cold path — sometimes root
//! (the daemon's ProxyCommand), sometimes the owning user — so writes go
//! through rename: the directory owner can always replace a root-owned file.

use std::path::Path;

use anyhow::{Context, Result};

pub fn replace(path: &Path, contents: &str) -> Result<()> {
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, contents).with_context(|| format!("cannot write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(())
}
