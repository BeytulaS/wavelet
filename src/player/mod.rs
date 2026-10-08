//! The player: everything a daemon will own later. No ratatui types anywhere
//! under this module; `cargo tree -e normal -i ratatui` from a future
//! `wavelet-core` crate split must come back empty.

pub mod api;
pub mod core;
pub mod mpv;
pub mod queue;
pub mod shell;
pub mod store;

pub use api::{Command, Out, Snapshot};
pub use shell::{PlayerHandle, PlayerOpts, Sink, spawn};
