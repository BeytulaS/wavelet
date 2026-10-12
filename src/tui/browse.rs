use std::sync::Arc;
use std::sync::mpsc::Sender;

use crate::daemon::client::Remote;
use crate::domain::{Playable, SearchId, Station, Tickets};
use crate::player::api::{Command, Snapshot};
use crate::player::queue::EntryId;
use crate::sources::{Http, radio_browser, ytm};
use crate::tui::Msg;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    YouTube,
    Radio,
    FavoriteSongs,
    FavoriteStations,
    Queue,
}

impl Tab {
    pub const SOURCES: [Tab; 4] = [Tab::YouTube, Tab::Radio, Tab::FavoriteSongs, Tab::FavoriteStations];

    pub fn label(self) -> &'static str {
        match self {
            Tab::YouTube => "YouTube",
            Tab::Radio => "Radio",
            Tab::FavoriteSongs => "Favorite songs",
            Tab::FavoriteStations => "Favorite stations",
            Tab::Queue => "Queue",
        }
    }
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Cursor {
    pub selected: usize,
    pub offset: usize,
}

pub struct ResultList {
    pub query: String,
    pub items: Vec<Playable>,
    pub cursor: Cursor,
    pub state: Fetch,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Fetch {
    Idle,
    Loading(SearchId),
    Failed(String),
}

pub struct Browse {
    pub tab: Tab,
    source: Tab,
    pub youtube: ResultList,
    pub radio: ResultList,
    favorite_songs: Cursor,
    favorite_stations: Cursor,
    queue: Cursor,
    tickets: Tickets,
}

pub struct Row {
    pub item: Playable,
    pub entry: Option<EntryId>,
    pub from_radio: bool,
}

pub enum Selected {
    Item(Playable),
    Entry(EntryId, Playable),
}

impl ResultList {
    fn new() -> ResultList {
        ResultList { query: String::new(), items: vec![], cursor: Cursor::default(), state: Fetch::Idle }
    }
}

impl Default for Browse {
    fn default() -> Self {
        Browse {
            tab: Tab::YouTube,
            source: Tab::YouTube,
            youtube: ResultList::new(),
            radio: ResultList::new(),
            favorite_songs: Cursor::default(),
            favorite_stations: Cursor::default(),
            queue: Cursor::default(),
            tickets: Tickets::default(),
        }
    }
}

impl Browse {
    pub fn rows(&self, snap: &Snapshot) -> Vec<Row> {
        let plain = |item| Row { item, entry: None, from_radio: false };
        match self.tab {
            Tab::YouTube => self.youtube.items.iter().cloned().map(plain).collect(),
            Tab::Radio => self.radio.items.iter().cloned().map(plain).collect(),
            Tab::FavoriteSongs => snap.favorites.tracks.iter().cloned().map(Playable::Track).map(plain).collect(),
            Tab::FavoriteStations => {
                snap.favorites.stations.iter().cloned().map(Playable::Station).map(plain).collect()
            }
            Tab::Queue => snap
                .queue
                .iter()
                .map(|r| Row { item: r.item.clone(), entry: Some(r.id), from_radio: r.from_radio })
                .collect(),
        }
    }

    pub fn list(&self, tab: Tab) -> Option<&ResultList> {
        match tab {
            Tab::YouTube => Some(&self.youtube),
            Tab::Radio => Some(&self.radio),
            Tab::FavoriteSongs | Tab::FavoriteStations | Tab::Queue => None,
        }
    }

    fn list_mut(&mut self, tab: Tab) -> Option<&mut ResultList> {
        match tab {
            Tab::YouTube => Some(&mut self.youtube),
            Tab::Radio => Some(&mut self.radio),
            Tab::FavoriteSongs | Tab::FavoriteStations | Tab::Queue => None,
        }
    }

    pub fn cursor(&self) -> Cursor {
        match self.tab {
            Tab::YouTube => self.youtube.cursor,
            Tab::Radio => self.radio.cursor,
            Tab::FavoriteSongs => self.favorite_songs,
            Tab::FavoriteStations => self.favorite_stations,
            Tab::Queue => self.queue,
        }
    }

    fn cursor_mut(&mut self) -> &mut Cursor {
        match self.tab {
            Tab::YouTube => &mut self.youtube.cursor,
            Tab::Radio => &mut self.radio.cursor,
            Tab::FavoriteSongs => &mut self.favorite_songs,
            Tab::FavoriteStations => &mut self.favorite_stations,
            Tab::Queue => &mut self.queue,
        }
    }

    pub fn set_offset(&mut self, offset: usize) {
        self.cursor_mut().offset = offset;
    }

