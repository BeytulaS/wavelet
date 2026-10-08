//! Search results and the lists the user moves through. View state only.
//! Results are never the queue: Enter copies an item into a `Command`.

use std::sync::mpsc::Sender;

use crate::domain::{Playable, SearchId, Tickets};
use crate::player::PlayerHandle;
use crate::tui::Msg;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    YouTube,
    Radio,
}

pub struct Browse {
    pub tab: Tab,
    pub youtube: ResultList,
    pub radio: ResultList,
    tickets: Tickets,
}

pub struct ResultList {
    pub query: String,
    pub items: Vec<Playable>,
    pub selected: usize,
    /// Sticky scroll offset (Myx scroll_offset), written back from FrameOut.
    pub offset: usize,
    pub state: Fetch,
}

/// The slot. `Loading` holds the only ticket whose result will be accepted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fetch {
    Idle,
    Loading(SearchId),
    Failed,
}

impl ResultList {
    /// The stations in this list, in display order. Enter on a station queues all of them.
    pub fn stations(&self) -> Vec<std::sync::Arc<crate::domain::Station>> {
        self.items.iter().filter_map(|i| match i { Playable::Station(s) => Some(s.clone()), _ => None }).collect()
    }
}

impl Browse {
    /// Start a search on the active tab. Replaces the slot's ticket, so any
    /// search still running for this tab becomes stale by construction.
    /// Radio with an empty query means top-voted.
    pub fn submit(&mut self, query: String, tx: &Sender<Msg>) {
        // TODO: ticket = tickets.issue(); list.state = Loading(ticket); list.query = query
        //       spawn: ytm::search / radio_browser::{search, top_voted} -> Msg::Search{tab, ticket, result}
        todo!()
    }

    /// Apply a result if its ticket still owns the slot. On a fresh YouTube
    /// result, prewarm the top three so Enter is fast (spec: < 2 s cold).
    pub fn landed(&mut self, tab: Tab, ticket: SearchId, result: Result<Vec<Playable>, String>, player: &PlayerHandle) {
        // TODO: let list = self.list_mut(tab);
        //       if list.state != Fetch::Loading(ticket) { return }   // stale
        //       Ok(items) -> items, selected 0, offset 0, Idle;
        //                    if tab == YouTube { player.send(Prewarm(first 3 track ids)) }
        //       Err(_)    -> Failed (keep old items visible)
        todo!()
    }

    pub fn selected(&self) -> Option<Playable> {
        todo!("active list items[selected].clone()")
    }

    pub fn move_by(&mut self, delta: isize) {
        todo!("clamp to 0..len")
    }

    pub fn active(&self) -> &ResultList {
        match self.tab {
            Tab::YouTube => &self.youtube,
            Tab::Radio => &self.radio,
        }
    }
}
