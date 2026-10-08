use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::domain::{Playable, Station, StationId, Track, VideoId, Volume};
use crate::player::core::{Library, Session};

#[derive(Clone, Debug)]
pub struct Store {
    library: PathBuf,
    session: PathBuf,
}

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct LibraryFile {
    version: u32,
    tracks: Vec<Arc<Track>>,
    stations: Vec<Arc<Station>>,
    radio_atlas_imported: bool,
}

#[derive(Serialize, Deserialize)]
struct SessionFile {
    version: u32,
    queue: Vec<Value>,
    cursor: Option<usize>,
    radio_seed: Option<VideoId>,
    volume: Volume,
    position_ms: Option<u64>,
}

/// Radio Atlas `state.json` favorites. Wire type, never leaves this module.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtlasStation {
    uuid: String,
    name: String,
    url: String,
    #[serde(default)]
    homepage: String,
    #[serde(default)]
    favicon: String,
    #[serde(default)]
    country_code: String,
    #[serde(default)]
    tags: String,
    #[serde(default)]
    codec: String,
    #[serde(default)]
    bitrate: u32,
}

#[derive(Deserialize)]
struct AtlasState {
    #[serde(default)]
    favorites: Vec<AtlasStation>,
}

impl Store {
    pub fn at_xdg() -> Option<Store> {
        Some(Store::at(dirs::data_dir()?.join("wavelet"), dirs::state_dir()?.join("wavelet")))
    }

    pub fn at(data_dir: PathBuf, state_dir: PathBuf) -> Store {
        Store { library: data_dir.join("library.json"), session: state_dir.join("session.json") }
    }

    pub fn load_library(&self) -> (Library, Option<String>) {
        let Ok(text) = std::fs::read_to_string(&self.library) else { return (Library::default(), None) };
        match serde_json::from_str::<LibraryFile>(&text) {
            Ok(f) => {
                (Library { tracks: f.tracks, stations: f.stations, radio_atlas_imported: f.radio_atlas_imported }, None)
            }
            Err(e) => {
                let bad = self.library.with_extension("json.bad");
                let _ = std::fs::rename(&self.library, &bad);
                (Library::default(), Some(format!("library.json unreadable ({e}); moved to {}", bad.display())))
            }
        }
    }

    pub fn load_session(&self) -> Option<Session> {
        let f: SessionFile = serde_json::from_str(&std::fs::read_to_string(&self.session).ok()?).ok()?;
        let mut queue = Vec::new();
        let mut cursor = f.cursor;
        for (i, v) in f.queue.into_iter().enumerate() {
            match serde_json::from_value::<Playable>(v) {
                Ok(p) => queue.push(p),
                Err(_) => {
                    cursor = match cursor {
                        Some(c) if c == i => None,
                        Some(c) if i < c => Some(c - 1),
                        c => c,
                    }
                }
            }
        }
        Some(Session {
            queue,
            cursor,
            radio_seed: f.radio_seed,
            volume: f.volume,
            position: f.position_ms.map(Duration::from_millis),
        })
    }

    pub fn save_library(&self, lib: &Library) -> std::io::Result<()> {
        let f = LibraryFile {
            version: VERSION,
            tracks: lib.tracks.clone(),
            stations: lib.stations.clone(),
            radio_atlas_imported: lib.radio_atlas_imported,
        };
        write_atomic(&self.library, &serde_json::to_vec_pretty(&f)?)
    }

    pub fn save_session(&self, s: &Session) -> std::io::Result<()> {
        let queue = s.queue.iter().map(serde_json::to_value).collect::<Result<_, _>>()?;
        let f = SessionFile {
            version: VERSION,
            queue,
            cursor: s.cursor,
            radio_seed: s.radio_seed.clone(),
            volume: s.volume,
            position_ms: s.position.map(|p| p.as_millis() as u64),
        };
        write_atomic(&self.session, &serde_json::to_vec(&f)?)
    }
}

