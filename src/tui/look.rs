//! Where colors come from, and the cross-fade between them. Pure apart from
//! reading colors.toml and the watcher.

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::tui::Msg;
use crate::tui::paint::{anim::ThemeFade, theme::Theme};

#[derive(Clone, Debug, PartialEq)]
pub enum ThemeMode {
    Album,
    Omarchy,
    OmarchyAlbum,
    /// Built-in (tokyonight, catppuccin, rosepine, gruvbox) or ~/.config/wavelet/themes/<name>.toml.
    Named(String),
}

pub struct Look {
    pub mode: ThemeMode,
    omarchy: Option<Theme>,
    album: Option<Theme>,
    named: Option<Theme>,
    /// The theme drawn when no fade runs. A fade's end writes here.
    settled: Theme,
    fade: Option<ThemeFade>,
}

pub const FADE: Duration = Duration::from_millis(1500);

impl Look {
    /// The colors to draw this frame.
    pub fn theme(&self) -> Theme {
        self.fade.as_ref().map_or(self.settled, |f| f.current())
    }
    pub fn fading(&self) -> bool {
        self.fade.is_some()
    }
    /// Pure. Mode + available sources -> target. omarchy+album keeps omarchy
    /// backgrounds/text and takes primary/secondary/accent/border_active from album.
    /// Missing source falls back to omarchy, then tokyonight.
    fn target(&self) -> Theme {
        todo!()
    }
    /// Start a fade from whatever is on screen now to `target()`; no-op when equal.
    fn retarget(&mut self, now: Instant) {
        todo!()
    }
    pub fn album_changed(&mut self, theme: Option<Theme>, now: Instant) {
        todo!("album = theme; if mode uses album -> retarget")
    }
    pub fn reload_omarchy(&mut self, now: Instant) {
        todo!("omarchy = load_omarchy(); if mode uses omarchy -> retarget")
    }
    pub fn cycle(&mut self, now: Instant) {
        todo!()
    }
    /// Settle a finished fade.
    pub fn tick(&mut self, now: Instant) {
        let _ = now;
        todo!()
    }
}

/// ~/.local/state/omarchy/current/theme/colors.toml -> Theme, per the SPEC token table.
pub fn load_omarchy() -> Option<Theme> {
    todo!("port prototype omarchy_theme()")
}

/// Watch the *parent* `~/.local/state/omarchy/current/`, non-recursive:
/// omarchy-theme-set swaps the `theme` directory, which kills a watch on it.
/// Debounce 100 ms, then send Msg::OmarchyChanged. The watcher lives for the process.
pub fn watch_omarchy(tx: Sender<Msg>) -> Option<notify::RecommendedWatcher> {
    let _: PathBuf;
    todo!()
}