    pub fn selected(&self, snap: &Snapshot) -> Option<Selected> {
        let mut rows = self.rows(snap);
        let last = rows.len().checked_sub(1)?;
        let row = rows.swap_remove(self.cursor().selected.min(last));
        Some(match row.entry {
            Some(id) => Selected::Entry(id, row.item),
            None => Selected::Item(row.item),
        })
    }

    pub fn stations(&self, snap: &Snapshot) -> Vec<Arc<Station>> {
        self.rows(snap)
            .into_iter()
            .filter_map(|r| match r.item {
                Playable::Station(s) => Some(s),
                Playable::Track(_) => None,
            })
            .collect()
    }

    pub fn move_by(&mut self, delta: isize, snap: &Snapshot) {
        let len = self.rows(snap).len();
        let c = self.cursor_mut();
        c.selected = c.selected.saturating_add_signed(delta).min(len.saturating_sub(1));
    }

    pub fn set_tab(&mut self, tab: Tab, snap: &Snapshot) {
        if tab == Tab::Queue {
            self.queue.selected = snap.cursor.unwrap_or(0);
        } else {
            self.source = tab;
        }
        self.tab = tab;
    }

    pub fn cycle_tab(&mut self, snap: &Snapshot) {
        let i = Tab::SOURCES.iter().position(|t| *t == self.source).unwrap_or(0);
        let next = if self.tab == Tab::Queue { self.source } else { Tab::SOURCES[(i + 1) % Tab::SOURCES.len()] };
        self.set_tab(next, snap);
    }

    pub fn toggle_queue(&mut self, snap: &Snapshot) {
        let to = if self.tab == Tab::Queue { self.source } else { Tab::Queue };
        self.set_tab(to, snap);
    }

    pub fn to_search_tab(&mut self) {
        if self.list(self.tab).is_none() {
            self.tab = if self.source == Tab::Radio { Tab::Radio } else { Tab::YouTube };
            self.source = self.tab;
        }
    }

    pub fn submit(&mut self, query: String, tx: &Sender<Msg>, http: &Http) {
        let tab = self.tab;
        let ticket = self.tickets.issue();
        let Some(list) = self.list_mut(tab) else { return };
        list.state = Fetch::Loading(ticket);
        list.query = query.clone();
        let (tx, http) = (tx.clone(), http.clone());
        std::thread::spawn(move || {
            let result = match tab {
                Tab::YouTube => {
                    ytm::search(&http, &query).map(|v| v.into_iter().map(|t| Playable::Track(Arc::new(t))).collect())
                }
                _ if query.trim().is_empty() => radio_browser::top_voted(&http).map(stations),
                _ => radio_browser::search(&http, &query).map(stations),
            };
            let _ = tx.send(Msg::Search { tab, ticket, result });
        });
    }

    pub fn landed(&mut self, tab: Tab, ticket: SearchId, result: Result<Vec<Playable>, String>, player: &Remote) {
        let Some(list) = self.list_mut(tab) else { return };
        if list.state != Fetch::Loading(ticket) {
            return;
        }
        match result {
            Ok(items) => {
                if tab == Tab::YouTube {
                    let ids = items
                        .iter()
                        .take(3)
                        .filter_map(|i| match i {
                            Playable::Track(t) => Some(t.id.clone()),
                            Playable::Station(_) => None,
                        })
                        .collect();
                    player.send(Command::Warm(ids));
                }
                list.items = items;
                list.cursor = Cursor::default();
                list.state = Fetch::Idle;
            }
            Err(e) => list.state = Fetch::Failed(e),
        }
    }
}

fn stations(v: Vec<Station>) -> Vec<Playable> {
    v.into_iter().map(|s| Playable::Station(Arc::new(s))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::api::Favorites;
    use crate::player::queue::tests::{station, track, vid};

    #[test]
    fn favorite_songs_and_stations_are_separate_tabs() {
        let snap = Snapshot {
            favorites: Favorites { tracks: vec![track(&vid('a'))], stations: vec![station(1), station(2)] },
            ..Snapshot::empty()
        };
        let mut b = Browse::default();
        b.set_tab(Tab::FavoriteSongs, &snap);
        let songs: Vec<Playable> = b.rows(&snap).into_iter().map(|r| r.item).collect();
        assert_eq!(songs, vec![Playable::Track(track(&vid('a')))]);
        b.cycle_tab(&snap);
        assert_eq!(b.tab, Tab::FavoriteStations);
        assert_eq!(b.stations(&snap), vec![station(1), station(2)]);
        assert_eq!(b.rows(&snap).len(), 2, "no songs among the stations");
    }
}
