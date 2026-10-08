//! The play queue. Pure. Owns the cursor and the autoplay-radio bookkeeping so
//! no caller ever touches an index.
//!
//! Invariants (enforced by keeping fields private):
//! - `cursor` is `None` or `< entries.len()`.
//! - `EntryId`s are unique for the life of the process and never reused, so a
//!   client holding one can only ever hit the entry it meant or nothing.

use std::collections::HashSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::domain::{ItemKey, Playable, RefillId, Track, VideoId};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct EntryId(u64);

impl EntryId {
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Entry {
    pub id: EntryId,
    pub item: Playable,
    pub from_radio: bool,
}

/// Refill when this many or fewer entries remain after the cursor.
pub const REFILL_AT: usize = 3;

#[derive(Debug, Default)]
pub struct Queue {
    entries: Vec<Entry>,
    cursor: Option<usize>,
    next_id: u64,
    radio: Radio,
}

#[derive(Debug, Default)]
enum Radio {
    #[default]
    Off,
    On {
        seed: VideoId,
        fetch: Fetch,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum Fetch {
    #[default]
    Idle,
    InFlight {
        ticket: RefillId,
        seed: VideoId,
    },
    Dry {
        at: VideoId,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Removed {
    NotFound,
    Other,
    Current,
}

impl Queue {
    pub fn replace(&mut self, items: Vec<Playable>, start: usize, radio_seed: Option<VideoId>) {
        let entries = items.into_iter().map(|item| self.entry(item, false)).collect();
        self.entries = entries;
        self.cursor = (!self.entries.is_empty()).then(|| start.min(self.entries.len() - 1));
        self.radio = match radio_seed {
            Some(seed) => Radio::On { seed, fetch: Fetch::Idle },
            None => Radio::Off,
        };
    }

    pub fn restore(&mut self, items: Vec<Playable>, cursor: Option<usize>, radio_seed: Option<VideoId>) {
        self.replace(items, cursor.unwrap_or(0), radio_seed);
        if cursor.is_none_or(|c| c >= self.entries.len()) {
            self.cursor = None;
        }
    }

    fn entry(&mut self, item: Playable, from_radio: bool) -> Entry {
        self.next_id += 1;
        Entry { id: EntryId(self.next_id), item, from_radio }
    }

    pub fn insert_next(&mut self, item: Playable) -> EntryId {
        let at = self.cursor.map_or(self.entries.len(), |c| c + 1);
        let e = self.entry(item, false);
        let id = e.id;
        self.entries.insert(at, e);
        id
    }

    pub fn current(&self) -> Option<&Entry> {
        self.cursor.map(|i| &self.entries[i])
    }

    /// Move the cursor one step. `None` at either end; the cursor does not move then.
    pub fn advance(&mut self) -> Option<&Entry> {
        let next = self.cursor.map_or(0, |c| c + 1);
        if next >= self.entries.len() {
            return None;
        }
        self.cursor = Some(next);
        self.current()
    }

    pub fn retreat(&mut self) -> Option<&Entry> {
        let c = self.cursor.filter(|&c| c > 0)?;
        self.cursor = Some(c - 1);
        self.current()
    }

    pub fn jump(&mut self, id: EntryId) -> Option<&Entry> {
        let i = self.entries.iter().position(|e| e.id == id)?;
        self.cursor = Some(i);
        self.current()
    }

    pub fn remove(&mut self, id: EntryId) -> Removed {
        let Some(i) = self.entries.iter().position(|e| e.id == id) else {
            return Removed::NotFound;
        };
        self.entries.remove(i);
        match self.cursor {
            Some(c) if i < c => {
                self.cursor = Some(c - 1);
                Removed::Other
            }
            Some(c) if i == c => {
                self.cursor = (i < self.entries.len()).then_some(i);
                Removed::Current
            }
            _ => Removed::Other,
        }
    }

    /// Entries after the cursor, in play order.
    pub fn upcoming(&self) -> impl Iterator<Item = &Entry> {
        let from = self.cursor.map_or(0, |c| c + 1);
        self.entries[from.min(self.entries.len())..].iter()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn cursor_index(&self) -> Option<usize> {
        self.cursor
    }

    pub fn radio_seed(&self) -> Option<&VideoId> {
        match &self.radio {
            Radio::On { seed, .. } => Some(seed),
            Radio::Off => None,
        }
    }

    pub fn fetch(&self) -> Option<&Fetch> {
        match &self.radio {
            Radio::On { fetch, .. } => Some(fetch),
            Radio::Off => None,
        }
    }

    pub fn refill_wanted(&self) -> Option<VideoId> {
        let Radio::On { fetch, .. } = &self.radio else { return None };
        let Some(Playable::Track(t)) = self.current().map(|e| &e.item) else { return None };
        let free = match fetch {
            Fetch::Idle => true,
            Fetch::InFlight { .. } => false,
            Fetch::Dry { at } => *at != t.id,
        };
        (free && self.upcoming().count() <= REFILL_AT).then(|| t.id.clone())
    }

    pub fn refill_started(&mut self, ticket: RefillId, seed: VideoId) {
        if let Radio::On { fetch, .. } = &mut self.radio {
            *fetch = Fetch::InFlight { ticket, seed };
        }
    }

    pub fn refill_landed(&mut self, ticket: RefillId, tracks: Option<Vec<Arc<Track>>>) -> Option<usize> {
        let Radio::On { fetch, .. } = &mut self.radio else { return None };
        let Fetch::InFlight { ticket: t, seed } = fetch else { return None };
        if *t != ticket {
            return None;
        }
        let seed = seed.clone();
        let mut have: HashSet<ItemKey> = self.entries.iter().map(|e| e.item.key()).collect();
        let fresh: Vec<Playable> =
            tracks.unwrap_or_default().into_iter().map(Playable::Track).filter(|p| have.insert(p.key())).collect();
        let added = fresh.len();
        *fetch = if added == 0 { Fetch::Dry { at: seed } } else { Fetch::Idle };
        for item in fresh {
            let e = self.entry(item, true);
            self.entries.push(e);
        }
        Some(added)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::domain::{Station, StationId, Tickets};

    pub fn track(id: &str) -> Arc<Track> {
        Arc::new(Track {
            id: VideoId::parse(id).expect("test video id"),
            title: format!("title {id}"),
            artists: vec![],
            album: None,
            duration: Some(std::time::Duration::from_secs(200)),
            art: None,
        })
    }

    pub fn station(n: u8) -> Arc<Station> {
        Arc::new(Station {
            id: StationId::parse(&format!("00000000-0000-0000-0000-0000000000{n:02x}")).expect("test uuid"),
            name: format!("station {n}"),
            stream: url::Url::parse(&format!("http://radio.test/{n}")).expect("test url"),
            homepage: None,
            favicon: None,
            country_code: None,
            tags: vec![],
            codec: None,
            bitrate_kbps: None,
        })
    }

    pub fn vid(c: char) -> String {
        std::iter::repeat_n(c, 11).collect()
    }

    fn t(c: char) -> Playable {
        Playable::Track(track(&vid(c)))
    }

    fn titles(q: &Queue) -> String {
        q.entries().iter().map(|e| e.item.title().chars().last().unwrap_or('?')).collect()
    }

    #[test]
    fn replace_clamps_start_and_a_refill_for_the_old_queue_is_dropped() {
        let mut tickets = Tickets::default();
        let mut q = Queue::default();
        q.replace(vec![t('a')], 0, Some(VideoId::parse(&vid('a')).expect("id")));
        let seed = q.refill_wanted().expect("refill due on a fresh radio queue");
        let old = tickets.issue();
        q.refill_started(old, seed);
        q.replace(vec![t('b'), t('c')], 9, Some(VideoId::parse(&vid('b')).expect("id")));
        assert_eq!(q.cursor_index(), Some(1));
        assert_eq!(q.refill_landed(old, Some(vec![track(&vid('z'))])), None);
        assert_eq!(titles(&q), "bc");
    }

    #[test]
    fn insert_next_lands_right_after_the_cursor() {
        let mut q = Queue::default();
        q.replace(vec![t('a'), t('b'), t('c')], 1, None);
        let id = q.insert_next(t('x'));
        assert_eq!(titles(&q), "abxc");
        assert_eq!(q.advance().map(|e| e.id), Some(id));
    }

    #[test]
    fn removing_before_the_cursor_keeps_the_current_entry_current() {
        let mut q = Queue::default();
        q.replace(vec![t('a'), t('b'), t('c')], 2, None);
        let a = q.entries()[0].id;
        assert_eq!(q.remove(a), Removed::Other);
        assert_eq!(q.current().map(|e| e.item.title().to_string()), Some(format!("title {}", vid('c'))));
    }

    #[test]
    fn removing_the_current_entry_moves_the_cursor_to_its_follower() {
        let mut q = Queue::default();
        q.replace(vec![t('a'), t('b'), t('c')], 1, None);
        let b = q.entries()[1].id;
        assert_eq!(q.remove(b), Removed::Current);
        assert_eq!(q.current().map(|e| e.item.key()), Some(t('c').key()));
        let c = q.entries()[1].id;
        assert_eq!(q.remove(c), Removed::Current);
        assert_eq!(q.current(), None);
    }

    #[test]
    fn advance_at_the_end_returns_none_and_keeps_the_cursor() {
        let mut q = Queue::default();
        q.replace(vec![t('a'), t('b')], 1, None);
        assert!(q.advance().is_none());
        assert_eq!(q.cursor_index(), Some(1));
    }

    #[test]
    fn refill_wanted_only_at_three_or_fewer_upcoming() {
        let mut q = Queue::default();
        let seed = VideoId::parse(&vid('a')).expect("id");
        q.replace(vec![t('a'), t('b'), t('c'), t('d'), t('e')], 0, Some(seed));
        assert_eq!(q.refill_wanted(), None, "4 upcoming");
        q.advance();
        assert_eq!(q.refill_wanted(), VideoId::parse(&vid('b')), "3 upcoming, seeded by the current track");
    }

    #[test]
    fn refill_dedupes_against_the_whole_queue_and_an_empty_one_goes_dry() {
        let mut tickets = Tickets::default();
        let mut q = Queue::default();
        q.replace(vec![t('a'), t('b')], 1, VideoId::parse(&vid('a')));
        let seed = q.refill_wanted().expect("due");
        let tk = tickets.issue();
        q.refill_started(tk, seed.clone());
        assert_eq!(q.refill_landed(tk, Some(vec![track(&vid('a')), track(&vid('c'))])), Some(1));
        assert_eq!(titles(&q), "abc");
        assert!(q.entries()[2].from_radio);

        let seed = q.refill_wanted().expect("still few upcoming");
        let tk = tickets.issue();
        q.refill_started(tk, seed.clone());
        assert_eq!(q.refill_landed(tk, Some(vec![track(&vid('c'))])), Some(0));
        assert_eq!(q.fetch(), Some(&Fetch::Dry { at: seed }));
        assert_eq!(q.refill_wanted(), None, "dry for this track");
        q.advance();
        assert!(q.refill_wanted().is_some(), "a new current track may refill again");
    }

    #[test]
    fn entry_ids_survive_inserts_and_removals() {
        let mut q = Queue::default();
        q.replace(vec![t('a'), t('b')], 0, None);
        let b = q.entries()[1].id;
        q.insert_next(t('x'));
        let a = q.entries()[0].id;
        q.remove(a);
        assert_eq!(q.jump(b).map(|e| e.item.key()), Some(t('b').key()));
        assert!(q.jump(a).is_none());
    }

    #[test]
    fn restore_drops_an_out_of_range_cursor() {
        let mut q = Queue::default();
        q.restore(vec![t('a')], Some(4), None);
        assert_eq!(q.cursor_index(), None);
        q.restore(vec![t('a'), t('b')], Some(1), None);
        assert_eq!(q.cursor_index(), Some(1));
    }
}
