//! Upstream services. Each module: blocking fetch functions plus pure parsers
//! from the upstream JSON to `domain` types. Wire shapes never leave a module.
//! Callers run these on worker threads; nothing here holds state.

pub mod radio_browser;
pub mod ytdlp;
pub mod ytm;

use std::time::Duration;

pub type Http = reqwest::blocking::Client;

/// Built once in `main` and cloned; clones share one connection pool.
pub fn http() -> Result<Http, String> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("wavelet/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())
}
