//! What travels over `wavelet.sock`: newline-delimited JSON, serde's default
//! enum tagging. Clients send `Command` lines; the daemon sends `ServerLine`s.

use std::os::unix::fs::MetadataExt;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::player::api::Out;

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum ServerLine {
    Hello { build: BuildId },
    Out(Out),
}

/// Identity of the executable file: "<dev>:<inode>:<mtime_ns>", read once.
/// A reinstall changes it, so a client can tell it is talking to an old daemon.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct BuildId(String);

impl BuildId {
    pub fn current() -> &'static BuildId {
        static ID: OnceLock<BuildId> = OnceLock::new();
        // /proc/self/exe follows the running image even after `cargo install`
        // replaced the file at current_exe()'s path.
        ID.get_or_init(|| match std::fs::metadata("/proc/self/exe") {
            Ok(m) => BuildId(format!(
                "{}:{}:{}",
                m.dev(),
                m.ino(),
                m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128
            )),
            Err(e) => BuildId(format!("unknown:{e}")),
        })
    }
}

/// One wire line, newline included.
pub fn line<T: Serialize>(value: &T) -> String {
    let mut s = serde_json::to_string(value).unwrap_or_default();
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::domain::{Playable, VideoId, Volume};
    use crate::player::api::{
        ClockView, Command, Favorites, Level, Notice, Now, QueueRow, RadioStatus, SeekTo, Snapshot, StationView,
        TrackView, VolumeTo,
    };
    use crate::player::queue::EntryId;
    use crate::player::queue::tests::{station, track, vid};

    fn round_trip<T: Serialize + for<'de> Deserialize<'de>>(v: &T) -> T {
        let l = line(v);
        assert!(l.ends_with('\n') && !l[..l.len() - 1].contains('\n'), "one value per line: {l}");
        serde_json::from_str(&l).expect("parses back")
    }

    /// Adding a `Command` variant breaks this match, which sends the author here
    /// to add it to `every_command`.
    fn variant(c: &Command) -> usize {
        match c {
            Command::PlayTrack(_) => 0,
            Command::PlayStations { .. } => 1,
            Command::PlayNext(_) => 2,
            Command::Next => 3,
            Command::Prev => 4,
            Command::Jump(_) => 5,
            Command::Remove(_) => 6,
            Command::TogglePause => 7,
            Command::SetPaused(_) => 8,
            Command::Seek(_) => 9,
            Command::Volume(_) => 10,
            Command::SetFavorite { .. } => 11,
            Command::Warm(_) => 12,
            Command::Shutdown => 13,
        }
    }
    const VARIANTS: usize = 14;

    fn every_command() -> Vec<Command> {
        let s = station(3);
        vec![
            Command::PlayTrack(track(&vid('a'))),
            Command::PlayStations { stations: vec![station(1), s.clone()], start: s.id.clone() },
            Command::PlayNext(Playable::Station(s.clone())),
            Command::Next,
            Command::Prev,
            Command::Jump(EntryId::from_raw(7)),
            Command::Remove(EntryId::from_raw(8)),
            Command::TogglePause,
            Command::SetPaused(true),
            Command::Seek(SeekTo::By(-5)),
            Command::Seek(SeekTo::At(Duration::from_millis(61_500))),
            Command::Volume(VolumeTo::By(-5)),
            Command::Volume(VolumeTo::At(Volume::new(60))),
            Command::SetFavorite { item: Playable::Track(track(&vid('b'))), on: false },
            Command::Warm(vec![VideoId::parse(&vid('c')).expect("id")]),
            Command::Shutdown,
        ]
    }

    #[test]
    fn every_command_survives_the_wire() {
        let all = every_command();
        let mut seen: Vec<usize> = all.iter().map(variant).collect();
        seen.dedup();
        assert_eq!(seen, (0..VARIANTS).collect::<Vec<_>>(), "every variant, in order");
        for c in all {
            assert_eq!(round_trip(&c), c);
        }
    }

    #[test]
    fn a_snapshot_hello_and_notice_survive_the_wire() {
        let t = track(&vid('a'));
        let s = station(2);
        let snap = Snapshot {
            now: Some(Now::Track {
                entry: EntryId::from_raw(4),
                track: t.clone(),
                stage: TrackView::Playing {
                    clock: ClockView { pos: Duration::from_millis(12_345), running: true },
                    duration: Some(Duration::from_secs(200)),
                },
            }),
            queue: vec![
                QueueRow { id: EntryId::from_raw(4), item: Playable::Track(t), from_radio: false },
                QueueRow { id: EntryId::from_raw(5), item: Playable::Station(s.clone()), from_radio: true },
            ],
            cursor: Some(0),
            volume: Volume::new(60),
            radio: RadioStatus::Following { seed: VideoId::parse(&vid('a')).expect("id"), fetching: true, dry: false },
            favorites: Favorites { tracks: vec![], stations: vec![s] },
        };
        let on_air = Now::Station {
            entry: EntryId::from_raw(5),
            station: station(2),
            stage: StationView::OnAir {
                listened: ClockView { pos: Duration::ZERO, running: false },
                icy: Some("Artist - Song".into()),
            },
        };
        let lines = [
            ServerLine::Hello { build: BuildId::current().clone() },
            ServerLine::Out(Out::Snapshot(Arc::new(snap.clone()))),
            ServerLine::Out(Out::Snapshot(Arc::new(Snapshot { now: Some(on_air), ..snap }))),
            ServerLine::Out(Out::Snapshot(Arc::new(Snapshot::empty()))),
            ServerLine::Out(Out::Notice(Notice { level: Level::Warn, text: "x".into() })),
        ];
        for l in lines {
            assert_eq!(round_trip(&l), l);
        }
    }

    #[test]
    fn the_build_id_names_this_executable_file() {
        let id = &BuildId::current().0;
        let parts: Vec<&str> = id.split(':').collect();
        assert_eq!(parts.len(), 3, "{id}");
        assert!(parts.iter().all(|p| p.parse::<i128>().is_ok()), "{id}");
        assert_eq!(BuildId::current(), BuildId::current());
    }
}
