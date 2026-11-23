use std::time::Duration;

use reqwest::Client;

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
