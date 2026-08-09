use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Writes `data` to `target` atomically via a unique temporary file and rename.
pub fn atomic_write(target: &Path, data: &[u8]) -> io::Result<()> {
    let pid = std::process::id();
    let tmp_name = format!(
        ".{}.{pid}.tmp",
        target.file_name().unwrap_or_default().to_string_lossy()
    );
    let tmp = target.with_file_name(tmp_name);
    fs::write(&tmp, data)?;
    if let Err(e) = fs::rename(&tmp, target) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Removes a directory, but if the path is a symlink, only removes the symlink itself.
pub fn safe_remove_dir_all(path: &Path) -> io::Result<()> {
    if path.is_symlink() {
        fs::remove_file(path)
    } else if path.exists() {
        fs::remove_dir_all(path)
    } else {
        Ok(())
    }
}

/// Renames a corrupt file to `{name}.corrupted-{unix_ts}` for later inspection.
/// Returns the path of the preserved file.
pub fn preserve_corrupt(path: &Path) -> io::Result<PathBuf> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut dest_name = path.file_name().unwrap_or_default().to_os_string();
    dest_name.push(format!(".corrupted-{ts}"));
    let dest = path.with_file_name(dest_name);
    fs::rename(path, &dest)?;
    Ok(dest)
}