pub fn import_radio_atlas(dir: &Path) -> Result<Vec<Arc<Station>>, String> {
    let state = std::fs::read_to_string(dir.join("state.json")).map_err(|e| e.to_string())?;
    let state: AtlasState = serde_json::from_str(&state).map_err(|e| e.to_string())?;
    let overrides: HashMap<String, String> = std::fs::read_to_string(dir.join("url-overrides.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    Ok(state
        .favorites
        .into_iter()
        .filter_map(|a| {
            let url = overrides.get(&a.uuid).unwrap_or(&a.url);
            Some(Arc::new(Station {
                id: StationId::parse(&a.uuid)?,
                stream: Url::parse(url.trim()).ok()?,
                name: a.name.trim().to_string(),
                homepage: Url::parse(a.homepage.trim()).ok(),
                favicon: Url::parse(a.favicon.trim()).ok(),
                country_code: crate::sources::radio_browser::non_empty(a.country_code),
                tags: crate::sources::radio_browser::split_tags(&a.tags),
                codec: crate::sources::radio_browser::non_empty(a.codec),
                bitrate_kbps: (a.bitrate > 0).then_some(a.bitrate),
            }))
        })
        .collect())
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::queue::tests::{station, track, vid};

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wavelet-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn library_round_trips_tracks_and_stations() {
        let d = temp("lib");
        let store = Store::at(d.join("data"), d.join("state"));
        assert_eq!(store.load_library(), (Library::default(), None), "missing file is the default");
        let lib = Library { tracks: vec![track(&vid('a'))], stations: vec![station(7)], radio_atlas_imported: true };
        store.save_library(&lib).expect("save");
        assert_eq!(store.load_library(), (lib, None));
        assert!(!d.join("data/library.tmp").exists(), "the temp file was renamed away");
    }

    #[test]
    fn a_corrupt_library_is_moved_aside_with_a_warning() {
        let d = temp("bad");
        let store = Store::at(d.clone(), d.clone());
        write_atomic(&d.join("library.json"), b"{not json").expect("write");
        let (lib, warn) = store.load_library();
        assert_eq!(lib, Library::default());
        assert!(warn.is_some());
        assert!(d.join("library.json.bad").exists());
    }

    #[test]
    fn session_round_trips_and_a_bad_entry_drops_alone() {
        let d = temp("session");
        let store = Store::at(d.clone(), d.clone());
        let s = Session {
            queue: vec![
                Playable::Track(track(&vid('a'))),
                Playable::Station(station(2)),
                Playable::Track(track(&vid('c'))),
            ],
            cursor: Some(2),
            radio_seed: VideoId::parse(&vid('a')),
            volume: Volume::new(55),
            position: Some(Duration::from_millis(61_500)),
        };
        store.save_session(&s).expect("save");
        assert_eq!(store.load_session(), Some(s.clone()));

        let mut raw: Value =
            serde_json::from_str(&std::fs::read_to_string(d.join("session.json")).expect("read")).expect("json");
        raw["queue"][0]["Track"]["id"] = Value::String("bad".into());
        write_atomic(&d.join("session.json"), raw.to_string().as_bytes()).expect("write");
        let loaded = store.load_session().expect("still loads");
        assert_eq!(loaded.queue, s.queue[1..].to_vec());
        assert_eq!(loaded.cursor, Some(1), "the cursor follows its entry");
    }

    #[test]
    fn radio_atlas_import_maps_fields_and_applies_overrides() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/radio-atlas");
        let st = import_radio_atlas(&dir).expect("imports");
        assert_eq!(st.len(), 2, "the bad uuid is skipped");
        assert_eq!(st[0].stream.as_str(), "https://nova-radio.example/thevoice", "override wins");
        assert_eq!(st[0].tags, vec!["pop", "dance"]);
        assert_eq!(
            (st[0].country_code.as_deref(), st[0].codec.as_deref(), st[0].bitrate_kbps),
            (Some("BG"), Some("AAC"), Some(105))
        );
        assert_eq!(st[1].name, "Jazz FM");
        assert_eq!((st[1].codec.clone(), st[1].bitrate_kbps, st[1].country_code.clone()), (None, None, None));
        assert!(st[1].favicon.is_some());

        let mut lib = Library::default();
        assert_eq!(lib.merge_stations(st.clone()), 2);
        assert_eq!(lib.merge_stations(st), 0, "a second import adds nothing");
    }
}
