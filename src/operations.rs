use std::path::PathBuf;

use irmin::game_installer::{SophonError, UpdateInfo};
use irmin::{DownloadHandle, Sophon, SophonProgress};
use tokio::sync::mpsc::Sender;

use crate::game::GameId;

/// Wraps irmin's `Sophon` entry point, routing progress through a channel
/// and persisting resume state in `data_dir`.
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

    fn sophon(&self, game: GameId, vo_lang: &str, output_path: &str) -> Sophon {
        Sophon::builder(game.as_str().to_string(), PathBuf::from(output_path))
            .vo_lang(vo_lang)
            .state_dir(self.data_dir.clone())
            .client(self.client.clone())
            .build()
    }

    pub async fn check_update(
        &self,
        game: GameId,
        vo_lang: &str,
        output_path: &str,
    ) -> Result<UpdateInfo, SophonError> {
        self.sophon(game, vo_lang, output_path).check_update().await
    }

    /// Fresh game download with automatic resume.
    pub async fn download(
        &self,
        game: GameId,
        vo_lang: &str,
        output_path: &str,
        handle: &DownloadHandle,
        tx: Sender<SophonProgress>,
    ) -> Result<(), SophonError> {
        self.sophon(game, vo_lang, output_path)
            .download(handle, move |p| { let _ = tx.try_send(p); })
            .await
    }

    /// Update an existing installation with automatic resume.
    pub async fn update(
        &self,
        game: GameId,
        vo_lang: &str,
        output_path: &str,
        handle: &DownloadHandle,
        tx: Sender<SophonProgress>,
    ) -> Result<(), SophonError> {
        self.sophon(game, vo_lang, output_path)
            .update(handle, move |p| { let _ = tx.try_send(p); })
            .await
    }

    /// Preinstall an upcoming version with automatic resume.
    pub async fn preinstall(
        &self,
        game: GameId,
        vo_lang: &str,
        output_path: &str,
        handle: &DownloadHandle,
        tx: Sender<SophonProgress>,
    ) -> Result<(), SophonError> {
        self.sophon(game, vo_lang, output_path)
            .preinstall(handle, move |p| { let _ = tx.try_send(p); })
            .await
    }

    /// Apply a previously downloaded preinstall patch.
    pub async fn apply_preinstall(
        &self,
        game: GameId,
        vo_lang: &str,
        preinstall_tag: &str,
        output_path: &str,
        handle: &DownloadHandle,
        tx: Sender<SophonProgress>,
    ) -> Result<(), SophonError> {
        self.sophon(game, vo_lang, output_path)
            .apply_preinstall(preinstall_tag, handle, move |p| { let _ = tx.try_send(p); })
            .await
    }

    /// Verify integrity of an installed game.
    pub async fn verify(
        &self,
        game: GameId,
        vo_lang: &str,
        output_path: &str,
        tx: Sender<SophonProgress>,
    ) -> Result<(), SophonError> {
        self.sophon(game, vo_lang, output_path)
            .verify_integrity(move |p| { let _ = tx.try_send(p); })
            .await
    }
}
