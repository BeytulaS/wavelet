//! The player thread: the I/O half of the player.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::player::api::{Command, Level, Notice, Out, Snapshot};
use crate::player::core::{Core, Effect, EngineCmd, EngineEvent, Input, Restore};
use crate::player::mpv::{Attribution, attribute};
use crate::player::store::Store;
use crate::sources::{Http, radio_browser, ytdlp, ytm};

#[derive(Clone)]
pub struct PlayerHandle {
    tx: Sender<Inbox>,
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

    /// Saves the session, quits mpv, and waits for both (bounded), so no child
    /// outlives the process.
    pub fn shutdown(&self, wait: Duration) {
        let (done, rx) = channel();
        if self.tx.send(Inbox::Shutdown(done)).is_ok() {
            let _ = rx.recv_timeout(wait);
        }
    }
}

pub type Sink = Box<dyn FnMut(&Out) -> bool + Send>;

pub struct PlayerOpts {
    pub restore: Restore,
    pub audio_device: Option<String>,
    pub runtime_dir: PathBuf,
    pub store: Store,
    pub http: Http,
}

pub enum Inbox {
    Core(Input),
    MpvGone(u64),
    Subscribe(Sink),
    Shutdown(Sender<()>),
}

/// Starts mpv and the player thread. Returns once mpv's socket answers, so the
/// first command never races mpv startup.
pub fn spawn(opts: PlayerOpts) -> std::io::Result<PlayerHandle> {
    let (tx, rx) = channel();
    let PlayerOpts { restore, audio_device, runtime_dir, store, http } = opts;
    let (startup, ready) = channel();
    let tx2 = tx.clone();
    thread::Builder::new().name("player".into()).spawn(move || {
        // mpv is spawned here, not by the caller, because PR_SET_PDEATHSIG fires
        // when the spawning thread exits; this thread lives as long as the player.
        let mpv = match Mpv::spawn(&runtime_dir, audio_device, tx2.clone(), 0) {
            Ok(m) => m,
            Err(e) => {
                let _ = startup.send(Err(e));
                return;
            }
        };
        let (core, fx) = Core::new(restore);
        let last = Arc::new(core.snapshot(Instant::now()));
        let _ = startup.send(Ok(()));
        Shell { core, mpv, http, store, tx: tx2, rx, sinks: Vec::new(), last, respawned: None }.run(fx);
    })?;
    ready.recv().map_err(|_| std::io::Error::other("player thread died during startup"))??;
    Ok(PlayerHandle { tx })
}

struct Shell {
    core: Core,
    mpv: Mpv,
    http: Http,
    store: Store,
    tx: Sender<Inbox>,
    rx: Receiver<Inbox>,
    sinks: Vec<Sink>,
    last: Arc<Snapshot>,
    respawned: Option<Instant>,
}

const TICK: Duration = Duration::from_secs(1);

impl Shell {
    fn run(mut self, initial: Vec<Effect>) {
        for e in initial {
            self.run_effect(e);
        }
        let mut next_tick = Instant::now() + TICK;
        loop {
            let msg = self.rx.recv_timeout(next_tick.saturating_duration_since(Instant::now()));
            let (input, done) = match msg {
                Ok(Inbox::Core(i)) => (i, None),
                Ok(Inbox::Shutdown(done)) => (Input::Command(Command::Shutdown), Some(done)),
                Ok(Inbox::MpvGone(generation)) => {
                    if generation != self.mpv.generation || !self.respawn_mpv() {
                        continue;
                    }
                    (Input::Engine(EngineEvent::Restarted), None)
                }
                Ok(Inbox::Subscribe(mut sink)) => {
                    if sink(&Out::Snapshot(self.last.clone())) {
                        self.sinks.push(sink);
                    }
                    continue;
                }
                Err(RecvTimeoutError::Timeout) => {
                    next_tick = Instant::now() + TICK;
                    (Input::Tick, None)
                }
                Err(RecvTimeoutError::Disconnected) => return,
            };
            let quitting = matches!(input, Input::Command(Command::Shutdown));
            let now = Instant::now();
            for e in self.core.step(input, now) {
                self.run_effect(e);
            }
            self.publish_if_changed(now);
            if quitting {
                if let Some(done) = done {
                    let _ = done.send(());
                }
                return;
            }
        }
    }

    fn respawn_mpv(&mut self) -> bool {
        if self.respawned.is_some_and(|t| t.elapsed() < Duration::from_secs(5)) {
            thread::sleep(Duration::from_secs(2));
        }
        self.respawned = Some(Instant::now());
        match self.mpv.respawn() {
            Ok(()) => true,
            Err(e) => {
                self.notify(Level::Error, format!("mpv failed to restart: {e}"));
                false
            }
        }
    }

