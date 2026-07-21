//! The single flock-based lock serializing machine mutations.

use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use rustix::fs::{FlockOperation, flock};

pub struct Lock {
    _file: File,
}

pub fn acquire(path: &Path) -> Result<Lock> {
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .with_context(|| format!("cannot open lock file {}", path.display()))?;
    flock(&file, FlockOperation::LockExclusive)
        .with_context(|| format!("cannot lock {}", path.display()))?;
    Ok(Lock { _file: file })
}
