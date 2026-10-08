//! Keys to `Action` to (view change | player `Command`).
//!
//! The keymap is data: a table from key sequence to `Action`, defaults below,
//! overrides from keymap.toml. `KeyState::feed` is a pure prefix/count machine.
//! `dispatch` is the one match that turns actions into effects.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::domain::Playable;
use crate::player::api::{Command, Level, Notice, SeekTo, VolumeTo};
use crate::tui::browse::{Selected, Tab};
use crate::tui::{Mode, Ui};

/// Generates `Action`, its kebab-case names (the keymap.toml spelling), and
/// the help text, from one table so they cannot drift apart.
macro_rules! actions {
    ($($variant:ident $name:literal $help:literal,)*) => {
        /// Everything a key can mean.
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub enum Action { $($variant,)* }

        impl Action {
            pub const ALL: &[Action] = &[$(Action::$variant,)*];
            pub fn name(self) -> &'static str {
                match self { $(Action::$variant => $name,)* }
            }
            pub fn help(self) -> &'static str {
                match self { $(Action::$variant => $help,)* }
            }
        }
    };
}

actions! {
    Down "down" "move down",
    Up "up" "move up",
    Top "top" "first row",
    Bottom "bottom" "last row",
    HalfDown "half-down" "half page down",
    HalfUp "half-up" "half page up",
    Activate "activate" "play the selected row",
    TogglePause "toggle-pause" "play / pause",
    Next "next" "next in queue",
    Prev "prev" "restart, or previous in queue",
    SeekBack "seek-back" "seek -5 s (tracks)",
    SeekForward "seek-forward" "seek +5 s (tracks)",
    VolumeDown "volume-down" "volume -5",
    VolumeUp "volume-up" "volume +5",
    PlayNext "play-next" "play the selection next",
    ToggleFavorite "favorite" "favorite / unfavorite",
    Remove "remove" "remove from the queue",
    Search "search" "search (Enter runs, Esc cancels)",
    TabYouTube "tab-youtube" "YouTube",
    TabRadio "tab-radio" "Radio",
    TabFavorites "tab-favorites" "Favorites",
    CycleTab "cycle-tab" "next source",
    GoQueue "go-queue" "queue view",
    CycleLayout "cycle-layout" "cycle layout",
    CycleTheme "cycle-theme" "cycle theme",
    ToggleSpectrum "toggle-spectrum" "spectrum on / off",
    Help "help" "this help",
    Quit "quit" "stop and quit",
    Unbind "unbind" "remove a default binding",
}

impl FromStr for Action {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Action::ALL.iter().copied().find(|a| a.name() == s).ok_or_else(|| format!("unknown action '{s}'"))
    }
}

/// A key with modifiers, normalized: a shifted letter is its uppercase char
/// with no SHIFT bit.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Key {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl Key {
    pub fn from_event(k: &KeyEvent) -> Key {
        let mut mods = k.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        if matches!(k.code, KeyCode::Char(_)) {
            mods.remove(KeyModifiers::SHIFT);
        }
        Key { code: k.code, mods }
    }

    fn label(&self) -> String {
        let base = match self.code {
            KeyCode::Char(' ') => "space".into(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "enter".into(),
            KeyCode::Tab => "tab".into(),
            KeyCode::Esc => "esc".into(),
            KeyCode::Up => "up".into(),
            KeyCode::Down => "down".into(),
            KeyCode::Left => "left".into(),
            KeyCode::Right => "right".into(),
            other => format!("{other:?}").to_lowercase(),
        };
        if self.mods.contains(KeyModifiers::CONTROL) { format!("C-{base}") } else { base }
    }
}

/// "g g", "gg", "C-d", "space", "enter". Tokens are space-separated; a token of
/// several plain characters is a sequence of those characters.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct KeySeq(pub Vec<Key>);

