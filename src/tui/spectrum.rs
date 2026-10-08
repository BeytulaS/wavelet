//! Spectrum from `pw-record --target wavelet` (mpv runs with --audio-client-name=wavelet).
//! A capture thread feeds the ported Myx analyzer; the renderer reads the bands.
//! The bands are written by one thread and read by one; the Mutex guards a
//! 128-float copy, try_lock on the render side so a draw never waits.
//!
//! Client-side on purpose: in the daemon future the TUI runs on the same machine
//! and captures the daemon's mpv node by name; the daemon never computes spectra
//! nobody draws.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::tui::paint::visualizer::VisBands;

pub struct Spectrum {
    pub bands: Arc<Mutex<VisBands>>,
    want: Arc<AtomicBool>,
    pub enabled: bool,
}

impl Spectrum {
    pub fn spawn(enabled: bool) -> Spectrum {
        todo!("port prototype spawn_capture with target 'wavelet'; respawns while wanted")
    }
    /// Capture only while enabled, visible in the layout, and audio is playing.
    pub fn set_wanted(&self, playing_and_visible: bool) {
        todo!()
    }
    pub fn active(&self) -> bool {
        todo!("enabled && bands.try_lock().is_active")
    }
}
