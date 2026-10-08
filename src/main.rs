// Render code adapted from Myx (MIT, Haseeb Khalid); see NOTICE.
mod config;
mod domain;
mod mpris;
mod player;
mod sources;
mod tui;

use player::core::Restore;
use player::store::{Store, import_radio_atlas};

fn main() {
    // TODO:
    //   let (cfg, issues) = config::load();
    //   let store = Store::at_xdg();
    //   let (mut library, lib_issue) = store.load_library();
    //   if !library.radio_atlas_imported {
    //       if let Ok(st) = import_radio_atlas(&dirs::data_dir()/radio-atlas) { library.merge_stations(st) }
    //       library.radio_atlas_imported = true;          // even on Err: the import is offered once
    //   }
    //   let restore = Restore { library, session: store.load_session(), initial_volume: cfg.volume };
    //   let player = player::spawn(PlayerOpts { restore, audio_device, runtime_dir, store })?;
    //   mpris::spawn(player.clone());
    //   tui::run(cfg, issues, player)   // returns on q; sends Command::Shutdown before returning
    //
    // Later `wavelet daemon` = the same minus tui::run plus a socket listener that
    // turns each client into a Sink + a Command reader. `wavelet` = tui::run with a
    // PlayerHandle that speaks the socket.
    let _ = (Restore { library: Default::default(), session: None, initial_volume: domain::Volume::new(70) }, Store::at_xdg, import_radio_atlas);
    todo!()
}
