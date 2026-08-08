use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Writes `data` to `target` atomically via a temporary file and rename.
pub fn atomic_write(target: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = target.with_extension("tmp");
    fs::write(&tmp, data)?;
    fs::rename(&tmp, target)
}

/// Renames a corrupt file to `{name}.corrupted-{unix_ts}` for later inspection.
/// Returns the path of the preserved file.
pub fn preserve_corrupt(path: &Path) -> io::Result<PathBuf> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let ext = format!("corrupted-{ts}");
    let dest = path.with_extension(ext);
    fs::rename(path, &dest)?;
    Ok(dest)
}
