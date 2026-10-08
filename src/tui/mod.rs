//! The TUI client. Owns view state only: what is selected, what was searched,
//! the theme, the art, the spectrum. Player state arrives as `Arc<Snapshot>` and
//! is never mutated here; the TUI changes the player only by sending `Command`s.
//!
//! One thread (main). One inbox. Draw is a pure function of `&Ui` that returns
//! what it measured (`FrameOut`) instead of writing through `Cell`s.

pub mod art;
pub mod browse;
pub mod draw;
pub mod input;
pub mod look;
pub mod paint;
pub mod spectrum;

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::config::{Config, Issue};
use crate::domain::SearchId;
use crate::player::api::{Level, Notice};
use crate::player::{Out, PlayerHandle, Snapshot};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutKind {
    Myx,
    Stage,
    List,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Sidebar,
    Main,
}

/// What owns the keyboard right now. Search input and help swallow keys before the keymap.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Mode {
    Normal,
    Search { draft: String },
    Help,
}

pub struct Ui {
    pub player: PlayerHandle,
    /// Latest snapshot and when it arrived; clocks extrapolate from `received`.
    pub snap: Arc<Snapshot>,
    pub received: Instant,
    pub browse: browse::Browse,
    pub mode: Mode,
    pub focus: Focus,
    pub layout: LayoutKind,
    pub keys: input::KeyState,
    pub keymap: input::Keymap,
    pub look: look::Look,
    pub art: art::Art,
    pub spectrum: spectrum::Spectrum,
    pub toasts: Toasts,
    /// Measurements from the last draw: list viewport heights, scroll offsets, hit rects.
    pub last_frame: draw::FrameOut,
    pub repaint: ArtRepaint,
    pub quit: bool,
}

/// Everything the TUI thread can receive.
pub enum Msg {
    Term(crossterm::event::Event),
    Player(Out),
    Search { tab: browse::Tab, ticket: SearchId, result: Result<Vec<crate::domain::Playable>, String> },
    Art(art::Loaded),
    /// Something changed under ~/.local/state/omarchy/current/ (debounced 100 ms).
    OmarchyChanged,
}

/// Myx's art flicker handling, as is: blank the box one frame, then draw.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArtRepaint {
    Idle,
    Wipe,
    Draw,
}

pub struct Toasts {
    items: VecDeque<(Notice, Instant)>,
}

pub const TOAST_LIFE: Duration = Duration::from_secs(3);

impl Toasts {
    pub fn push(&mut self, n: Notice, now: Instant) {
        todo!("cap at 4; drop oldest")
    }
    pub fn visible(&self, now: Instant) -> impl Iterator<Item = (&Notice, f32 /*fade 1..0*/)> {
        let _ = now;
        self.items.iter().map(|(n, _)| (n, 1.0))
    }
    pub fn expire(&mut self, now: Instant) {
        todo!()
    }
    pub fn any(&self) -> bool {
        !self.items.is_empty()
    }
}

/// Myx `app/frame.rs` cadence.
pub const MIN_FRAME: Duration = Duration::from_millis(16);
pub const ANIM_FRAME: Duration = Duration::from_millis(33);
pub const IDLE_REDRAW: Duration = Duration::from_millis(500);

pub fn should_draw(dirty: bool, animating: bool, since_last: Duration) -> bool {
    if dirty {
        since_last >= MIN_FRAME
    } else if animating {
        since_last >= ANIM_FRAME
    } else {
        since_last >= IDLE_REDRAW
    }
}

impl Ui {
    /// Spectrum visible and receiving, theme fade running, toast fading, or a spinner up.
    pub fn animating(&self, now: Instant) -> bool {
        todo!()
    }

    fn handle(&mut self, msg: Msg, tx: &Sender<Msg>, now: Instant) {
        match msg {
            Msg::Term(ev) => input::on_event(self, ev, tx, now),
            Msg::Player(Out::Snapshot(s)) => {
                // TODO: if now-playing art URL changed -> self.art.want(url, tx)
                //       spectrum.set_wanted(status == Playing && cfg spectrum && layout shows it)
                self.snap = s;
                self.received = now;
            }
            Msg::Player(Out::Notice(n)) => self.toasts.push(n, now),
            Msg::Search { tab, ticket, result } => self.browse.landed(tab, ticket, result, &self.player),
            Msg::Art(loaded) => {
                // Identity check: only the art for what plays now is applied.
                if self.art.accept(loaded) {
                    self.look.album_changed(self.art.theme(), now);
                    self.repaint = ArtRepaint::Wipe;
                }
            }
            Msg::OmarchyChanged => {
                self.look.reload_omarchy(now);
                self.toasts.push(Notice { level: Level::Info, text: "theme reloaded".into() }, now);
            }
        }
    }
}

/// Owns the terminal. Returns when the user quits; sends `Shutdown` first.
pub fn run(cfg: Config, issues: Vec<Issue>, player: PlayerHandle) -> std::io::Result<()> {
    // TODO:
    //   terminal setup (raw, alt screen, mouse capture if cfg.mouse); picker = art::make_picker()
    //   let (tx, rx) = channel::<Msg>();
    //   thread: crossterm::event::read() -> Msg::Term
    //   player.subscribe(Box::new({ let tx = tx.clone(); move |out| tx.send(Msg::Player(out.clone())).is_ok() }))
    //   look::watch_omarchy(tx.clone())        // notify watcher on the parent dir
    //   ui.browse.load_top_stations(tx)        // radio tab starts with top-voted
    //   issues -> toasts
    //   loop {
    //       let wait = if ui.animating(now) { ANIM_FRAME } else { IDLE_REDRAW };
    //       recv_timeout(wait - since_last) -> handle(msg); drain try_recv; dirty = true
    //       if !should_draw(dirty, animating, since_last) { continue }
    //       ui.look.tick(now); ui.toasts.expire(now)
    //       terminal.draw(|f| ui.last_frame = draw::frame(f, &ui, now))
    //       ui.repaint = advance(ui.repaint)
    //       if ui.quit { player.send(Shutdown); break }
    //   }
    let _: Option<(Receiver<Msg>, Config, Vec<Issue>, PlayerHandle)> = None;
    let _ = (cfg, issues, player);
    todo!()
}
