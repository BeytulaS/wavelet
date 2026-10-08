//! The play queue. Pure. Owns the cursor and the autoplay-radio bookkeeping so
//! no caller ever touches an index.
//!
//! Invariants (enforced by keeping fields private):
//! - `cursor` is `None` or `< entries.len()`.
//! - `EntryId`s are unique for the life of the process and never reused, so a
//!   client holding one can only ever hit the entry it meant or nothing.
//! - `radio.pending` is cleared by every `replace`, which is what makes a late
//!   refill for an old queue inert.

use std::collections::HashSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::domain::{ItemKey, Playable, RefillId, Track, VideoId};

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
    /// Autoplay is on. `seed` is the track the radio grew from; refills seed from
    /// the current track instead, because re-asking for `RDAMVM<seed>` returns the
    /// same list and dedupe would drop all of it.
    On { seed: VideoId, pending: Option<RefillId> },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Removed {
    NotFound,
    Other,
    /// The current entry left the queue. The cursor now points at what followed it
    /// (or nothing); playback decides whether to start that.
    Current,
}

/// What the core should ask for, derived from queue shape.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RefillWanted {
    pub seed: VideoId,
}

impl Queue {
    /// Replace everything. `radio_seed` turns autoplay on (YouTube Enter) or off (stations).
    /// Returns the entry at `start` (clamped), now current.
    pub fn replace(&mut self, items: Vec<Playable>, start: usize, radio_seed: Option<VideoId>) -> Option<&Entry> {
        // TODO: assign fresh EntryIds; cursor = Some(min(start, len-1)) or None if empty;
        //       radio = On { seed, pending: None } or Off.
        todo!()
    }

    /// Insert right after the cursor (or at the end when there is no cursor). Returns its id.
    pub fn insert_next(&mut self, item: Playable) -> EntryId {
        todo!()
    }

    pub fn current(&self) -> Option<&Entry> {
        self.cursor.map(|i| &self.entries[i])
    }

    pub fn get(&self, id: EntryId) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Move the cursor one step. `None` at either end; the cursor does not move then.
    pub fn advance(&mut self) -> Option<&Entry> {
        todo!()
    }
    pub fn retreat(&mut self) -> Option<&Entry> {
        todo!()
    }
    pub fn jump(&mut self, id: EntryId) -> Option<&Entry> {
        todo!()
    }

    pub fn remove(&mut self, id: EntryId) -> Removed {
        // TODO: find index i.
        //   i < cursor  -> cursor -= 1, Other
        //   i == cursor -> cursor stays at i if i < new len (the follower), else None; Current
        //   i > cursor  -> Other
        todo!()
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

    /// Pure check, no mutation: is a refill due and not already in flight?
    pub fn refill_wanted(&self) -> Option<RefillWanted> {
        // TODO:
        //   Radio::On { pending: None, .. }
        //   && upcoming().count() <= REFILL_AT
        //   && current() is a Track (seed from it; if the current entry is a station, never)
        //   -> Some(RefillWanted { seed: current track id })
        todo!()
    }

    /// Record that a refill request went out under `ticket`.
    pub fn refill_started(&mut self, ticket: RefillId) {
        // TODO: Radio::On { pending } = Some(ticket); no-op when Off.
        todo!()
    }

    /// Apply a refill result. Ignored (returns 0) unless `ticket` is the pending one,
    /// which is false after any `replace`. Dedupes against every key already queued.
    /// Clears `pending` either way when the ticket matches, so a failed fetch can retry
    /// on the next reconcile.
    pub fn refill_landed(&mut self, ticket: RefillId, tracks: Option<Vec<Arc<Track>>>) -> usize {
        // TODO:
        //   let Radio::On { pending: Some(p), .. } = &mut self.radio else { return 0 };
        //   if *p != ticket { return 0 }
        //   *p = None-equivalent
        //   let have: HashSet<ItemKey> = entries keys
        //   push each unseen track with from_radio = true; return count
        //   (Failure path: tracks == None, just clear pending. A backoff lives in core if needed.)
        let _: HashSet<ItemKey>;
        todo!()
    }

    pub fn radio_seed(&self) -> Option<&VideoId> {
        match &self.radio {
            Radio::On { seed, .. } => Some(seed),
            Radio::Off => None,
        }
    }
    pub fn refill_pending(&self) -> bool {
        matches!(self.radio, Radio::On { pending: Some(_), .. })
    }

    /// Restore from session.json. Same as `replace` but does not start playback.
    pub fn restore(&mut self, items: Vec<Playable>, cursor: Option<usize>, radio_seed: Option<VideoId>) {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    // Each test names the defect it catches.
    //
    // replace_clamps_start_and_resets_radio_pending
    //   catches: a refill from the previous queue appending into the new one.
    // insert_next_lands_after_cursor_not_after_selection
    //   catches: the prototype's "a" inserting relative to the results list.
    // remove_before_cursor_keeps_current_entry_current
    // remove_current_moves_cursor_to_follower_and_reports_current
    // advance_at_end_returns_none_and_keeps_cursor
    // refill_wanted_only_when_three_or_fewer_remain_and_none_pending
    // refill_landed_with_stale_ticket_is_ignored
    // refill_landed_dedupes_against_whole_queue_not_just_upcoming
    // entry_ids_survive_inserts_and_removals (jump(id) after insert_next hits the same item)
}
