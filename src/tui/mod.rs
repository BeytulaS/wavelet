pub mod art;
pub mod browse;
pub mod draw;
pub mod input;
pub mod look;
#[rustfmt::skip]
pub mod paint;
pub mod spectrum;

use std::collections::VecDeque;
use std::io::Stdout;
use std::sync::Arc;
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui_image::picker::{Picker, ProtocolType};
use url::Url;

use crate::config::Config;
use crate::domain::SearchId;
use crate::player::api::{Notice, Now, StationView, TrackView};
use crate::player::{Out, PlayerHandle, Snapshot};
use crate::sources::Http;
use crate::tui::paint::cover::Cover;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayoutKind {
    Myx,
    Stage,
    List,
}

impl LayoutKind {
    pub fn parse(s: &str) -> Option<LayoutKind> {
        match s {
            "myx" => Some(LayoutKind::Myx),
            "stage" => Some(LayoutKind::Stage),
            "list" => Some(LayoutKind::List),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            LayoutKind::Myx => "myx",
            LayoutKind::Stage => "stage",
            LayoutKind::List => "list",
        }
    }
    pub fn next(self) -> LayoutKind {
        match self {
            LayoutKind::Myx => LayoutKind::Stage,
            LayoutKind::Stage => LayoutKind::List,
            LayoutKind::List => LayoutKind::Myx,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Mode {
    Normal,
    Search { draft: String },
    Help,
}

pub struct Ui {
    pub player: PlayerHandle,
    pub http: Http,
    pub tx: Sender<Msg>,
    /// Latest snapshot and when it arrived; clocks extrapolate from `received`.
    pub snap: Arc<Snapshot>,
    pub received: Instant,
    pub browse: browse::Browse,
    pub mode: Mode,
    pub layout: LayoutKind,
    pub keys: input::KeyState,
    pub keymap: input::Keymap,
    pub look: look::Look,
    pub art: art::Art,
    pub spectrum: spectrum::Spectrum,
    pub toasts: Toasts,
    pub last_frame: draw::FrameOut,
    pub force_repaint: bool,
    pub quit: bool,
    pub spin: usize,
}

/// Everything the TUI thread can receive.
pub enum Msg {
    Term(crossterm::event::Event),
    Player(Out),
    Search { tab: browse::Tab, ticket: SearchId, result: Result<Vec<crate::domain::Playable>, String> },
    Art(art::Loaded),
    OmarchyChanged,
}

pub struct Toasts {
    items: VecDeque<(Notice, Instant)>,
}

pub const TOAST_LIFE: Duration = Duration::from_secs(3);

impl Toasts {
    pub fn push(&mut self, n: Notice, now: Instant) {
        if self.items.len() == 4 {
            self.items.pop_front();
        }
        self.items.push_back((n, now));
    }
    pub fn visible(&self) -> impl Iterator<Item = &Notice> {
        self.items.iter().map(|(n, _)| n)
    }
    pub fn expire(&mut self, now: Instant) {
        self.items.retain(|(_, at)| now.saturating_duration_since(*at) < TOAST_LIFE);
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

fn now_art(s: &Snapshot) -> Option<&Url> {
    match &s.now {
        Some(Now::Track { track, .. }) => track.art.as_ref(),
        Some(Now::Station { station, .. }) => station.favicon.as_ref(),
        None => None,
    }
}

pub fn audible(s: &Snapshot) -> bool {
    match &s.now {
        Some(Now::Track { stage: TrackView::Playing { clock, .. }, .. }) => clock.running,
        Some(Now::Station { stage: StationView::OnAir { listened, .. }, .. }) => listened.running,
        _ => false,
    }
}

impl Ui {
    pub fn animating(&self) -> bool {
        let spinning = match &self.snap.now {
            Some(Now::Track { stage, .. }) => !matches!(stage, TrackView::Playing { .. }),
            Some(Now::Station { stage, .. }) => matches!(stage, StationView::Connecting),
            None => false,
        };
        let searching = [browse::Tab::YouTube, browse::Tab::Radio]
            .iter()
            .any(|t| self.browse.list(*t).is_some_and(|l| matches!(l.state, browse::Fetch::Loading(_))));
        self.look.fading()
            || (self.spectrum.enabled && self.spectrum.active())
            || self.toasts.any()
            || spinning
            || searching
            || self.keys.pending().is_some()
    }

    fn handle(&mut self, msg: Msg, now: Instant) {
        match msg {
            Msg::Term(ev) => input::on_event(self, ev, now),
            Msg::Player(Out::Snapshot(s)) => {
                if self.art.want(now_art(&s), &self.tx, &self.http) {
                    self.force_repaint = true;
                }
                self.spectrum.set_wanted(audible(&s));
                self.snap = s;
                self.received = now;
            }
            Msg::Player(Out::Notice(n)) => self.toasts.push(n, now),
            Msg::Search { tab, ticket, result } => self.browse.landed(tab, ticket, result, &self.player),
            Msg::Art(loaded) => {
                if let Some(theme) = self.art.accept(loaded) {
                    self.look.album_changed(Some(theme));
                    self.force_repaint = true;
                }
            }
            Msg::OmarchyChanged => {
                self.look.reload_omarchy();
                self.toasts.push(
                    Notice { level: crate::player::api::Level::Info, text: "omarchy theme reloaded".into() },
                    now,
                );
            }
        }
    }
}

pub struct Term {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    picker: Picker,
}

impl Term {
    /// Must run before any other thread starts: the image picker may rewrite
    /// environment variables while it queries the terminal (see `Cover::make_picker`).
    pub fn enter(images: Option<ProtocolType>) -> std::io::Result<Term> {
        enable_raw_mode()?;
        execute!(std::io::stdout(), EnterAlternateScreen)?;
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            prev(info);
        }));
        let picker = Cover::make_picker(images);
        let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
        terminal.clear()?;
        Ok(Term { terminal, picker })
    }
}

fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
}

impl Drop for Term {
    fn drop(&mut self) {
        restore();
    }
}

pub fn run(mut term: Term, cfg: Config, notices: Vec<Notice>, player: PlayerHandle, http: Http) -> std::io::Result<()> {
    let (tx, rx) = channel::<Msg>();
    let input_tx = tx.clone();
    std::thread::Builder::new().name("input".into()).spawn(move || {
        while let Ok(ev) = crossterm::event::read() {
            if input_tx.send(Msg::Term(ev)).is_err() {
                return;
            }
        }
    })?;
    let player_tx = tx.clone();
    player.subscribe(Box::new(move |out| player_tx.send(Msg::Player(out.clone())).is_ok()));
    let _watcher = look::watch_omarchy(tx.clone());

    let now = Instant::now();
    let mut ui = Ui {
        art: art::Art::new(term.picker.clone()),
        player,
        http,
        tx,
        snap: Arc::new(Snapshot::empty()),
        received: now,
        browse: browse::Browse::default(),
        mode: Mode::Normal,
        layout: cfg.layout,
        keys: input::KeyState::default(),
        keymap: input::Keymap::with_overrides(&cfg.keys),
        look: look::Look::new(cfg.theme),
        spectrum: spectrum::Spectrum::spawn(cfg.spectrum),
        toasts: Toasts { items: VecDeque::new() },
        last_frame: draw::FrameOut::default(),
        force_repaint: false,
        quit: false,
        spin: 0,
    };
    for n in notices {
        ui.toasts.push(n, now);
    }
    ui.browse.set_tab(browse::Tab::Radio, &ui.snap);
    ui.browse.submit(String::new(), &ui.tx, &ui.http);
    ui.browse.set_tab(browse::Tab::YouTube, &ui.snap);

    let mut last_draw = Instant::now() - IDLE_REDRAW;
    let mut dirty = true;
    while !ui.quit {
        let wait = if dirty {
            MIN_FRAME
        } else if ui.animating() {
            ANIM_FRAME
        } else {
            IDLE_REDRAW
        };
        match rx.recv_timeout(wait.saturating_sub(last_draw.elapsed())) {
            Ok(msg) => {
                ui.handle(msg, Instant::now());
                while let Ok(m) = rx.try_recv() {
                    ui.handle(m, Instant::now());
                }
                dirty = true;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if ui.quit || !should_draw(dirty, ui.animating(), last_draw.elapsed()) {
            continue;
        }
        let now = Instant::now();
        ui.look.tick();
        ui.toasts.expire(now);
        ui.spin = ui.spin.wrapping_add(1);
        let mut out = draw::FrameOut::default();
        term.terminal.draw(|f| out = draw::frame(f, &ui, now))?;
        ui.browse.set_offset(out.list_offset);
        ui.last_frame = out;
        ui.force_repaint = false;
        last_draw = now;
        dirty = false;
    }
    ui.spectrum.stop();
    ui.player.shutdown(Duration::from_secs(3));
    Ok(())
}