impl KeySeq {
    pub fn parse(s: &str) -> Result<KeySeq, String> {
        let mut keys = Vec::new();
        for tok in s.split_whitespace() {
            let (mods, name) = match tok.strip_prefix("C-") {
                Some(rest) if !rest.is_empty() => (KeyModifiers::CONTROL, rest),
                _ => (KeyModifiers::NONE, tok),
            };
            let named = match name {
                "space" => Some(KeyCode::Char(' ')),
                "enter" => Some(KeyCode::Enter),
                "tab" => Some(KeyCode::Tab),
                "esc" => Some(KeyCode::Esc),
                "up" => Some(KeyCode::Up),
                "down" => Some(KeyCode::Down),
                "left" => Some(KeyCode::Left),
                "right" => Some(KeyCode::Right),
                _ => None,
            };
            match named {
                Some(code) => keys.push(Key { code, mods }),
                None if mods.is_empty() => keys.extend(name.chars().map(|c| Key { code: KeyCode::Char(c), mods })),
                None if name.chars().count() == 1 => {
                    keys.extend(name.chars().map(|c| Key { code: KeyCode::Char(c.to_ascii_lowercase()), mods }))
                }
                None => return Err(format!("bad key '{tok}'")),
            }
        }
        if keys.is_empty() { Err("empty key sequence".into()) } else { Ok(KeySeq(keys)) }
    }

    pub fn label(&self) -> String {
        let labels: Vec<String> = self.0.iter().map(Key::label).collect();
        if self.0.iter().all(|k| matches!(k.code, KeyCode::Char(c) if c != ' ') && k.mods.is_empty()) {
            labels.concat()
        } else {
            labels.join(" ")
        }
    }
}

const DEFAULTS: &[(&str, Action)] = &[
    ("j", Action::Down),
    ("down", Action::Down),
    ("k", Action::Up),
    ("up", Action::Up),
    ("gg", Action::Top),
    ("G", Action::Bottom),
    ("C-d", Action::HalfDown),
    ("C-u", Action::HalfUp),
    ("enter", Action::Activate),
    ("space", Action::TogglePause),
    ("n", Action::Next),
    ("b", Action::Prev),
    ("H", Action::SeekBack),
    ("L", Action::SeekForward),
    ("-", Action::VolumeDown),
    ("+", Action::VolumeUp),
    ("=", Action::VolumeUp),
    ("a", Action::PlayNext),
    ("f", Action::ToggleFavorite),
    ("dd", Action::Remove),
    ("/", Action::Search),
    ("1", Action::TabYouTube),
    ("2", Action::TabRadio),
    ("3", Action::TabFavorites),
    ("tab", Action::CycleTab),
    ("gq", Action::GoQueue),
    ("w", Action::CycleLayout),
    ("t", Action::CycleTheme),
    ("v", Action::ToggleSpectrum),
    ("?", Action::Help),
    ("q", Action::Quit),
    ("C-c", Action::Quit),
];

pub struct Keymap {
    bindings: HashMap<KeySeq, Action>,
    /// Every proper prefix of a binding; derived from `bindings`.
    prefixes: HashSet<Vec<Key>>,
}

impl Keymap {
    pub fn with_overrides(overrides: &[(KeySeq, Action)]) -> Keymap {
        let mut bindings: HashMap<KeySeq, Action> =
            DEFAULTS.iter().filter_map(|(s, a)| KeySeq::parse(s).ok().map(|k| (k, *a))).collect();
        for (seq, action) in overrides {
            if *action == Action::Unbind {
                bindings.remove(seq);
            } else {
                bindings.insert(seq.clone(), *action);
            }
        }
        let prefixes = bindings.keys().flat_map(|s| (1..s.0.len()).map(|n| s.0[..n].to_vec())).collect();
        Keymap { bindings, prefixes }
    }

    /// For the help overlay and the prefix popup, sorted by action order.
    pub fn bindings_starting_with(&self, prefix: &[Key]) -> Vec<(String, Action)> {
        let mut v: Vec<(String, Action)> =
            self.bindings.iter().filter(|(k, _)| k.0.starts_with(prefix)).map(|(k, a)| (k.label(), *a)).collect();
        v.sort_by_key(|(k, a)| (Action::ALL.iter().position(|x| x == a), k.clone()));
        v
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
    /// A prefix (or count digit) is pending.
    Pending,
    Fire {
        action: Action,
        count: u32,
    },
    Unbound,
}

pub const PREFIX_POPUP_AFTER: Duration = Duration::from_millis(300);

impl KeyState {
    /// Pure. A bound digit fires its binding; unbound digits build a count
    /// ("5j"). Pending plus key: an exact binding fires, a proper prefix waits,
    /// anything else is unbound and resets.
    pub fn feed(&mut self, map: &Keymap, key: Key, now: Instant) -> Fed {
        let mut seq = std::mem::take(&mut self.pending);
        seq.push(key);
        if let Some(&action) = map.bindings.get(&KeySeq(seq.clone())) {
            let count = self.count.take().unwrap_or(1);
            self.since = None;
            return Fed::Fire { action, count };
        }
        if map.prefixes.contains(&seq) {
            self.pending = seq;
            self.since = Some(now);
            return Fed::Pending;
        }
        if seq.len() == 1
            && let KeyCode::Char(c @ '0'..='9') = key.code
            && key.mods.is_empty()
            && (c != '0' || self.count.is_some())
        {
            let d = c.to_digit(10).unwrap_or(0);
            self.count = Some(self.count.unwrap_or(0).saturating_mul(10).saturating_add(d).min(999));
            self.since = Some(now);
            return Fed::Pending;
        }
        self.count = None;
        self.since = None;
        Fed::Unbound
    }

