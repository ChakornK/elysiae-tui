use std::sync::Arc;

use irmin::game_installer::SophonError;
use irmin::game_installer::UpdateInfo;
use irmin::{DownloadHandle, SophonProgress};
use tokio::sync::mpsc::Sender;

use crate::game::GameId;

/// Wraps irmin's async functions, routing progress through a channel.
pub struct Operations {
    client: reqwest::Client,
}

impl Operations {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
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

    pub async fn download(
        &self,
        game: GameId,
        vo_lang: &str,
        output_path: &str,
        handle: &DownloadHandle,
        tx: Sender<SophonProgress>,
    ) -> Result<(), SophonError> {
        let on_progress: Arc<dyn Fn(SophonProgress) + Send + Sync> =
            Arc::new(move |p| { let _ = tx.try_send(p); });
        irmin::sophon_download(&self.client, game.as_str(), vo_lang, output_path, handle, on_progress).await
    }

    pub async fn update(
        &self,
        game: GameId,
        vo_lang: &str,
        output_path: &str,
        handle: &DownloadHandle,
        tx: Sender<SophonProgress>,
    ) -> Result<(), SophonError> {
        let on_progress: Arc<dyn Fn(SophonProgress) + Send + Sync> =
            Arc::new(move |p| { let _ = tx.try_send(p); });
        irmin::sophon_update(&self.client, game.as_str(), vo_lang, output_path, handle, on_progress).await
    }

    pub async fn preinstall(
        &self,
        game: GameId,
        vo_lang: &str,
        output_path: &str,
        handle: &DownloadHandle,
        tx: Sender<SophonProgress>,
    ) -> Result<(), SophonError> {
        let on_progress: Arc<dyn Fn(SophonProgress) + Send + Sync> =
            Arc::new(move |p| { let _ = tx.try_send(p); });
        irmin::sophon_preinstall(&self.client, game.as_str(), vo_lang, output_path, handle, on_progress).await
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
}
