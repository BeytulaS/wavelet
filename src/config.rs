//! ~/.config/wavelet/config.toml and keymap.toml. Parsed once at startup.
//! A bad key yields an `Issue` (file, line, reason) and the default for that key
//! only; the TUI shows issues as toasts. Nothing downstream re-validates.

use std::path::PathBuf;

use serde::Deserialize;

use crate::domain::Volume;
use crate::tui::input::{Action, KeySeq};
use crate::tui::look::ThemeMode;
use crate::tui::LayoutKind;

#[derive(Debug, Clone)]
pub struct Config {
    pub layout: LayoutKind,
    pub theme: ThemeMode,
    pub transparent: bool,
    pub audio_device: Option<String>,
    pub volume: Volume,
    pub spectrum: bool,
    pub mouse: bool,
    /// Overrides layered onto `Keymap::default()`. `Action::Unbind` removes a default.
    pub keys: Vec<(KeySeq, Action)>,
}

#[derive(Debug, Clone)]
pub struct Issue {
    pub file: PathBuf,
    pub line: Option<usize>,
    pub reason: String,
}

/// Raw TOML shape; every field optional so a partial file is valid.
#[derive(Deserialize, Default)]
struct RawConfig {
    layout: Option<String>,
    theme: Option<String>,
    transparent: Option<bool>,
    audio_device: Option<String>,
    volume: Option<i64>,
    spectrum: Option<bool>,
    mouse: Option<bool>,
}

pub fn load() -> (Config, Vec<Issue>) {
    // TODO: config.toml missing -> write commented defaults, use defaults.
    //       Parse table; convert each field independently, pushing an Issue on failure.
    //       keymap.toml: `[keys]` table of "<seq>" = "<action-name>", e.g. "g q" = "go-queue", "n" = "next".
    //       KeySeq::parse and Action::from_str per entry; failures become Issues.
    let _ = RawConfig::default();
    todo!()
}
