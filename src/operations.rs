use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use irmin::game_installer::installer::{InstallCallbacks, InstallOptions, ResumeContext};
use irmin::game_installer::{self, SophonError, UpdateInfo};
use irmin::{DownloadHandle, SophonProgress};
use rustc_hash::FxHashMap;
use tokio::sync::mpsc::Sender;

use crate::game::GameId;
use crate::state::{DownloadState, DownloadType, StateSaverFn, make_state_saver};

/// Wraps irmin's game installer, routing progress through a channel and
/// persisting download state for resume.
pub struct Operations {
    client: reqwest::Client,
    data_dir: PathBuf,
}

impl Operations {
    pub fn new(client: reqwest::Client, data_dir: PathBuf) -> Self {
        Self { client, data_dir }
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub async fn check_update(
        &self,
        game: GameId,
        vo_lang: &str,
        output_path: &str,
    ) -> Result<UpdateInfo, SophonError> {
        irmin::sophon_check_update(&self.client, game.as_str(), vo_lang, output_path).await
    }

    /// Fresh game download with state persistence for resume.
    pub async fn download(
        &self,
        game: GameId,
        vo_lang: &str,
        output_path: &str,
        handle: &DownloadHandle,
        tx: Sender<SophonProgress>,
    ) -> Result<(), SophonError> {
        let game_dir = PathBuf::from(output_path);
        let updater: Arc<dyn Fn(SophonProgress) + Send + Sync> =
            Arc::new(move |p| { let _ = tx.try_send(p); });

        updater(SophonProgress::FetchingManifest);
        let (installers, tag, manifest_hash) =
            game_installer::build_installers(&self.client, game.as_str(), vo_lang).await?;

        let state_path = DownloadState::state_path(&self.data_dir, game.as_str());
        let (resume, state_saver) = self.build_resume_context(
            game, DownloadType::Fresh, vo_lang, output_path, &manifest_hash, &state_path,
        );
        let options = InstallOptions {
            is_preinstall: false,
            is_resume: resume.prev_manifest_hash == manifest_hash && !resume.prev_downloaded_chunks.is_empty(),
            handle: handle.clone(),
        };
        let callbacks = InstallCallbacks {
            updater: updater.clone(),
            state_saver,
            completion_state: Arc::new(OnceLock::new()),
        };
        let vo_langs = vec![vo_lang.to_string()];

        game_installer::install(
            installers, &game_dir, vec![], &tag, resume, options, callbacks, game.as_str(), &vo_langs,
        ).await?;

        DownloadState::remove(&state_path);
        updater(SophonProgress::Finished);
        Ok(())
    }

    /// Update an existing installation with state persistence for resume.
    pub async fn update(
        &self,
        game: GameId,
        vo_lang: &str,
        output_path: &str,
        handle: &DownloadHandle,
        tx: Sender<SophonProgress>,
    ) -> Result<(), SophonError> {
        let game_dir = PathBuf::from(output_path);
        let current_tag = game_installer::read_installed_tag(&game_dir)
            .ok_or(SophonError::NoInstalledVersion)?;

        let updater: Arc<dyn Fn(SophonProgress) + Send + Sync> =
            Arc::new(move |p| { let _ = tx.try_send(p); });

        updater(SophonProgress::FetchingManifest);
        let (installers, deleted_files, new_tag, manifest_hash) =
            game_installer::build_update_installers(
                &self.client, game.as_str(), vo_lang, &current_tag, &game_dir,
            ).await?;

        let state_path = DownloadState::state_path(&self.data_dir, game.as_str());
        let (resume, state_saver) = self.build_resume_context(
            game, DownloadType::Update, vo_lang, output_path, &manifest_hash, &state_path,
        );
        let options = InstallOptions {
            is_preinstall: false,
            is_resume: resume.prev_manifest_hash == manifest_hash && !resume.prev_downloaded_chunks.is_empty(),
            handle: handle.clone(),
        };
        let callbacks = InstallCallbacks {
            updater: updater.clone(),
            state_saver,
            completion_state: Arc::new(OnceLock::new()),
        };
        let vo_langs = vec![vo_lang.to_string()];

        game_installer::install(
            installers, &game_dir, deleted_files, &new_tag, resume, options, callbacks, game.as_str(), &vo_langs,
        ).await?;

        DownloadState::remove(&state_path);
        updater(SophonProgress::Finished);
        Ok(())
    }

    /// Preinstall an upcoming version.
    pub async fn preinstall(
        &self,
        game: GameId,
        vo_lang: &str,
        output_path: &str,
        handle: &DownloadHandle,
        tx: Sender<SophonProgress>,
    ) -> Result<(), SophonError> {
        let game_dir = PathBuf::from(output_path);
        let updater: Arc<dyn Fn(SophonProgress) + Send + Sync> =
            Arc::new(move |p| { let _ = tx.try_send(p); });

        updater(SophonProgress::FetchingManifest);
        let plan = game_installer::build_preinstall_plan(
            &self.client, game.as_str(), vo_lang, &game_dir,
        ).await?;

        let state_path = DownloadState::state_path(&self.data_dir, game.as_str());
        let prev_state = DownloadState::load(&state_path);
        let prev_chunks: HashMap<String, u64> = prev_state
            .filter(|s| s.download_type == DownloadType::Preinstall && s.game_id == game.as_str())
            .map(|s| s.downloaded_chunks)
            .unwrap_or_default();

        let new_state = DownloadState {
            game_id: game.as_str().to_string(),
            download_type: DownloadType::Preinstall,
            manifest_hash: String::new(),
            downloaded_chunks: prev_chunks.clone(),
            output_path: PathBuf::from(output_path),
            vo_lang: vo_lang.to_string(),
        };
        let saver = make_state_saver(new_state, state_path.clone());

        game_installer::preinstall_download(
            &self.client,
            plan,
            &game_dir,
            game.as_str(),
            vo_lang,
            handle.clone(),
            updater.clone(),
            saver,
            prev_chunks.into_iter().collect(),
        ).await?;

        DownloadState::remove(&state_path);
        updater(SophonProgress::Finished);
        Ok(())
    }

    pub async fn apply_preinstall(
        &self,
        preinstall_tag: &str,
        output_path: &str,
        handle: &DownloadHandle,
        tx: Sender<SophonProgress>,
    ) -> Result<(), SophonError> {
        let on_progress: Arc<dyn Fn(SophonProgress) + Send + Sync> =
            Arc::new(move |p| { let _ = tx.try_send(p); });
        irmin::sophon_apply_preinstall(&self.client, preinstall_tag, output_path, handle, on_progress).await
    }

    pub async fn verify(
        &self,
        game: GameId,
        vo_lang: &str,
        output_path: &str,
        tx: Sender<SophonProgress>,
    ) -> Result<(), SophonError> {
        let on_progress: Arc<dyn Fn(SophonProgress) + Send + Sync> =
            Arc::new(move |p| { let _ = tx.try_send(p); });
        irmin::sophon_verify_integrity(&self.client, game.as_str(), vo_lang, output_path, on_progress).await
    }

    /// Loads any persisted resume state and builds the ResumeContext for irmin.
    /// If a prior state exists with matching manifest hash, its chunks are passed
    /// for resume. Otherwise starts fresh.
    fn build_resume_context(
        &self,
        game: GameId,
        dl_type: DownloadType,
        vo_lang: &str,
        output_path: &str,
        current_manifest_hash: &str,
        state_path: &Path,
    ) -> (ResumeContext, StateSaverFn) {
        let prev_state = DownloadState::load(state_path);

        let (prev_chunks, prev_hash) = match prev_state {
            Some(ref s) if s.game_id == game.as_str() && s.download_type == dl_type => {
                if s.manifest_hash == current_manifest_hash {
                    (s.downloaded_chunks.clone(), s.manifest_hash.clone())
                } else {
                    // Manifest changed upstream; stale chunks are useless
                    tracing::warn!("manifest changed since last download, discarding stale state");
                    let chunks_dir = PathBuf::from(output_path).join("chunks");
                    let _ = std::fs::remove_dir_all(&chunks_dir);
                    (HashMap::new(), String::new())
                }
            }
            _ => (HashMap::new(), String::new()),
        };

        let new_state = DownloadState {
            game_id: game.as_str().to_string(),
            download_type: dl_type,
            manifest_hash: current_manifest_hash.to_string(),
            downloaded_chunks: prev_chunks.clone(),
            output_path: PathBuf::from(output_path),
            vo_lang: vo_lang.to_string(),
        };
        let saver = make_state_saver(new_state, state_path.to_path_buf());

        let resume = ResumeContext {
            prev_manifest_hash: prev_hash,
            prev_downloaded_chunks: prev_chunks.into_iter().collect::<FxHashMap<_, _>>(),
            resume_seed: Default::default(),
        };

        (resume, saver)
    }
}
