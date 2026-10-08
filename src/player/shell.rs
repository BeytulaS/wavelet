//! The player thread: the I/O half of the player.
//!
//! One thread owns `Core` and `Mpv`. It reads one inbox, steps the core, runs
//! the returned effects, and publishes a snapshot when it changed. Workers
//! (yt-dlp, HTTP, the mpv socket reader) only ever send into the inbox; none of
//! them holds player state, so nothing here needs a lock.
//!
//! The mpv adapter lives in this file on purpose: "user presses n" reads as
//! tui/input.rs -> player/core.rs -> here, three files.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Child;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crate::domain::{LoadId, Volume};
use crate::player::api::{Command, Out, Snapshot};
use crate::player::core::{Core, Effect, EndReason, EngineCmd, EngineEvent, Input, Restore};
use crate::player::store::Store;
use crate::sources::{radio_browser, ytdlp, ytm};

/// The only way to talk to the player. Cloneable. The later daemon client
/// implements the same three methods over the Unix socket.
#[derive(Clone)]
pub struct PlayerHandle {
    tx: Sender<Inbox>,
}

impl PlayerHandle {
    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(Inbox::Core(Input::Command(cmd)));
    }
    /// The sink receives the current snapshot at once, then every change and notice.
    /// Returning `false` from the sink unsubscribes it.
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

/// Starts mpv and the player thread. Returns once mpv's socket answers, so the
/// first command never races mpv startup.
pub fn spawn(opts: PlayerOpts) -> std::io::Result<PlayerHandle> {
    // TODO:
    //   let (tx, rx) = channel();
    //   let mpv = Mpv::spawn(&opts.runtime_dir, opts.audio_device.as_deref(), tx.clone())?;
    //   let (core, fx) = Core::new(opts.restore);
    //   let lanes = ytdlp::Lanes::spawn(tx.clone() mapped to Inbox::Core(Input::Resolved{..}));
    //   thread::Builder::new().name("player").spawn(move || Shell{..}.run(fx));
    let _: (Sender<Inbox>, Receiver<Inbox>) = channel();
    todo!()
}

/// Everything the player thread can receive.
pub enum Inbox {
    Core(Input),
    /// A raw line from mpv's socket, parsed into the adapter's private shape.
    Mpv(MpvRaw),
    Subscribe(Sink),
}

struct Shell {
    core: Core,
    mpv: Mpv,
    lanes: ytdlp::Lanes,
    http: reqwest::blocking::Client,
    store: Store,
    tx: Sender<Inbox>,
    rx: Receiver<Inbox>,
    sinks: Vec<Sink>,
    last: Arc<Snapshot>,
}

const TICK: Duration = Duration::from_secs(1);