    /// The pending prefix and when it started, for the completion popup.
    pub fn pending(&self) -> Option<(&[Key], Instant)> {
        self.since.filter(|_| !self.pending.is_empty()).map(|t| (self.pending.as_slice(), t))
    }

    pub fn reset(&mut self) {
        *self = KeyState::default();
    }
}

pub fn on_event(ui: &mut Ui, ev: Event, now: Instant) {
    match ev {
        Event::Key(k) if k.kind == KeyEventKind::Press => on_key(ui, k, now),
        Event::Resize(..) => ui.force_art = true,
        _ => {}
    }
}

fn on_key(ui: &mut Ui, k: KeyEvent, now: Instant) {
    match &mut ui.mode {
        Mode::Help => {
            ui.mode = Mode::Normal;
            ui.force_art = true;
        }
        Mode::Search { draft } => match k.code {
            KeyCode::Esc => ui.mode = Mode::Normal,
            KeyCode::Enter => {
                let q = std::mem::take(draft);
                ui.mode = Mode::Normal;
                ui.browse.submit(q, &ui.tx, &ui.http);
            }
            KeyCode::Backspace => {
                draft.pop();
            }
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => draft.push(c),
            _ => {}
        },
        Mode::Normal => {
            if let Fed::Fire { action, count } = ui.keys.feed(&ui.keymap, Key::from_event(&k), now) {
                let repeat = if matches!(action, Action::Down | Action::Up | Action::SeekBack | Action::SeekForward) {
                    count
                } else {
                    1
                };
                for _ in 0..repeat {
                    dispatch(ui, action, now);
                }
            }
        }
    }
}

fn toast(ui: &mut Ui, text: impl Into<String>, now: Instant) {
    ui.toasts.push(Notice { level: Level::Info, text: text.into() }, now);
}

/// The one place actions become effects.
pub fn dispatch(ui: &mut Ui, action: Action, now: Instant) {
    let rows = ui.last_frame.list_rows.max(2);
    let p = &ui.player;
    match action {
        Action::Next => p.send(Command::Next),
        Action::Prev => p.send(Command::Prev),
        Action::TogglePause => p.send(Command::TogglePause),
        Action::SeekBack => p.send(Command::Seek(SeekTo::By(-5))),
        Action::SeekForward => p.send(Command::Seek(SeekTo::By(5))),
        Action::VolumeDown => p.send(Command::Volume(VolumeTo::By(-5))),
        Action::VolumeUp => p.send(Command::Volume(VolumeTo::By(5))),
        Action::Activate => match ui.browse.selected(&ui.snap) {
            // The selection travels by value; nothing downstream holds an index.
            Some(Selected::Item(Playable::Track(t))) => p.send(Command::PlayTrack(t)),
            Some(Selected::Item(Playable::Station(s))) => {
                p.send(Command::PlayStations { stations: ui.browse.stations(&ui.snap), start: s.id.clone() })
            }
            Some(Selected::Entry(id, _)) => p.send(Command::Jump(id)),
            None => {}
        },
        Action::PlayNext => {
            if let Some(Selected::Item(item)) = ui.browse.selected(&ui.snap) {
                p.send(Command::PlayNext(item));
            }
        }
        Action::ToggleFavorite => {
            if let Some(Selected::Item(item) | Selected::Entry(_, item)) = ui.browse.selected(&ui.snap) {
                let on = !ui.snap.favorites.contains(&item.key());
                let text = format!("{} {}", if on { "♥ favorited" } else { "unfavorited" }, item.title());
                p.send(Command::SetFavorite { item, on });
                toast(ui, text, now);
            }
        }
        Action::Remove => match ui.browse.selected(&ui.snap) {
            Some(Selected::Entry(id, _)) => p.send(Command::Remove(id)),
            _ => toast(ui, "dd removes from the queue; gq opens it", now),
        },
        Action::Down => ui.browse.move_by(1, &ui.snap),
        Action::Up => ui.browse.move_by(-1, &ui.snap),
        Action::HalfDown => ui.browse.move_by(rows as isize / 2, &ui.snap),
        Action::HalfUp => ui.browse.move_by(-(rows as isize / 2), &ui.snap),
        Action::Top => ui.browse.move_by(isize::MIN / 2, &ui.snap),
        Action::Bottom => ui.browse.move_by(isize::MAX / 2, &ui.snap),
        Action::Search => {
            ui.browse.to_search_tab();
            ui.mode = Mode::Search { draft: String::new() };
        }
        Action::TabYouTube => ui.browse.set_tab(Tab::YouTube, &ui.snap),
        Action::TabRadio => ui.browse.set_tab(Tab::Radio, &ui.snap),
        Action::TabFavorites => ui.browse.set_tab(Tab::Favorites, &ui.snap),
        Action::CycleTab => ui.browse.cycle_tab(&ui.snap),
        Action::GoQueue => ui.browse.toggle_queue(&ui.snap),
        Action::CycleLayout => {
            ui.layout = ui.layout.next();
            ui.force_art = true;
        }
        Action::CycleTheme => {
            ui.look.cycle();
            toast(ui, format!("theme: {}", ui.look.mode.label()), now);
        }
        Action::ToggleSpectrum => ui.spectrum.enabled = !ui.spectrum.enabled,
        Action::Help => ui.mode = Mode::Help,
        Action::Quit => ui.quit = true,
        Action::Unbind => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> Key {
        Key { code: KeyCode::Char(c), mods: KeyModifiers::NONE }
    }

    #[test]
    fn gg_fires_top_after_a_pending_g() {
        let map = Keymap::with_overrides(&[]);
        let mut ks = KeyState::default();
        let t = Instant::now();
        assert_eq!(ks.feed(&map, key('g'), t), Fed::Pending);
        assert_eq!(ks.feed(&map, key('g'), t), Fed::Fire { action: Action::Top, count: 1 });
        assert_eq!(ks.feed(&map, key('g'), t), Fed::Pending);
        assert_eq!(ks.feed(&map, key('q'), t), Fed::Fire { action: Action::GoQueue, count: 1 });
    }

    #[test]
    fn an_unbound_digit_builds_a_count_and_a_bound_one_fires() {
        let map = Keymap::with_overrides(&[]);
        let mut ks = KeyState::default();
        let t = Instant::now();
        assert_eq!(ks.feed(&map, key('5'), t), Fed::Pending);
        assert_eq!(ks.feed(&map, key('j'), t), Fed::Fire { action: Action::Down, count: 5 });
        assert_eq!(ks.feed(&map, key('2'), t), Fed::Fire { action: Action::TabRadio, count: 1 });
    }

    #[test]
    fn an_unbound_key_after_a_prefix_resets() {
        let map = Keymap::with_overrides(&[]);
        let mut ks = KeyState::default();
        let t = Instant::now();
        ks.feed(&map, key('d'), t);
        assert_eq!(ks.feed(&map, key('z'), t), Fed::Unbound);
        assert_eq!(ks.feed(&map, key('j'), t), Fed::Fire { action: Action::Down, count: 1 });
    }

    #[test]
    fn overrides_rebind_and_unbind_removes_the_default_and_its_prefix() {
        let gg = KeySeq::parse("g g").expect("seq");
        let map = Keymap::with_overrides(&[(gg, Action::Unbind), (KeySeq::parse("x").expect("seq"), Action::Quit)]);
        let mut ks = KeyState::default();
        let t = Instant::now();
        assert_eq!(ks.feed(&map, key('x'), t), Fed::Fire { action: Action::Quit, count: 1 });
        assert_eq!(ks.feed(&map, key('g'), t), Fed::Pending, "gq still uses the g prefix");
        assert_eq!(ks.feed(&map, key('g'), t), Fed::Unbound);
    }

    #[test]
    fn sequences_parse_from_both_spellings() {
        assert_eq!(KeySeq::parse("gg"), KeySeq::parse("g g"));
        let cd = KeySeq::parse("C-d").expect("seq");
        assert_eq!(cd.0, vec![Key { code: KeyCode::Char('d'), mods: KeyModifiers::CONTROL }]);
        assert_eq!(cd.label(), "C-d");
        assert!(KeySeq::parse("C-foo").is_err());
        assert!("not-an-action".parse::<Action>().is_err());
        assert_eq!("go-queue".parse::<Action>(), Ok(Action::GoQueue));
    }
}
