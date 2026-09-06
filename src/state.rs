use std::path::Path;

/// Loads the per-game resume state and decides whether it can resume into
/// `install_path` with `vo_lang`.
///
/// irmin persists one state file per game in `data_dir`, but each state
/// records the exact `output_path` and `vo_lang` it belongs to. The state
/// file is therefore "global" to a game across install paths; resume is only
/// real when its `output_path`/`vo_lang` match what the caller will dispatch
/// against — otherwise irmin discards the chunks and re-downloads, so
/// advertising "resume" would be a false positive.
///
/// Returns the loaded state when resumable, so callers can reuse
/// `output_path` (e.g. adopt it as the install path when none is set).
///
/// `install_path` is `None` when the game has no configured install path; the
/// state is still returned then provided its `output_path` exists on disk
/// (the partial chunks are salvageable there).
pub fn resumable_state(
    data_dir: &Path,
    game_id: &str,
    install_path: Option<&Path>,
    vo_lang: &str,
) -> Option<irmin::DownloadState> {
    let state = irmin::load_download_state(data_dir, game_id)?;
    // No recorded chunks → irmin would fresh-download anyway; not a real resume.
    if state.downloaded_chunks.is_empty() {
        return None;
    }
    let state_path = Path::new(&state.output_path);
    let path_ok = match install_path {
        Some(p) => p == state_path,
        None => state_path.exists(),
    };
    if !path_ok || state.vo_lang != vo_lang {
        return None;
    }
    Some(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use irmin::game_installer::CompletedFiles;
    use std::collections::HashMap;

    fn write_state(data_dir: &Path, game_id: &str, state: &irmin::DownloadState) {
        std::fs::create_dir_all(data_dir).unwrap();
        let json = serde_json::to_vec_pretty(state).unwrap();
        std::fs::write(irmin::state_file_path(data_dir, game_id), &json).unwrap();
    }

    fn make_state(
        game_id: &str,
        vo_lang: &str,
        output_path: &str,
        chunks: HashMap<String, u64>,
    ) -> irmin::DownloadState {
        irmin::DownloadState {
            game_id: game_id.to_string(),
            vo_lang: vo_lang.to_string(),
            output_path: output_path.to_string(),
            download_type: irmin::DownloadType::Fresh,
            current_tag: None,
            manifest_hash: "h".to_string(),
            downloaded_chunks: chunks,
            completed_files: CompletedFiles::default(),
        }
    }

    fn chunks_with(n: &str, sz: u64) -> HashMap<String, u64> {
        let mut m = HashMap::new();
        m.insert(n.to_string(), sz);
        m
    }

    #[test]
    fn resumable_when_path_and_vo_lang_match() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let game = tmp.path().join("game");
        std::fs::create_dir_all(&game).unwrap();
        write_state(
            &data,
            "hkrpg",
            &make_state(
                "hkrpg",
                "en-us",
                game.to_str().unwrap(),
                chunks_with("c0", 10),
            ),
        );
        assert!(resumable_state(&data, "hkrpg", Some(&game), "en-us").is_some());
    }

    #[test]
    fn not_resumable_when_install_path_differs() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let game = tmp.path().join("game");
        let other = tmp.path().join("other");
        std::fs::create_dir_all(&game).unwrap();
        write_state(
            &data,
            "hkrpg",
            &make_state(
                "hkrpg",
                "en-us",
                game.to_str().unwrap(),
                chunks_with("c0", 10),
            ),
        );
        // State belongs to |game|; a query about |other| must not resume.
        assert!(resumable_state(&data, "hkrpg", Some(&other), "en-us").is_none());
    }

    #[test]
    fn not_resumable_when_vo_lang_differs() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let game = tmp.path().join("game");
        std::fs::create_dir_all(&game).unwrap();
        write_state(
            &data,
            "hkrpg",
            &make_state(
                "hkrpg",
                "en-us",
                game.to_str().unwrap(),
                chunks_with("c0", 10),
            ),
        );
        assert!(resumable_state(&data, "hkrpg", Some(&game), "ja-jp").is_none());
    }

    #[test]
    fn not_resumable_when_no_chunks_recorded() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let game = tmp.path().join("game");
        std::fs::create_dir_all(&game).unwrap();
        write_state(
            &data,
            "hkrpg",
            &make_state("hkrpg", "en-us", game.to_str().unwrap(), HashMap::new()),
        );
        assert!(resumable_state(&data, "hkrpg", Some(&game), "en-us").is_none());
    }

    #[test]
    fn resumable_when_install_path_unset_and_output_dir_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let game = tmp.path().join("game");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&game).unwrap();
        write_state(
            &data,
            "hkrpg",
            &make_state(
                "hkrpg",
                "en-us",
                game.to_str().unwrap(),
                chunks_with("c0", 10),
            ),
        );
        let state = resumable_state(&data, "hkrpg", None, "en-us").expect("resumable");
        assert_eq!(state.output_path, game.to_str().unwrap());
    }

    #[test]
    fn not_resumable_when_install_path_unset_and_output_dir_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        write_state(
            &data,
            "hkrpg",
            &make_state(
                "hkrpg",
                "en-us",
                "/nonexistent/hkrpg",
                chunks_with("c0", 10),
            ),
        );
        assert!(resumable_state(&data, "hkrpg", None, "en-us").is_none());
    }

    #[test]
    fn not_resumable_when_no_state_file() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let game = tmp.path().join("game");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&game).unwrap();
        assert!(resumable_state(&data, "hkrpg", Some(&game), "en-us").is_none());
    }
}
