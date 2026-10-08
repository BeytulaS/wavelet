//! MPRIS on its own thread. mpris-server is async (zbus); this is the only
//! async code in wavelet, so it gets one thread and `block_on` instead of a
//! runtime under everything.
//!
//! In: snapshots through a `Sink` that pushes into an async channel.
//! Out: `Command`s through the same `PlayerHandle` the TUI uses.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mpris_server::{Metadata, PlaybackStatus, Player, Time, TrackId};

use crate::domain::Volume;
use crate::player::api::{Now, SeekTo, StationView, TrackView, VolumeTo};
use crate::player::{Command, Out, PlayerHandle, Snapshot};

pub fn spawn(player: PlayerHandle) {
    let (tx, rx) = async_channel::bounded::<Arc<Snapshot>>(1);
    player.subscribe(Box::new(move |out| {
        if let Out::Snapshot(s) = out {
            // Latest wins: a slow D-Bus never backs up the player thread.
            let _ = tx.force_send(s.clone());
        }
        !tx.is_closed()
    }));
    let _ = std::thread::Builder::new().name("mpris".into()).spawn(move || {
        // No terminal to report to: the TUI owns it. Without D-Bus, wavelet runs without MPRIS.
        let _ = futures_lite::future::block_on(serve(player, rx));
    });
}

async fn serve(h: PlayerHandle, rx: async_channel::Receiver<Arc<Snapshot>>) -> mpris_server::zbus::Result<()> {
    let mp = Player::builder("wavelet")
        .identity("wavelet")
        .can_play(true)
        .can_pause(true)
        .can_go_next(true)
        .can_go_previous(true)
        .can_seek(true)
        .can_control(true)
        .build()
        .await?;
    let on = |cmd: fn() -> Command| {
        let h = h.clone();
        move |_: &Player| h.send(cmd())
    };
    mp.connect_play_pause(on(|| Command::TogglePause));
    mp.connect_play(on(|| Command::SetPaused(false)));
    mp.connect_pause(on(|| Command::SetPaused(true)));
    mp.connect_stop(on(|| Command::SetPaused(true)));
    mp.connect_next(on(|| Command::Next));
    mp.connect_previous(on(|| Command::Prev));
    let hs = h.clone();
    mp.connect_seek(move |_, offset| hs.send(Command::Seek(SeekTo::By((offset.as_millis() / 1000) as i32))));
    let hp = h.clone();
    mp.connect_set_position(move |_, _, at| {
        hp.send(Command::Seek(SeekTo::At(Duration::from_micros(at.as_micros().max(0) as u64))))
    });
    let hv = h.clone();
    mp.connect_set_volume(move |_, v| hv.send(Command::Volume(VolumeTo::At(Volume::new((v * 100.0).round() as i32)))));

    let updates = async {
        let mut last = State::default();
        while let Ok(s) = rx.recv().await {
            last = apply(&mp, &s, last).await;
        }
    };
    futures_lite::future::or(mp.run(), updates).await;
    Ok(())
}

#[derive(Default, PartialEq, Clone)]
struct Meta {
    entry: Option<u64>,
    title: String,
    artists: Vec<String>,
    album: Option<String>,
    length: Option<Duration>,
    art: Option<String>,
}

#[derive(Default)]
struct State {
    meta: Meta,
    /// Position and when it was seen, while running, to detect seeks.
    clock: Option<(Duration, Instant, bool)>,
}

fn meta(s: &Snapshot) -> (Meta, PlaybackStatus, Option<(Duration, bool)>) {
    match &s.now {
        None => (Meta::default(), PlaybackStatus::Stopped, None),
        Some(Now::Track { entry, track, stage }) => {
            let (status, clock, length) = match stage {
                TrackView::Playing { clock, duration } => {
                    let st = if clock.running { PlaybackStatus::Playing } else { PlaybackStatus::Paused };
                    (st, Some((clock.pos, clock.running)), duration.or(track.duration))
                }
                _ => (PlaybackStatus::Playing, None, track.duration),
            };
            let m = Meta {
                entry: Some(entry.get()),
                title: track.title.clone(),
                artists: track.artists.iter().map(|a| a.name.clone()).collect(),
                album: track.album.clone(),
                length,
                art: track.art.as_ref().map(|u| u.to_string()),
            };
            (m, status, clock)
        }
        Some(Now::Station { entry, station, stage }) => {
            let (status, title) = match stage {
                StationView::OnAir { listened, icy } => {
                    let st = if listened.running { PlaybackStatus::Playing } else { PlaybackStatus::Paused };
                    (st, icy.clone())
                }
                StationView::Connecting => (PlaybackStatus::Playing, None),
            };
            let m = Meta {
                entry: Some(entry.get()),
                title: title.unwrap_or_else(|| station.name.clone()),
                artists: vec![station.name.clone()],
                album: None,
                length: None,
                art: station.favicon.as_ref().map(|u| u.to_string()),
            };
            (m, status, None)
        }
    }
}

fn to_metadata(m: &Meta) -> Metadata {
    let mut b = Metadata::builder().title(m.title.clone()).artist(m.artists.clone());
    if let Some(id) = m.entry.and_then(|e| TrackId::try_from(format!("/org/wavelet/entry/{e}")).ok()) {
        b = b.trackid(id);
    }
    if let Some(a) = &m.album {
        b = b.album(a.clone());
    }
    if let Some(l) = m.length {
        b = b.length(Time::from_micros(l.as_micros() as i64));
    }
    if let Some(u) = &m.art {
        b = b.art_url(u.clone());
    }
    b.build()
}

async fn apply(mp: &Player, s: &Snapshot, last: State) -> State {
    let now = Instant::now();
    let (m, status, clock) = meta(s);
    if m != last.meta {
        let _ = mp.set_metadata(to_metadata(&m)).await;
    }
    let _ = mp.set_playback_status(status).await;
    let _ = mp.set_can_seek(clock.is_some()).await;
    let _ = mp.set_volume(s.volume.get() as f64 / 100.0).await;
    let pos = clock.map_or(Duration::ZERO, |(p, _)| p);
    mp.set_position(Time::from_micros(pos.as_micros() as i64));
    if let (Some((p, _)), Some((lp, at, running))) = (clock, last.clock)
        && m.entry == last.meta.entry
    {
        let expected = if running { lp + now.saturating_duration_since(at) } else { lp };
        if p.abs_diff(expected) > Duration::from_secs(2) {
            let _ = mp.seeked(Time::from_micros(p.as_micros() as i64)).await;
        }
    }
    State { meta: m, clock: clock.map(|(p, r)| (p, now, r)) }
}
