//! MPRIS on its own thread. mpris-server is async (zbus); this is the only async
//! code in wavelet, so it gets one thread with a local executor instead of
//! putting tokio under everything.
//!
//! In: snapshots, via a `Sink` that pushes into an async channel.
//! Out: `Command`s through the same `PlayerHandle` the TUI uses.

use std::sync::Arc;

use crate::player::{Command, Out, PlayerHandle, Snapshot};

pub fn spawn(player: PlayerHandle) {
    // TODO:
    //   let (tx, rx) = async_channel::bounded::<Arc<Snapshot>>(4);
    //   player.subscribe(Box::new(move |out| match out {
    //       Out::Snapshot(s) => { let _ = tx.force_send(s.clone()); !tx.is_closed() }   // latest wins
    //       Out::Notice(_) => true,
    //   }));
    //   thread "mpris": futures_lite::future::block_on(async {
    //       let mp = mpris_server::Player::builder("wavelet").can_play(true).can_pause(true)
    //                .can_go_next(true).can_go_previous(true).can_seek(true).can_control(true)
    //                .build().await?;
    //       mp.connect_play_pause(h -> TogglePause); connect_next -> Next; connect_previous -> Prev;
    //       connect_seek(offset_us -> Seek(By(secs))); connect_set_position(-> Seek(At));
    //       connect_set_volume(f64 0..1 -> Volume(At(v*100)));
    //       race(mp.run(), async { while let Ok(s) = rx.recv().await { apply(&mp, &s).await } })
    //   })
    let _ = (player, Command::Next);
    todo!()
}

/// Snapshot -> MPRIS properties: PlaybackStatus, Metadata (trackid from EntryId,
/// title, artist list, album, length for tracks only, artUrl = art http URL),
/// Volume, CanSeek = now is a Track (stations report false), Position.
/// Emits `Seeked` when the position jumps by more than 2 s against the extrapolated clock.
async fn apply(mp: &mpris_server::Player, s: &Snapshot) {
    let _ = (mp, s, Arc::new(()));
    todo!()
}
