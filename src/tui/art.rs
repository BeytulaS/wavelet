use std::sync::mpsc::Sender;

use image::DynamicImage;
use ratatui_image::picker::Picker;
use url::Url;

use crate::sources::Http;
use crate::tui::Msg;
use crate::tui::paint::cover::Cover;
use crate::tui::paint::reactive::derive_theme;
use crate::tui::paint::theme::Theme;

pub struct Art {
    pub picker: Picker,
    /// The URL we want on screen. Results for any other URL are dropped.
    wanted: Option<Url>,
    cover: Option<Cover>,
}

pub struct Loaded {
    pub url: Url,
    pub img: DynamicImage,
    pub theme: Theme,
}

impl Art {
    pub fn new(picker: Picker) -> Art {
        Art { picker, wanted: None, cover: None }
    }

    pub fn want(&mut self, url: Option<&Url>, tx: &Sender<Msg>, http: &Http) -> bool {
        if url == self.wanted.as_ref() {
            return false;
        }
        self.wanted = url.cloned();
        let Some(url) = url.cloned() else {
            return self.cover.take().is_some();
        };
        let (tx, http) = (tx.clone(), http.clone());
        std::thread::spawn(move || {
            let Ok(bytes) = http.get(url.as_str()).send().and_then(|r| r.error_for_status()).and_then(|r| r.bytes())
            else {
                return;
            };
            let Ok(img) = image::load_from_memory(&bytes) else { return };
            let theme = derive_theme(&img, "album");
            let _ = tx.send(Msg::Art(Loaded { url, img, theme }));
        });
        false
    }

    pub fn accept(&mut self, l: Loaded) -> Option<Theme> {
        if self.wanted.as_ref() != Some(&l.url) {
            return None;
        }
        self.cover = Some(Cover::from_image(l.img, self.picker.clone()));
        Some(l.theme)
    }

    pub fn cover(&self) -> Option<&Cover> {
        self.cover.as_ref()
    }
}
