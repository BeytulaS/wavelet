//! Keys -> `Action` -> (view change | player `Command`).
//!
//! The keymap is data: a table from key sequence to `Action`, defaults below,
//! overrides from keymap.toml. `KeyState::feed` is a pure prefix/count machine.
//! `dispatch` is the one match that turns actions into effects, which is why
//! "user presses n" is three files: this one, player/core.rs, player/shell.rs.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent};

use crate::domain::Playable;
use crate::player::api::{Command, SeekTo, VolumeTo};
use crate::tui::{Focus, Mode, Msg, Ui};

/// Everything a key can mean. Names are the keymap.toml spelling (kebab-case).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Action {
    Down,
    Up,
    Top,
    Bottom,
    HalfDown,
    HalfUp,
    FocusLeft,
    FocusRight,
    CycleFocus,
    /// Enter: play the selection (track -> PlayTrack, station -> PlayStations of the list).
    Activate,
    TogglePause,
    Next,
    Prev,
    SeekBack,
    SeekForward,
    VolumeDown,
    VolumeUp,
    /// `a`: play the selection next.
    PlayNext,
    ToggleFavorite,
    Search,
    TabYouTube,
    TabRadio,
    CycleTab,
    GoQueue,
    GoSearch,
    CycleLayout,
    CycleTheme,
    ToggleSpectrum,
    Help,
    Quit,
    /// keymap.toml only: remove a default binding.
    Unbind,
}

impl FromStr for Action {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        todo!("kebab-case names; unknown -> Err(\"unknown action 'x'\")")
    }
}

/// A key with modifiers, normalized (shifted letters are their uppercase char, no SHIFT bit).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Key {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

/// "g g", "C-d", "space", "enter". Parsed from keymap.toml with space-separated keys.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct KeySeq(pub Vec<Key>);

impl KeySeq {
    pub fn parse(s: &str) -> Result<KeySeq, String> {
        todo!()
    }
}

pub struct Keymap {
    bindings: HashMap<KeySeq, Action>,
    /// Every proper prefix of a binding; derived from `bindings` at build time.
    prefixes: HashSet<KeySeq>,
}

impl Keymap {
    /// Vim defaults: j k gg G C-d C-u h l Tab Enter space n b H L - + a f / 1 2 gq gs w t v ? q.
    pub fn with_overrides(overrides: &[(KeySeq, Action)]) -> Keymap {
        todo!("defaults, apply overrides (Unbind removes), derive prefixes")
    }
    /// For the help overlay and the 300 ms prefix popup.
    pub fn bindings_starting_with<'a>(&'a self, prefix: &'a [Key]) -> impl Iterator<Item = (&'a KeySeq, Action)> + 'a {
        self.bindings.iter().filter(move |(k, _)| k.0.starts_with(prefix)).map(|(k, a)| (k, *a))
    }
}

#[derive(Default, Debug)]
pub struct KeyState {
    pending: Vec<Key>,
    count: Option<u32>,
    since: Option<Instant>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Fed {
    /// A prefix (or count digit) is pending; show the popup after 300 ms.
    Pending,
    Fire { action: Action, count: u32 },
    Unbound,
}

pub const PREFIX_POPUP_AFTER: Duration = Duration::from_millis(300);

impl KeyState {
    /// Pure. Digits accumulate a count unless a sequence is pending ("0" alone is unbound).
    /// pending + key: exact binding -> Fire (reset); proper prefix -> Pending; else Unbound (reset).
    pub fn feed(&mut self, map: &Keymap, key: Key, now: Instant) -> Fed {
        todo!()
    }
    pub fn pending(&self) -> Option<(&[Key], Instant)> {
        self.since.map(|t| (self.pending.as_slice(), t))
    }
}

pub fn on_event(ui: &mut Ui, ev: Event, tx: &Sender<Msg>, now: Instant) {
    match ev {
        Event::Key(k) => on_key(ui, k, tx, now),
        Event::Mouse(m) => on_mouse(ui, m),
        Event::Resize(..) => ui.repaint = crate::tui::ArtRepaint::Wipe,
        _ => {}
    }
}

fn on_key(ui: &mut Ui, k: KeyEvent, tx: &Sender<Msg>, now: Instant) {
    // TODO:
    //   Mode::Help -> any key closes it
    //   Mode::Search{draft} -> Esc cancels, Enter: ui.browse.submit(draft, tx), Backspace, Char push
    //   Mode::Normal -> match ui.keys.feed(&ui.keymap, normalize(k), now) {
    //       Fire{action, count} -> for _ in 0..count.min(99) { dispatch(ui, action, tx) }   // count only for motions/seek
    //       Pending | Unbound -> {}
    //   }
    let _ = (ui, k, tx, now);
    todo!()
}

/// The one place actions become effects.
pub fn dispatch(ui: &mut Ui, action: Action, tx: &Sender<Msg>) {
    let p = &ui.player;
    match action {
        Action::Next => p.send(Command::Next),
        Action::Prev => p.send(Command::Prev),
        Action::TogglePause => p.send(Command::TogglePause),
        Action::SeekBack => p.send(Command::Seek(SeekTo::By(-5))),
        Action::SeekForward => p.send(Command::Seek(SeekTo::By(5))),
        Action::VolumeDown => p.send(Command::Volume(VolumeTo::By(-5))),
        Action::VolumeUp => p.send(Command::Volume(VolumeTo::By(5))),
        Action::Activate => {
            // The selection is resolved to a value *now* and sent by value; nothing
            // downstream holds an index into a list the TUI shows.
            // TODO: match ui.focus / ui.browse.selected():
            //   Some(Playable::Track(t))   -> Command::PlayTrack(t)
            //   Some(Playable::Station(s)) -> Command::PlayStations { stations: <stations of the visible list>, start: s.id }
            //   queue view row             -> Command::Jump(row.id)
            let _: Option<Playable> = None;
        }
        Action::PlayNext => {
            // TODO: if let Some(item) = ui.browse.selected() { p.send(Command::PlayNext(item)) }
        }
        Action::ToggleFavorite => {
            // TODO: on = !ui.snap.favorites.contains(&item.key()); p.send(SetFavorite{item, on})
        }
        Action::Search => ui.mode = Mode::Search { draft: String::new() },
        Action::Help => ui.mode = Mode::Help,
        Action::Quit => ui.quit = true,
        Action::CycleFocus => ui.focus = if ui.focus == Focus::Main { Focus::Sidebar } else { Focus::Main },
        // TODO: motions via ui.browse.move_by(delta, ui.last_frame.list_rows), tabs, layout, theme
        //       (ui.look.cycle(now)), spectrum toggle, go-queue.
        _ => {
            let _ = tx;
            todo!()
        }
    }
}

fn on_mouse(ui: &mut Ui, m: MouseEvent) {
    // TODO: hit-test against ui.last_frame.hits: row click selects (double click activates),
    //       wheel scrolls, progress-bar click -> Seek(At(frac * duration)) for tracks.
    let _ = (ui, m);
    todo!()
}

#[cfg(test)]
mod tests {
    // feed_gg_fires_top_after_pending
    // feed_5j_fires_down_with_count_5
    // feed_unbound_after_prefix_resets_state
    // override_unbind_removes_default_and_its_prefix
}