    fn run_effect(&mut self, effect: Effect) {
        match effect {
            Effect::Engine(EngineCmd::Quit) => self.mpv.quit(),
            Effect::Engine(cmd) => self.mpv.send(&cmd),
            Effect::Resolve(id) => {
                let tx = self.tx.clone();
                thread::spawn(move || {
                    let result = ytdlp::resolve(&id);
                    let _ = tx.send(Inbox::Core(Input::Resolved { id, result }));
                });
            }
            Effect::Refill { ticket, seed } => {
                let (http, tx) = (self.http.clone(), self.tx.clone());
                thread::spawn(move || {
                    let result = ytm::radio(&http, &seed).map(|v| v.into_iter().map(Arc::new).collect());
                    let _ = tx.send(Inbox::Core(Input::Refilled { ticket, result }));
                });
            }
            Effect::CountClick(id) => {
                let http = self.http.clone();
                thread::spawn(move || radio_browser::count_click(&http, &id));
            }
            Effect::SaveLibrary(lib) => {
                if let Err(e) = self.store.save_library(&lib) {
                    self.notify(Level::Error, format!("could not save favorites: {e}"));
                }
            }
            Effect::SaveSession(s) => {
                if let Err(e) = self.store.save_session(&s) {
                    self.notify(Level::Warn, format!("could not save the session: {e}"));
                }
            }
            Effect::Notify(n) => self.broadcast(&Out::Notice(n)),
        }
    }

    fn notify(&mut self, level: Level, text: String) {
        self.broadcast(&Out::Notice(Notice { level, text }));
    }

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

struct Mpv {
    child: Child,
    sock: UnixStream,
    sock_path: PathBuf,
    audio_device: Option<String>,
    tx: Sender<Inbox>,
    generation: u64,
}

const OBSERVED: [&str; 3] = ["time-pos", "duration", "metadata/by-key/icy-title"];

impl Mpv {
    /// The socket lives under `$XDG_RUNTIME_DIR` because sun_path holds 108 bytes.
    fn spawn(
        runtime_dir: &Path,
        audio_device: Option<String>,
        tx: Sender<Inbox>,
        generation: u64,
    ) -> std::io::Result<Mpv> {
        std::fs::create_dir_all(runtime_dir)?;
        let sock_path = runtime_dir.join("mpv.sock");
        let _ = std::fs::remove_file(&sock_path);
        let mut cmd = std::process::Command::new("mpv");
        cmd.args(["--idle=yes", "--no-video", "--no-config", "--no-terminal", "--ytdl=no"])
            .arg(format!("--audio-client-name={AUDIO_CLIENT_NAME}"))
            .arg(format!("--input-ipc-server={}", sock_path.display()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(dev) = &audio_device {
            cmd.arg(format!("--audio-device={dev}"));
        }
        die_with_spawning_thread(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| std::io::Error::other(format!("mpv: {e}")))?;
        let start = Instant::now();
        let sock = loop {
            if let Ok(s) = UnixStream::connect(&sock_path) {
                break s;
            }
            if child.try_wait()?.is_some() || start.elapsed() > Duration::from_secs(5) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(std::io::Error::other("mpv did not open its IPC socket"));
            }
            thread::sleep(Duration::from_millis(20));
        };
        let reader = sock.try_clone()?;
        let reader_tx = tx.clone();
        thread::Builder::new().name("mpv-reader".into()).spawn(move || {
            let mut attribution = Attribution::default();
            for line in BufReader::new(reader).lines() {
                let Ok(line) = line else { break };
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(ev) = attribute(&mut attribution, &v)
                    && reader_tx.send(Inbox::Core(Input::Engine(ev))).is_err()
                {
                    return;
                }
            }
            let _ = reader_tx.send(Inbox::MpvGone(generation));
        })?;
        let mut mpv = Mpv { child, sock, sock_path, audio_device, tx, generation };
        for (i, p) in OBSERVED.iter().enumerate() {
            mpv.write(json!({"command": ["observe_property", i + 1, p]}));
        }
        Ok(mpv)
    }

    fn respawn(&mut self) -> std::io::Result<()> {
        let dir = self.sock_path.parent().map(Path::to_path_buf).unwrap_or_default();
        let fresh = Mpv::spawn(&dir, self.audio_device.clone(), self.tx.clone(), self.generation + 1)?;
        *self = fresh;
        Ok(())
    }

    fn write(&mut self, msg: Value) {
        let _ = writeln!(self.sock, "{msg}");
    }

    fn send(&mut self, cmd: &EngineCmd) {
        let c = |args: Value| json!({ "command": args });
        match cmd {
            EngineCmd::Load { load, url, start } => {
                let args = match start {
                    Some(s) => {
                        json!(["loadfile", url.as_str(), "replace", -1, format!("start={:.3}", s.as_secs_f64())])
                    }
                    None => json!(["loadfile", url.as_str(), "replace"]),
                };
                self.write(json!({ "command": args, "request_id": load.request_id() }));
                self.write(c(json!(["set_property", "pause", false])));
            }
            EngineCmd::SetPause(p) => self.write(c(json!(["set_property", "pause", p]))),
            EngineCmd::Seek(to) => self.write(c(json!(["seek", to.as_secs_f64(), "absolute"]))),
            EngineCmd::SetVolume(v) => self.write(c(json!(["set_property", "volume", v.get()]))),
            EngineCmd::Stop => self.write(c(json!(["stop"]))),
            EngineCmd::Quit => self.quit(),
        }
    }

    fn quit(&mut self) {
        self.write(json!({"command": ["quit"]}));
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline && !matches!(self.child.try_wait(), Ok(Some(_))) {
            thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.sock_path);
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The PipeWire node name of wavelet's mpv; the spectrum capture targets it.
pub const AUDIO_CLIENT_NAME: &str = "wavelet";

pub fn die_with_spawning_thread(cmd: &mut std::process::Command) {
    // SAFETY: prctl is async-signal-safe and touches no Rust state.
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
}

pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from).join("wavelet")
}
