use std::process::ExitCode;

use wavelet::config;
use wavelet::player::api::{Level, Notice};
use wavelet::player::core::Restore;
use wavelet::player::shell::runtime_dir;
use wavelet::player::store::{Store, import_radio_atlas};
use wavelet::player::{self, PlayerOpts};
use wavelet::{mpris, sources, tui};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("wavelet: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let (cfg, issues) = config::load();
    let mut notices: Vec<Notice> = issues.iter().map(|i| Notice { level: Level::Warn, text: i.to_string() }).collect();
    let store = Store::at_xdg().ok_or("no home directory for XDG paths")?;
    let (mut library, warning) = store.load_library();
    notices.extend(warning.map(|text| Notice { level: Level::Warn, text }));
    if !library.radio_atlas_imported {
        if let Some(dir) = dirs::data_dir().map(|d| d.join("radio-atlas"))
            && let Ok(stations) = import_radio_atlas(&dir)
        {
            let n = library.merge_stations(stations);
            notices.push(Notice { level: Level::Info, text: format!("imported {n} stations from Radio Atlas") });
        }
        library.radio_atlas_imported = true;
        if let Err(e) = store.save_library(&library) {
            notices.push(Notice { level: Level::Warn, text: format!("could not save favorites: {e}") });
        }
    }
    let restore = Restore { library, session: store.load_session(), initial_volume: cfg.volume };
    let http = sources::http()?;

    let term = tui::Term::enter().map_err(|e| e.to_string())?;
    let opts = PlayerOpts {
        restore,
        audio_device: cfg.audio_device.clone(),
        runtime_dir: runtime_dir(),
        store,
        http: http.clone(),
    };
    let player = player::spawn(opts).map_err(|e| e.to_string())?;
    mpris::spawn(player.clone());
    tui::run(term, cfg, notices, player, http).map_err(|e| e.to_string())
}
