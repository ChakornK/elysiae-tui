#![allow(dead_code)]
use std::path::Path;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum DiskError {
    #[error("insufficient disk space at {path}: need {needed} bytes, have {available} bytes")]
    InsufficientSpace {
        path: String,
        needed: u64,
        available: u64,
    },
    #[error("failed to query disk space: {0}")]
    Io(#[from] std::io::Error),
}

/// Checks that `path` has at least `needed_bytes` + 10% margin of free space.
pub fn check_available_space(path: &Path, needed_bytes: u64) -> Result<(), DiskError> {
    use fs2::available_space;
    let available = available_space(path)?;
    let required = needed_bytes.saturating_add(needed_bytes / 10);
    if available < required {
        return Err(DiskError::InsufficientSpace {
            path: path.display().to_string(),
            needed: required,
            available,
        });
    }
    Ok(())
}
