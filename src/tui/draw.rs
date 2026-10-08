//! Rendering. `frame(f, &ui, now)` is a function of `&Ui`; it returns what it
//! measured instead of mutating through Cells (the prototype's list_cap/offset).
//! Layout bodies are ported from the prototype nearly as is.

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::player::api::ClockView;
use crate::tui::Ui;
use crate::tui::paint::theme::Theme;

#[derive(Default, Debug, Clone)]
pub struct FrameOut {
    /// Rows the active list could show; drives half-page motions.
    pub list_rows: usize,
    /// Scroll offset chosen this frame for the active list (sticky; fed back next frame).
    pub list_offset: usize,
    pub hits: Vec<(Rect, Hit)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hit {
    ListRow(usize),
    Tab(crate::tui::browse::Tab),
    Progress { x0: u16, width: u16 },
    Volume { x0: u16, width: u16 },
}

pub fn frame(f: &mut Frame, ui: &Ui, now: Instant) -> FrameOut {
    // TODO: theme = ui.look.theme(); base fill; rows: header | body | strip | footer
    //       match ui.layout { Myx => myx, Stage => stage, List => list }
    //       toasts bottom-right; help overlay; prefix popup if ui.keys.pending() older than 300 ms
    //       ArtRepaint::Wipe -> wipe art rect (Myx wipe_area), Draw -> force_area
    let _ = (f, ui, now);
    todo!()
}

/// Position to draw: snapshot position extrapolated from its receive time while running.
pub fn clock_now(c: &ClockView, ui: &Ui, now: Instant) -> std::time::Duration {
    todo!()
}

fn myx(f: &mut Frame, ui: &Ui, t: Theme, body: Rect, out: &mut FrameOut) {
    todo!("sidebar list (favorites + results) | art, now lines, spectrum")
}
fn stage(f: &mut Frame, ui: &Ui, t: Theme, body: Rect, out: &mut FrameOut) {
    todo!()
}
fn list(f: &mut Frame, ui: &Ui, t: Theme, body: Rect, out: &mut FrameOut) {
    todo!("table + rail with art and UP NEXT from ui.snap.queue after cursor")
}
fn strip(f: &mut Frame, ui: &Ui, t: Theme, area: Rect, now: Instant, out: &mut FrameOut) {
    todo!("Track: gradient progress mm:ss; Live: LIVE pill + icy title + listened; volume bars")
}
