use std::path::Path;

/// Whether a game directory holds partial download state a resume can
/// salvage: the persisted resume-state file, leftover chunk files, or
/// irmin's eager-path bitmap. The bitmap marks chunks written straight to
/// output files, so it covers cases where the state file was never created
/// (e.g. interrupted before a periodic save fired).
pub fn has_partial_download(data_dir: &Path, install_path: &Path, game_id: &str) -> bool {
    if irmin::state_file_path(data_dir, game_id).exists() {
        return true;
    }
    if install_path.join("chunks").exists() {
        return true;
    }
    if let Ok(entries) =
        std::fs::read_dir(install_path)
            && entries
                .flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with(".sophon_bitmap"))
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        std::fs::write(path, b"x").unwrap();
    }

    #[test]
    fn partial_detection_finds_state_file() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        touch(&irmin::state_file_path(&data, "hkrpg"));
        assert!(has_partial_download(&data, tmp.path(), "hkrpg"));
    }

    #[test]
    fn partial_detection_finds_bitmap() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("game");
        std::fs::create_dir_all(&game).unwrap();
        touch(&game.join(".sophon_bitmap_abcd1234ef567890"));
        assert!(has_partial_download(tmp.path(), &game, "hkrpg"));
    }

    #[test]
    fn partial_detection_finds_chunks_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("game");
        std::fs::create_dir_all(game.join("chunks")).unwrap();
        assert!(has_partial_download(tmp.path(), &game, "hkrpg"));
    }

    #[test]
    fn partial_detection_empty_install_is_false() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!has_partial_download(tmp.path(), tmp.path(), "hkrpg"));
    }

    #[test]
    fn partial_detection_empty_game_dir_is_false() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("game");
        std::fs::create_dir_all(&game).unwrap();
        assert!(!has_partial_download(tmp.path(), &game, "hkrpg"));
    }
}
