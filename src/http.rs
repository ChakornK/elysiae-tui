#![allow(dead_code)]
use std::time::Duration;

use reqwest::Client;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum HttpError {
    #[error("HTTP {code} from {url}")]
    Status { code: u16, url: String },
    #[error("response too large ({size} bytes) from {url}")]
    TooLarge { size: u64, url: String },
    #[error("JSON parse error for {url}: {detail}")]
    Parse { url: String, detail: String },
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
/// Streams the body with a 16 MB cap to prevent OOM on unbounded responses.
pub async fn fetch_json<T: serde::de::DeserializeOwned>(
    client: &Client,
    url: &str,
) -> Result<T, HttpError> {
    let mut resp = client.get(url).send().await?.error_for_status()?;
    if resp.content_length().is_some_and(|len| len > MAX_JSON_BODY as u64) {
        return Err(HttpError::TooLarge {
            size: resp.content_length().unwrap(),
            url: url.to_owned(),
        });
    }
    let mut buf = Vec::with_capacity(
        resp.content_length().unwrap_or(8192).min(MAX_JSON_BODY as u64) as usize
    );
    while let Some(chunk) = resp.chunk().await? {
        if buf.len() + chunk.len() > MAX_JSON_BODY {
            return Err(HttpError::TooLarge {
                size: (buf.len() + chunk.len()) as u64,
                url: url.to_owned(),
            });
        }
        buf.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&buf).map_err(|e| HttpError::Parse {
        url: url.to_owned(),
        detail: e.to_string(),
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
