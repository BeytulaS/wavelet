//! Domain types shared by the player core, the sources, and the TUI.
//! Every value here was parsed at a boundary (sources/, player/store.rs, config.rs)
//! and is trusted everywhere else. Nothing in this file knows about JSON, mpv, or ratatui.

use std::marker::PhantomData;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use url::Url;

/// 11-char YouTube video id, `[A-Za-z0-9_-]{11}`.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct VideoId(String);

impl TryFrom<String> for VideoId {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        VideoId::parse(&s).ok_or_else(|| format!("bad video id {s:?}"))
    }
}
impl From<VideoId> for String {
    fn from(v: VideoId) -> String {
        v.0
    }
}

impl VideoId {
    pub fn parse(s: &str) -> Option<VideoId> {
        let ok = s.len() == 11 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        ok.then(|| VideoId(s.to_string()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// radio-browser `stationuuid` (also what Radio Atlas stores as `uuid`).
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct StationId(String);

impl TryFrom<String> for StationId {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        StationId::parse(&s).ok_or_else(|| format!("bad station id {s:?}"))
    }
}
impl From<StationId> for String {
    fn from(v: StationId) -> String {
        v.0
    }
}

impl StationId {
    pub fn parse(s: &str) -> Option<StationId> {
        let s = s.trim().to_ascii_lowercase();
        let groups: Vec<&str> = s.split('-').collect();
        let shape = groups.iter().map(|g| g.len()).eq([8, 4, 4, 4, 12]);
        let hex = groups.iter().all(|g| g.bytes().all(|b| b.is_ascii_hexdigit()));
        (shape && hex).then_some(StationId(s))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ArtistRef {
    pub name: String,
    /// Present when InnerTube links the artist; artist pages (post-v1) need it.
    pub browse_id: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Track {
    pub id: VideoId,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub album: Option<String>,
    /// `None` when InnerTube omitted `lengthText`; mpv's `duration` fills the UI later.
    pub duration: Option<Duration>,
    pub art: Option<Url>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Station {
    pub id: StationId,
    pub name: String,
    /// After any user override (Radio Atlas `url-overrides.json`).
    pub stream: Url,
    pub homepage: Option<Url>,
    pub favicon: Option<Url>,
    pub country_code: Option<String>,
    pub tags: Vec<String>,
    pub codec: Option<String>,
    pub bitrate_kbps: Option<u32>,
}

/// One thing the queue can hold. `Arc` so the queue, playback, snapshots, and
/// favorites share one allocation; items are immutable after parsing.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Playable {
    Track(Arc<Track>),
    Station(Arc<Station>),
}

/// Identity of a playable, independent of metadata drift between sources.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum ItemKey {
    Video(VideoId),
    Station(StationId),
}

impl Playable {
    pub fn key(&self) -> ItemKey {
        match self {
            Playable::Track(t) => ItemKey::Video(t.id.clone()),
            Playable::Station(s) => ItemKey::Station(s.id.clone()),
        }
    }
    pub fn title(&self) -> &str {
        match self {
            Playable::Track(t) => &t.title,
            Playable::Station(s) => &s.name,
        }
    }
    pub fn art(&self) -> Option<&Url> {
        match self {
            Playable::Track(t) => t.art.as_ref(),
            Playable::Station(s) => s.favicon.as_ref(),
        }
    }
}

/// mpv volume, clamped to 0..=130 at construction.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Volume(u8);

impl Volume {
    pub const MAX: u8 = 130;
    pub fn new(v: i32) -> Volume {
        Volume(v.clamp(0, Self::MAX as i32) as u8)
    }
    pub fn get(self) -> u8 {
        self.0
    }
    pub fn nudge(self, by: i8) -> Volume {
        Volume::new(self.0 as i32 + by as i32)
    }
}

/// A resolved googlevideo URL and the monotonic deadline after which it must not be used.
/// Built only by `sources::ytdlp`, which reads `expire=` and subtracts five minutes,
/// so holding one proves the expiry was parsed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StreamUrl {
    pub url: Url,
    pub usable_until: Instant,
}

/// Correlates an async request with the slot that is waiting for it.
///
/// The single staleness rule of the whole app: the slot that issues a request
/// stores its ticket; a result is applied only if its ticket equals the one the
/// slot holds now. Replacing the slot's contents (new search, new queue, new load)
/// replaces the ticket, so late results fall on the floor without index checks.
/// `K` brands the slot so a search ticket can never satisfy a load slot.
#[derive(Debug, Serialize, Deserialize)]
pub struct Ticket<K>(u64, #[serde(skip)] PhantomData<K>);

impl<K> Clone for Ticket<K> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<K> Copy for Ticket<K> {}
impl<K> PartialEq for Ticket<K> {
    fn eq(&self, o: &Self) -> bool {
        self.0 == o.0
    }
}
impl<K> Eq for Ticket<K> {}

/// Ticket kinds. Zero-sized brands.
#[derive(Debug)]
pub enum Load {}
#[derive(Debug)]
pub enum Refill {}
#[derive(Debug)]
pub enum Search {}

/// One mpv `loadfile`. Sent as the IPC `request_id`, so mpv's reply binds it to a playlist entry id.
pub type LoadId = Ticket<Load>;
pub type RefillId = Ticket<Refill>;
pub type SearchId = Ticket<Search>;

/// Monotonic ticket source. One per actor; tickets never cross actors.
#[derive(Default, Debug)]
pub struct Tickets(u64);

impl Tickets {
    pub fn issue<K>(&mut self) -> Ticket<K> {
        self.0 += 1;
        Ticket(self.0, PhantomData)
    }
}

impl LoadId {
    pub fn request_id(self) -> u64 {
        self.0
    }
    pub fn from_request_id(id: u64) -> LoadId {
        Ticket(id, PhantomData)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_id_accepts_eleven_url_safe_chars_only() {
        assert!(VideoId::parse("dQw4w9WgXcQ").is_some());
        assert!(VideoId::parse("a-b_c123456").is_some());
        assert!(VideoId::parse("dQw4w9WgXc").is_none());
        assert!(VideoId::parse("dQw4w9WgXcQQ").is_none());
        assert!(VideoId::parse("dQw4w9WgX/Q").is_none());
    }

    #[test]
    fn station_id_is_a_lowercased_uuid() {
        let id = StationId::parse("960B6F0B-0601-11E8-AE97-52543BE04C81").expect("uuid");
        assert_eq!(id.as_str(), "960b6f0b-0601-11e8-ae97-52543be04c81");
        assert!(StationId::parse("960b6f0b-0601-11e8-ae97").is_none());
        assert!(StationId::parse("960b6f0g-0601-11e8-ae97-52543be04c81").is_none());
    }

    #[test]
    fn volume_clamps_at_construction_and_nudge() {
        assert_eq!(Volume::new(500).get(), Volume::MAX);
        assert_eq!(Volume::new(3).nudge(-5).get(), 0);
    }
}
