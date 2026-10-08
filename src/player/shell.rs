//! The player thread: the I/O half of the player.
//!
//! One thread owns `Core` and the mpv child. It reads one inbox, steps the
//! core, runs the returned effects, and publishes a snapshot when it changed.
//! Workers (yt-dlp, HTTP, the mpv socket reader) only send into the inbox.

use std::path::PathBuf;

use crate::player::api::{Command, Out};
use crate::player::core::{Input, Restore};
use crate::player::store::Store;

/// The only way to talk to the player. Cloneable. The later daemon client
/// implements the same methods over the Unix socket.
#[derive(Clone)]
pub struct PlayerHandle {
    tx: std::sync::mpsc::Sender<Inbox>,
}

impl PlayerHandle {
    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(Inbox::Core(Input::Command(cmd)));
    }
    /// The sink receives the current snapshot at once, then every change and
    /// notice. Returning `false` from the sink unsubscribes it.
    pub fn subscribe(&self, sink: Sink) {
        let _ = self.tx.send(Inbox::Subscribe(sink));
    }
}

pub type Sink = Box<dyn FnMut(&Out) -> bool + Send>;

pub struct PlayerOpts {
    pub restore: Restore,
    pub audio_device: Option<String>,
    pub runtime_dir: PathBuf,
    pub store: Store,
}

/// Everything the player thread can receive.
pub enum Inbox {
    /// Commands, worker results, and attributed mpv events.
    Core(Input),
    /// The mpv socket hit EOF. Carries the generation of the reader that saw it,
    /// so a reader of an already-replaced mpv cannot trigger a second respawn.
    MpvGone(u64),
    Subscribe(Sink),
}

pub fn spawn(opts: PlayerOpts) -> std::io::Result<PlayerHandle> {
    let _ = opts;
    todo!()
}
