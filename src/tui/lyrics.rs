use std::sync::Arc;
use std::sync::mpsc::Sender;

use crate::domain::{Lyrics, Track, VideoId};
use crate::sources::{Http, lrclib};
use crate::tui::Msg;

pub enum View {
    /// Nothing to look up: no song is playing.
    Idle,
    Loading,
    Found(Lyrics),
    Missing,
    Failed(String),
}

/// Lyrics for the playing song. Fetched only while the pane is open, once per song.
pub struct Pane {
    pub open: bool,
    pub view: View,
    /// The song `view` belongs to. Results for any other song are dropped.
    wanted: Option<VideoId>,
}

impl Default for Pane {
    fn default() -> Pane {
        Pane { open: false, view: View::Idle, wanted: None }
    }
}

impl Pane {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if matches!(self.view, View::Failed(_)) {
            self.wanted = None;
        }
    }

    pub fn sync(&mut self, track: Option<&Arc<Track>>, tx: &Sender<Msg>, http: &Http) {
        if !self.open || track.map(|t| &t.id) == self.wanted.as_ref() {
            return;
        }
        self.wanted = track.map(|t| t.id.clone());
        let Some(track) = track.cloned() else {
            self.view = View::Idle;
            return;
        };
        self.view = View::Loading;
        let (tx, http) = (tx.clone(), http.clone());
        std::thread::spawn(move || {
            let result = lrclib::fetch(&http, &track);
            let _ = tx.send(Msg::Lyrics { id: track.id.clone(), result });
        });
    }

    pub fn accept(&mut self, id: VideoId, result: Result<Option<Lyrics>, String>) {
        if self.wanted.as_ref() != Some(&id) {
            return;
        }
        self.view = match result {
            Ok(Some(l)) => View::Found(l),
            Ok(None) => View::Missing,
            Err(e) => View::Failed(e),
        };
    }

    /// Whether the pane needs frames faster than idle: a spinner, or a synced line that can change.
    pub fn animating(&self, audible: bool) -> bool {
        self.open
            && match &self.view {
                View::Loading => true,
                View::Found(Lyrics::Synced(_)) => audible,
                _ => false,
            }
    }
}