impl Shell {
    fn run(mut self, initial: Vec<Effect>) {
        for e in initial {
            self.run_effect(e);
        }
        let mut next_tick = Instant::now() + TICK;
        loop {
            let input = match self.rx.recv_timeout(next_tick.saturating_duration_since(Instant::now())) {
                Ok(Inbox::Core(i)) => Some(i),
                Ok(Inbox::Mpv(raw)) => self.mpv.translate(raw).map(Input::Engine),
                Ok(Inbox::Subscribe(mut sink)) => {
                    if sink(&Out::Snapshot(self.last.clone())) {
                        self.sinks.push(sink);
                    }
                    None
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    next_tick = Instant::now() + TICK;
                    Some(Input::Tick)
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            };
            let Some(input) = input else { continue };
            let quitting = matches!(input, Input::Command(Command::Shutdown));
            let now = Instant::now();
            for e in self.core.step(input, now) {
                self.run_effect(e);
            }
            self.publish_if_changed(now);
            if quitting {
                return;
            }
        }
    }

    fn run_effect(&mut self, effect: Effect) {
        match effect {
            Effect::Engine(cmd) => self.mpv.send(cmd),
            Effect::Resolve { id, urgency } => self.lanes.request(id, urgency),
            Effect::Refill { ticket, seed } => {
                let (http, tx) = (self.http.clone(), self.tx.clone());
                std::thread::spawn(move || {
                    let result = ytm::radio(&http, &seed).map(|v| v.into_iter().map(Arc::new).collect());
                    let _ = tx.send(Inbox::Core(Input::Refilled { ticket, result }));
                });
            }
            Effect::CountClick(id) => {
                let http = self.http.clone();
                std::thread::spawn(move || radio_browser::count_click(&http, &id));
            }
            Effect::SaveLibrary(lib) => self.store.save_library(&lib),
            Effect::SaveSession(s) => self.store.save_session(&s),
            Effect::Notify(n) => self.broadcast(&Out::Notice(n)),
        }
    }

    /// Derived publish: build, compare, send on difference. Position updates are
    /// throttled to 1 Hz by the adapter, so this runs a few times a second at most.
    fn publish_if_changed(&mut self, now: Instant) {
        let snap = self.core.snapshot(now);
        if *self.last != snap {
            self.last = Arc::new(snap);
            let out = Out::Snapshot(self.last.clone());
            self.broadcast(&out);
        }
    }

    fn broadcast(&mut self, out: &Out) {
        self.sinks.retain_mut(|s| s(out));
    }
}

// ------------------------------------------------------------------ mpv adapter

/// mpv child plus its JSON IPC socket. Owned by the player thread only; the
/// socket reader thread forwards parsed lines into the inbox and holds no state.
///
/// Hides three mpv facts from the core:
/// 1. Which file an event belongs to. `loadfile` goes out with `request_id = LoadId`;
///    mpv's reply carries `playlist_entry_id` (verified on mpv 0.41), and start-file,
///    end-file carry the same id. `bound` maps entry id -> LoadId.
/// 2. `time-pos` has no entry id, so it is tagged with the entry of the last
///    start-file, and dropped between end-file and the next start-file.
/// 3. `time-pos` fires many times a second; only discontinuities and 1 s steps pass.
pub struct Mpv {
    child: Child,
    sock: UnixStream,
    sock_path: PathBuf,
    args: Vec<String>,
    tx: Sender<Inbox>,
    bound: HashMap<i64, LoadId>,
    /// Entry id of the last start-file, until its end-file.
    playing: Option<i64>,
    began: bool,
    last_pos: Option<Duration>,
}

/// The adapter's private parse of an mpv line. Not visible to the core.
pub enum MpvRaw {
    Reply { request_id: u64, entry_id: Option<i64> },
    StartFile { entry_id: i64 },
    EndFile { entry_id: i64, reason: String, error: Option<String> },
    PlaybackRestart,
    Prop { name: String, data: serde_json::Value },
    /// Socket hit EOF: mpv died.
    Closed,
}

impl Mpv {
    /// `mpv --idle=yes --no-video --no-config --no-terminal --ytdl=no
    ///  --audio-client-name=wavelet --input-ipc-server=$XDG_RUNTIME_DIR/wavelet/mpv.sock
    ///  [--audio-device=..]`, then observe pause, volume, time-pos, duration,
    /// metadata/by-key/icy-title. The socket path must stay under 108 bytes (sun_path);
    /// $XDG_RUNTIME_DIR keeps it short.
    pub fn spawn(runtime_dir: &std::path::Path, audio_device: Option<&str>, tx: Sender<Inbox>) -> std::io::Result<Mpv> {
        let _ = (BufReader::new(std::io::empty()).lines(), runtime_dir, audio_device, tx);
        todo!("spawn, poll-connect up to 5 s, start reader thread, observe_property x5")
    }

    pub fn send(&mut self, cmd: EngineCmd) {
        // TODO: one JSON line each.
        //   Load{load, url, start} -> {"command":["loadfile", url, "replace", -1, "start=<s>"?], "request_id": load.request_id()}
        //                             then ["set_property","pause",false]
        //   SetPause(p)            -> ["set_property","pause",p]
        //   Seek(By(s))            -> ["seek", s, "relative"];  Seek(At(d)) -> ["seek", d, "absolute"]
        //   SetVolume(v)           -> ["set_property","volume",v]
        //   Stop                   -> ["stop"]
        //   Quit                   -> ["quit"], wait child
        // A write error means mpv is gone: treat like MpvRaw::Closed (respawn below).
        let _ = writeln!(self.sock);
        todo!()
    }

    /// Raw mpv line -> core event, or nothing.
    pub fn translate(&mut self, raw: MpvRaw) -> Option<EngineEvent> {
        // TODO:
        //   Reply{request_id, entry_id: Some(e)} -> bound.insert(e, LoadId::from_request_id(request_id)); None
        //   StartFile{e}     -> playing = Some(e); began = false; last_pos = None; None
        //   PlaybackRestart  -> load = bound[playing?];
        //                       if !began { began = true; Began{load} } else { Position{load, pos: last_pos} }  (a seek landed)
        //   EndFile{e, reason, error} -> load = bound.remove(e)?; if playing == Some(e) { playing = None }
        //                       "eof"   -> Ended{load, Eof}
        //                       "error" -> Ended{load, Error(error)}
        //                       "stop" | "quit" | "redirect" -> None   (our own replace)
        //   Prop "time-pos"  -> load = bound[playing?]; pos; emit Position only if
        //                       last_pos is None or |pos - last_pos - elapsed| > 1 s jitter or pos crossed a whole second
        //   Prop "duration"  -> Duration{load, ..}
        //   Prop "metadata/by-key/icy-title" -> Icy{load, title: non-empty string}
        //   Prop "pause"     -> Paused(bool);  Prop "volume" -> Volume(Volume::new(round))
        //   Closed           -> self.respawn(); bound.clear(); playing = None; Some(Restarted)
        let _ = (Volume::new(0), EndReason::Eof);
        todo!()
    }

    fn respawn(&mut self) {
        todo!("kill/wait old child, spawn with self.args, reconnect, re-observe")
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
