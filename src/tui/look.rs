use std::path::PathBuf;
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

use crate::tui::Msg;
use crate::tui::paint::anim::ThemeFade;
use crate::tui::paint::gradient::Rgb;
use crate::tui::paint::theme::{THEMES, TOKYONIGHT, Theme};

#[derive(Clone, Debug, PartialEq)]
pub enum ThemeMode {
    Album,
    Omarchy,
    OmarchyAlbum,
    Builtin(usize),
}

impl ThemeMode {
    pub fn parse(s: &str) -> Option<ThemeMode> {
        match s {
            "album" => Some(ThemeMode::Album),
            "omarchy" => Some(ThemeMode::Omarchy),
            "omarchy+album" => Some(ThemeMode::OmarchyAlbum),
            name => THEMES.iter().position(|t| t.name == name).map(ThemeMode::Builtin),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ThemeMode::Album => "album",
            ThemeMode::Omarchy => "omarchy",
            ThemeMode::OmarchyAlbum => "omarchy+album",
            ThemeMode::Builtin(i) => THEMES[*i].name,
        }
    }

    fn next(&self) -> ThemeMode {
        match self {
            ThemeMode::Album => ThemeMode::Omarchy,
            ThemeMode::Omarchy => ThemeMode::OmarchyAlbum,
            ThemeMode::OmarchyAlbum => ThemeMode::Builtin(0),
            ThemeMode::Builtin(i) if i + 1 < THEMES.len() => ThemeMode::Builtin(i + 1),
            ThemeMode::Builtin(_) => ThemeMode::Album,
        }
    }
}

pub struct Look {
    pub mode: ThemeMode,
    omarchy: Option<Theme>,
    album: Option<Theme>,
    settled: Theme,
    fade: Option<ThemeFade>,
}

pub const FADE: Duration = Duration::from_millis(1500);

impl Look {
    pub fn new(mode: ThemeMode) -> Look {
        let mut look = Look { mode, omarchy: load_omarchy(), album: None, settled: TOKYONIGHT, fade: None };
        look.settled = look.target();
        look
    }

    /// The colors to draw this frame.
    pub fn theme(&self) -> Theme {
        self.fade.as_ref().map_or(self.settled, ThemeFade::current)
    }

    pub fn fading(&self) -> bool {
        self.fade.is_some()
    }

    fn target(&self) -> Theme {
        let om = self.omarchy.unwrap_or(TOKYONIGHT);
        match &self.mode {
            ThemeMode::Album => self.album.unwrap_or(om),
            ThemeMode::Omarchy => om,
            ThemeMode::OmarchyAlbum => match self.album {
                Some(a) => Theme {
                    name: "omarchy+album",
                    primary: a.primary,
                    secondary: a.secondary,
                    accent: a.accent,
                    border_active: a.primary,
                    ..om
                },
                None => om,
            },
            ThemeMode::Builtin(i) => THEMES[*i],
        }
    }

    fn retarget(&mut self) {
        self.fade = Some(ThemeFade::new(self.theme(), self.target(), FADE));
    }

    pub fn album_changed(&mut self, theme: Option<Theme>) {
        self.album = theme;
        if matches!(self.mode, ThemeMode::Album | ThemeMode::OmarchyAlbum) {
            self.retarget();
        }
    }

    pub fn reload_omarchy(&mut self) {
        self.omarchy = load_omarchy();
        if matches!(self.mode, ThemeMode::Omarchy | ThemeMode::OmarchyAlbum) {
            self.retarget();
        }
    }

    pub fn cycle(&mut self) {
        self.mode = self.mode.next();
        self.retarget();
    }

    /// Settle a finished fade.
    pub fn tick(&mut self) {
        if let Some(f) = self.fade.as_ref().filter(|f| f.is_done()) {
            self.settled = f.target();
            self.fade = None;
        }
    }
}

/// Omarchy writes this path itself; it is not under XDG_STATE_HOME.
fn omarchy_dir() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".local/state/omarchy/current"))
}

pub fn load_omarchy() -> Option<Theme> {
    let raw = std::fs::read_to_string(omarchy_dir()?.join("theme/colors.toml")).ok()?;
    let t: toml::Table = toml::from_str(&raw).ok()?;
    let g = |k: &str| t.get(k).and_then(|v| v.as_str()).map(Rgb::from_hex);
    let fg = g("foreground")?;
    let muted = g("muted").unwrap_or(fg);
    let sel = g("selection").unwrap_or(muted);
    let acc = g("accent").unwrap_or(fg);
    Some(Theme {
        name: "omarchy",
        primary: acc,
        secondary: g("blue").unwrap_or(acc),
        accent: g("magenta").unwrap_or(acc),
        error: g("red").unwrap_or(acc),
        warning: g("yellow").unwrap_or(acc),
        success: g("green").unwrap_or(acc),
        info: g("cyan").unwrap_or(acc),
        text: fg,
        text_muted: muted,
        background: g("background")?,
        background_panel: g("dark_background").or(g("background"))?,
        background_element: sel,
        border: muted,
        border_active: acc,
        border_subtle: sel,
        border_dimmest: sel,
        transparent: false,
    })
}

/// Watches the *parent* of `theme/`, non-recursively: omarchy-theme-set swaps
/// the `theme` directory, which would kill a watch on it.
pub fn watch_omarchy(tx: Sender<Msg>) -> Option<notify::RecommendedWatcher> {
    let dir = omarchy_dir()?;
    let (raw_tx, raw_rx) = channel::<()>();
    let mut watcher = notify::recommended_watcher(move |ev: notify::Result<notify::Event>| {
        if ev.is_ok() {
            let _ = raw_tx.send(());
        }
    })
    .ok()?;
    watcher.watch(&dir, RecursiveMode::NonRecursive).ok()?;
    std::thread::spawn(move || {
        while raw_rx.recv().is_ok() {
            std::thread::sleep(Duration::from_millis(100));
            while raw_rx.try_recv().is_ok() {}
            if tx.send(Msg::OmarchyChanged).is_err() {
                return;
            }
        }
    });
    Some(watcher)
}
