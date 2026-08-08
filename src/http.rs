#![allow(dead_code)]
use std::time::Duration;

use reqwest::Client;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum HttpError {
    #[error("HTTP {code} from {url}")]
    Status { code: u16, url: String },
    #[error("request timeout: {0}")]
    Timeout(String),
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
}

/// Builds a shared HTTP client with proper timeouts, pool tuning, and user-agent.
pub fn build_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(300))
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(30))
        .pool_max_idle_per_host(24)
        .pool_idle_timeout(Duration::from_secs(90))
        .user_agent(concat!("elysiae-tui/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("failed to build HTTP client")
}

/// Max response body size for JSON endpoints (16 MB).
const MAX_JSON_BODY: usize = 16 * 1024 * 1024;

/// Fetches JSON from a URL, returning an error on non-2xx status.
/// Rejects responses larger than 16 MB to prevent OOM.
pub async fn fetch_json<T: serde::de::DeserializeOwned>(
    client: &Client,
    url: &str,
) -> Result<T, HttpError> {
    let resp = client.get(url).send().await?.error_for_status()?;
    if let Some(len) = resp.content_length() {
        if len > MAX_JSON_BODY as u64 {
            return Err(HttpError::Status {
                code: 0,
                url: format!("response too large ({len} bytes): {url}"),
            });
        }
    }
    let bytes = resp.bytes().await?;
    if bytes.len() > MAX_JSON_BODY {
        return Err(HttpError::Status {
            code: 0,
            url: format!("response too large ({} bytes): {url}", bytes.len()),
        });
    }
    serde_json::from_slice(&bytes).map_err(|e| HttpError::Status {
        code: 0,
        url: format!("JSON parse error: {e}"),
    })
}

/// Starts a streaming download, returning an error on non-2xx status.
pub async fn download_stream(
    client: &Client,
    url: &str,
) -> Result<reqwest::Response, HttpError> {
    let resp = client.get(url).send().await?.error_for_status()?;
    Ok(resp)
}

/// Retries a fallible async operation up to `max_retries` times with exponential backoff.
pub async fn with_retry<F, Fut, T, E>(mut f: F, max_retries: u32) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut attempt = 0;
    loop {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if attempt < max_retries => {
                attempt += 1;
                let delay = Duration::from_secs(1 << attempt.min(3));
                tokio::time::sleep(delay).await;
                tracing::warn!("retry {attempt}/{max_retries} after error: {e}");
            }
            Err(e) => return Err(e),
        }
    }
}
