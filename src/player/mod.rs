//! The player: everything a daemon will own later. No UI types anywhere under
//! this module (tests/layering.rs checks).

pub mod api;
pub mod core;
pub mod mpv;
pub mod queue;
pub mod shell;
pub mod store;

pub use api::{Command, Out, Snapshot};
pub use shell::{PlayerHandle, PlayerOpts, Sink, spawn};
