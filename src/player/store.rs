//! Files the player owns: library.json, session.json, and the one-time Radio
//! Atlas import. File schemas are private to this module; the core only sees
//! `Library` and `Session`. Writes are atomic (temp file + rename), so a crash
//! mid-write leaves the previous file, and writing the same value twice is a no-op
//! in effect.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::domain::Station;
use crate::player::core::{Library, Session};

pub struct Store {
    /// ~/.local/share/wavelet/library.json
    library: PathBuf,
    /// ~/.local/state/wavelet/session.json
    session: PathBuf,
}

/// On-disk shape. Versioned so SQLite (post-v1) can migrate from it.
#[derive(Serialize, Deserialize)]
struct LibraryFile {
    version: u32,
    tracks: Vec<crate::domain::Track>,
    stations: Vec<Station>,
    radio_atlas_imported: bool,
}

#[derive(Serialize, Deserialize)]
struct SessionFile {
    version: u32,
    queue: Vec<crate::domain::Playable>,
    cursor: Option<usize>,
    radio_seed: Option<crate::domain::VideoId>,
    volume: crate::domain::Volume,
    position_ms: Option<u64>,
}

/// Radio Atlas `state.json` favorites. Wire type, never leaves this module.
/// Fields seen on this machine: uuid, name, url, homepage, favicon, countryCode,
/// tags (comma string), codec, bitrate (0 = unknown).
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

impl Store {
    pub fn at_xdg() -> Store {
        todo!("dirs::data_dir()/wavelet/library.json, dirs::state_dir()/wavelet/session.json")
    }

    /// Missing file -> default. Corrupt file -> renamed to `.bad`, default, and a warning string.
    pub fn load_library(&self) -> (Library, Option<String>) {
        todo!()
    }
    pub fn load_session(&self) -> Option<Session> {
        todo!("missing or corrupt -> None; entries that fail to parse are dropped, not fatal")
    }
    pub fn save_library(&self, lib: &Library) {
        todo!("write_atomic(serde_json of LibraryFile)")
    }
    pub fn save_session(&self, s: &Session) {
        todo!()
    }
}

/// One-time import. `main` calls this when `!library.radio_atlas_imported`, merges,
/// and sets the flag, so a second run is a no-op even if the user deleted favorites.
/// Overrides (`url-overrides.json`, uuid -> url) replace `url` before parsing.
/// Stations whose uuid or url fail to parse are skipped.
pub fn import_radio_atlas(dir: &Path) -> Result<Vec<Arc<Station>>, String> {
    // TODO: read dir/state.json -> { favorites: [AtlasStation] }, dir/url-overrides.json -> HashMap<String,String>
    //       map AtlasStation -> Station { tags: split(',').trim, codec/country empty -> None, bitrate 0 -> None }
    todo!()
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    todo!("create_dir_all(parent); write path.tmp; fsync; rename")
}
