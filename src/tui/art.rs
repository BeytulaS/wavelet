//! Album art and station favicons: fetch, decode, derive the album theme, all off
//! the UI thread. Drawing uses the ported Myx `Cover` with its flicker handling.

use std::sync::mpsc::Sender;

use image::DynamicImage;
use ratatui_image::picker::Picker;
use url::Url;

use crate::tui::Msg;
use crate::tui::paint::{cover::Cover, theme::Theme};

pub struct Art {
    picker: Picker,
    /// The URL we want on screen. Results for any other URL are dropped.
    wanted: Option<Url>,
    shown: Option<(Url, Cover, Theme)>,
}

pub struct Loaded {
    pub url: Url,
    pub img: DynamicImage,
    pub theme: Theme,
}

impl Art {
    pub fn new(picker: Picker) -> Art {
        Art { picker, wanted: None, shown: None }
    }
    /// Now-playing changed. `None` clears the cover (glyph placeholder).
    pub fn want(&mut self, url: Option<Url>, tx: &Sender<Msg>) {
        // TODO: if url == wanted { return } wanted = url; spawn fetch+decode+reactive::derive_theme -> Msg::Art
        //       (disk cache in ~/.cache/wavelet/art/ keyed by URL hash: post-v1 unless cheap)
        todo!()
    }
    /// Identity check against `wanted`. Returns whether the cover changed.
    pub fn accept(&mut self, l: Loaded) -> bool {
        todo!()
    }
    pub fn theme(&self) -> Option<Theme> {
        self.shown.as_ref().map(|(_, _, t)| *t)
    }
    pub fn cover(&self) -> Option<&Cover> {
        self.shown.as_ref().map(|(_, c, _)| c)
    }
}
