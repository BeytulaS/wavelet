//! status.json: the bar's view of the player. A projection of `Snapshot` that
//! leaves out the clock, so it changes only on real events.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::{ArtistRef, Playable, StationId};
use crate::player::api::{Now, Snapshot, StationView, TrackView};
use crate::player::store::write_atomic;

pub const UP_NEXT: usize = 10;

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct BarStatus {
    pub state: PlayState,
    pub kind: Option<Kind>,
    pub title: String,
    pub subtitle: String,
    pub station_id: Option<StationId>,
    pub volume: u8,
    pub stations: Vec<StationRow>,
    pub up_next: Vec<UpNextRow>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayState {
    Off,
    Idle,
    Loading,
    Playing,
    Paused,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Track,
    Station,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct StationRow {
    pub id: StationId,
    pub name: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct UpNextRow {
    pub entry: u64,
    pub title: String,
    pub subtitle: String,
}

/// What `wavelet ctl status` prints when no daemon answers.
pub const OFF_JSON: &str = r#"{"state":"off"}"#;

pub fn project(s: &Snapshot) -> BarStatus {
    let (state, item, icy) = match &s.now {
        Some(Now::Track { track, stage, .. }) => {
            let state = match stage {
                TrackView::Resolving | TrackView::Loading => PlayState::Loading,
                TrackView::Playing { clock, .. } => running(clock.running),
            };
            (state, Some(Playable::Track(track.clone())), None)
        }
        Some(Now::Station { station, stage, .. }) => {
            let (state, icy) = match stage {
                StationView::Connecting => (PlayState::Loading, None),
                StationView::OnAir { listened, icy } => (running(listened.running), icy.clone()),
            };
            (state, Some(Playable::Station(station.clone())), icy)
        }
        None => (PlayState::Idle, s.cursor.and_then(|c| s.queue.get(c)).map(|r| r.item.clone()), None),
    };
    let (kind, title, subtitle, station_id) = match &item {
        None => (None, String::new(), String::new(), None),
        Some(Playable::Track(t)) => (Some(Kind::Track), t.title.clone(), artists(&t.artists), None),
        Some(Playable::Station(st)) => match icy {
            Some(icy) => (Some(Kind::Station), icy, st.name.clone(), Some(st.id.clone())),
            None => (Some(Kind::Station), st.name.clone(), String::new(), Some(st.id.clone())),
        },
    };
    let from = s.cursor.map_or(0, |c| c + 1);
    BarStatus {
        state,
        kind,
        title,
        subtitle,
        station_id,
        volume: s.volume.get(),
        stations: s
            .favorites
            .items
            .iter()
            .filter_map(|i| match i {
                Playable::Station(st) => Some(StationRow { id: st.id.clone(), name: st.name.clone() }),
                Playable::Track(_) => None,
            })
            .collect(),
        up_next: s
            .queue
            .iter()
            .skip(from)
            .take(UP_NEXT)
            .map(|r| UpNextRow {
                entry: r.id.get(),
                title: r.item.title().to_string(),
                subtitle: match &r.item {
                    Playable::Track(t) => artists(&t.artists),
                    Playable::Station(_) => String::new(),
                },
            })
            .collect(),
    }
}

fn running(r: bool) -> PlayState {
    if r { PlayState::Playing } else { PlayState::Paused }
}

fn artists(a: &[ArtistRef]) -> String {
    a.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")
}

impl BarStatus {
    /// The daemon's last word. Keeps the favorite stations and the volume, so
    /// the bar can still offer a station to start.
    pub fn stopped(&self) -> BarStatus {
        BarStatus {
            state: PlayState::Off,
            kind: None,
            title: String::new(),
            subtitle: String::new(),
            station_id: None,
            up_next: vec![],
            ..self.clone()
        }
    }
}

/// Writes status.json by temp file and rename, and only when the value changed.
pub struct StatusFile {
    path: PathBuf,
    last: Option<BarStatus>,
}

impl StatusFile {
    pub fn new(path: PathBuf) -> StatusFile {
        StatusFile { path, last: None }
    }

    pub fn last(&self) -> Option<&BarStatus> {
        self.last.as_ref()
    }

    pub fn write(&mut self, status: BarStatus) -> std::io::Result<()> {
        if self.last.as_ref() == Some(&status) {
            return Ok(());
        }
        write_atomic(&self.path, &serde_json::to_vec(&status)?)?;
        self.last = Some(status);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::domain::{Station, Track, Volume};
    use crate::player::api::{ClockView, Favorites, QueueRow, RadioStatus};
    use crate::player::queue::EntryId;
    use crate::player::queue::tests::{station, track, vid};

    fn song(c: char, artists: &[&str]) -> Arc<Track> {
        let mut t = (*track(&vid(c))).clone();
        t.artists = artists.iter().map(|a| ArtistRef { name: a.to_string(), browse_id: None }).collect();
        Arc::new(t)
    }

    fn row(n: u64, item: Playable) -> QueueRow {
        QueueRow { id: EntryId::from_raw(n), item, from_radio: false }
    }

    fn snap(now: Option<Now>, queue: Vec<QueueRow>, cursor: Option<usize>) -> Snapshot {
        Snapshot {
            now,
            queue,
            cursor,
            volume: Volume::new(60),
            radio: RadioStatus::Off,
            favorites: Favorites {
                items: vec![
                    Playable::Track(track(&vid('z'))),
                    Playable::Station(station(2)),
                    Playable::Station(station(1)),
                ],
            },
        }
    }

    fn clock(running: bool) -> ClockView {
        ClockView { pos: Duration::from_secs(42), running }
    }

    fn playing_track(running: bool) -> Snapshot {
        let a = song('a', &["Daft Punk", "Romanthony"]);
        let b = song('b', &["Daft Punk"]);
        let now = Now::Track {
            entry: EntryId::from_raw(1),
            track: a.clone(),
            stage: TrackView::Playing { clock: clock(running), duration: None },
        };
        snap(Some(now), vec![row(1, Playable::Track(a)), row(2, Playable::Track(b))], Some(0))
    }

    fn on_air(st: Arc<Station>, icy: Option<&str>) -> Snapshot {
        let now = Now::Station {
            entry: EntryId::from_raw(1),
            station: st.clone(),
            stage: StationView::OnAir { listened: clock(true), icy: icy.map(String::from) },
        };
        snap(Some(now), vec![row(1, Playable::Station(st)), row(2, Playable::Station(station(9)))], Some(0))
    }

    #[test]
    fn a_playing_track_shows_title_artists_and_what_follows() {
        let s = project(&playing_track(true));
        assert_eq!(s.state, PlayState::Playing);
        assert_eq!(s.kind, Some(Kind::Track));
        assert_eq!(s.title, format!("title {}", vid('a')));
        assert_eq!(s.subtitle, "Daft Punk, Romanthony");
        assert_eq!(s.station_id, None);
        assert_eq!(s.volume, 60);
        assert_eq!(
            s.up_next,
            vec![UpNextRow { entry: 2, title: format!("title {}", vid('b')), subtitle: "Daft Punk".into() }]
        );
        assert_eq!(
            s.stations,
            vec![
                StationRow { id: station(2).id.clone(), name: "station 2".into() },
                StationRow { id: station(1).id.clone(), name: "station 1".into() },
            ],
            "favorite stations only, in favorites order"
        );
    }

    #[test]
    fn a_paused_track_is_paused() {
        assert_eq!(project(&playing_track(false)).state, PlayState::Paused);
    }

    #[test]
    fn a_station_with_icy_shows_the_song_over_the_station_name() {
        let s = project(&on_air(station(2), Some("Daft Punk - One More Time")));
        assert_eq!(
            (s.state, s.kind, s.title.as_str(), s.subtitle.as_str()),
            (PlayState::Playing, Some(Kind::Station), "Daft Punk - One More Time", "station 2")
        );
        assert_eq!(s.station_id, Some(station(2).id.clone()));
        assert_eq!(s.up_next[0].subtitle, "");
    }

    #[test]
    fn a_station_without_icy_shows_its_name_alone() {
        let s = project(&on_air(station(2), None));
        assert_eq!((s.title.as_str(), s.subtitle.as_str()), ("station 2", ""));
        assert_eq!(s.station_id, Some(station(2).id.clone()));
    }

    #[test]
    fn resolving_loading_and_connecting_are_loading() {
        let t = track(&vid('a'));
        for stage in [TrackView::Resolving, TrackView::Loading] {
            let now = Now::Track { entry: EntryId::from_raw(1), track: t.clone(), stage };
            let s = project(&snap(Some(now), vec![row(1, Playable::Track(t.clone()))], Some(0)));
            assert_eq!((s.state, s.title.clone()), (PlayState::Loading, t.title.clone()));
        }
        let now = Now::Station { entry: EntryId::from_raw(1), station: station(1), stage: StationView::Connecting };
        let s = project(&snap(Some(now), vec![row(1, Playable::Station(station(1)))], Some(0)));
        assert_eq!((s.state, s.title.as_str()), (PlayState::Loading, "station 1"));
    }

    #[test]
    fn a_restored_session_is_idle_and_describes_the_cursor_item() {
        let a = song('a', &["A"]);
        let b = song('b', &["B"]);
        let s = project(&snap(None, vec![row(1, Playable::Track(a)), row(2, Playable::Track(b.clone()))], Some(1)));
        assert_eq!(s.state, PlayState::Idle);
        assert_eq!((s.kind, s.title.as_str(), s.subtitle.as_str()), (Some(Kind::Track), b.title.as_str(), "B"));
        assert!(s.up_next.is_empty(), "nothing after the last entry");

        let restored_station = project(&snap(None, vec![row(1, Playable::Station(station(4)))], Some(0)));
        assert_eq!(restored_station.station_id, Some(station(4).id.clone()));
        assert_eq!(restored_station.state, PlayState::Idle);
    }

    #[test]
    fn nothing_queued_is_idle_and_empty() {
        let s = project(&Snapshot::empty());
        assert_eq!(
            (s.state, s.kind, s.title.as_str(), s.subtitle.as_str(), s.station_id),
            (PlayState::Idle, None, "", "", None)
        );
    }

    #[test]
    fn up_next_stops_at_ten() {
        let rows: Vec<QueueRow> = (0..15).map(|n| row(n + 100, Playable::Station(station(n as u8)))).collect();
        let s = project(&snap(None, rows, Some(2)));
        let entries: Vec<u64> = s.up_next.iter().map(|r| r.entry).collect();
        assert_eq!(entries, (103..113).collect::<Vec<_>>());
    }

    #[test]
    fn the_json_uses_the_contract_names() {
        let v = serde_json::to_value(project(&on_air(station(2), Some("x")))).expect("json");
        assert_eq!(v["state"], "playing");
        assert_eq!(v["kind"], "station");
        assert_eq!(v["station_id"], station(2).id.as_str());
        assert_eq!(v["stations"][0]["name"], "station 2");
        assert_eq!(v["up_next"][0]["entry"], 2);
        let off = serde_json::to_value(project(&Snapshot::empty()).stopped()).expect("json");
        assert_eq!((off["state"].as_str(), off["kind"].is_null()), (Some("off"), true));
        let printed: serde_json::Value = serde_json::from_str(OFF_JSON).expect("json");
        assert_eq!(printed["state"], "off");
    }

    #[test]
    fn the_file_is_rewritten_only_when_the_status_changes() {
        let dir = std::env::temp_dir().join(format!("wavelet-status-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("status.json");
        let mut f = StatusFile::new(path.clone());
        f.write(project(&playing_track(true))).expect("write");
        let first: BarStatus = serde_json::from_slice(&std::fs::read(&path).expect("read")).expect("json");
        assert_eq!(first, project(&playing_track(true)));
        std::fs::remove_file(&path).expect("rm");
        f.write(project(&playing_track(true))).expect("write");
        assert!(!path.exists(), "an equal status writes nothing");
        f.write(project(&playing_track(false))).expect("write");
        let second: BarStatus = serde_json::from_slice(&std::fs::read(&path).expect("read")).expect("json");
        assert_eq!(second.state, PlayState::Paused);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
