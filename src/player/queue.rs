//! The play queue. Pure. Owns the cursor and the autoplay-radio bookkeeping so
//! no caller ever touches an index.
//!
//! Invariants (enforced by keeping fields private):
//! - `cursor` is `None` or `< entries.len()`.
//! - `EntryId`s are unique for the life of the process and never reused, so a
//!   client holding one can only ever hit the entry it meant or nothing.
//! - Every `replace` resets the radio fetch, which makes a late refill for an
//!   old queue inert.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::domain::{Playable, RefillId, Track, VideoId};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct EntryId(u64);

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
    /// Autoplay is on. `seed` is the track the radio grew from; refills seed
    /// from the current track, because re-asking for `RDAMVM<seed>` returns the
    /// same list and dedupe would drop all of it.
    On { seed: VideoId, fetch: Fetch },
}

/// The radio refill slot.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum Fetch {
    #[default]
    Idle,
    InFlight(RefillId),
    /// A refill seeded by `at` added nothing (or failed). No refill runs until
    /// the current track is a different one, so an empty answer cannot loop.
    Dry { at: VideoId },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Removed {
    NotFound,
    Other,
    /// The current entry left the queue. The cursor now points at what followed
    /// it (or nothing); playback decides whether to start that.
    Current,
}

impl Queue {
    pub fn current(&self) -> Option<&Entry> {
        self.cursor.map(|i| &self.entries[i])
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

    pub fn replace(&mut self, items: Vec<Playable>, start: usize, radio_seed: Option<VideoId>) {
        let _ = (items, start, radio_seed);
        todo!()
    }

    pub fn insert_next(&mut self, item: Playable) -> EntryId {
        let _ = item;
        todo!()
    }

    pub fn refill_landed(&mut self, ticket: RefillId, tracks: Option<Vec<Arc<Track>>>) -> usize {
        let _ = (ticket, tracks);
        todo!()
    }
}
