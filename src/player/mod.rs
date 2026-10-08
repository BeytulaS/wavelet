//! The player: everything a daemon will own later. No UI types anywhere under
//! this module (tests/layering.rs checks).

pub mod api;
pub mod core;
pub mod queue;

pub use api::{Command, Out, Snapshot};
