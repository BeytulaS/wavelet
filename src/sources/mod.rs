//! Upstream services. Each module: blocking fetch functions plus pure parsers
//! from the upstream JSON to `domain` types. Wire shapes never leave a module.
//! Callers run these on worker threads; nothing here holds state.

pub mod radio_browser;
pub mod ytdlp;
pub mod ytm;

pub fn http() -> reqwest::blocking::Client {
    todo!("user agent wavelet/<version>, 10 s timeout; built once in main and cloned")
}
