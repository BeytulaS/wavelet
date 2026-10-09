//! `wavelet daemon`: owns mpv, the player thread, MPRIS, the session, and
//! status.json. Clients talk to it over `wavelet.sock` (see `wire`).

pub mod client;
pub mod status;
pub mod wire;

use std::fs::{DirBuilder, File};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel, sync_channel};
use std::thread;
use std::time::Duration;

use crate::config;
use crate::daemon::status::{StatusFile, project};
use crate::daemon::wire::{BuildId, ServerLine, line};
use crate::mpris;
use crate::player::api::Command;
use crate::player::core::Restore;
use crate::player::shell::runtime_dir;
use crate::player::store::{Store, import_radio_atlas};
use crate::player::{self, Out, PlayerHandle, PlayerOpts, Snapshot};
use crate::sources;

pub fn sock_path() -> PathBuf {
    runtime_dir().join("wavelet.sock")
}

/// Per-client queue depth. A client this far behind is dropped, so a stalled
/// client never blocks the player thread.
const CLIENT_BACKLOG: usize = 64;

/// Set once the stop path starts. Writers then leave their sockets open, so
/// clients see EOF only at exit, after status.json says "off".
static STOPPING: AtomicBool = AtomicBool::new(false);

enum ToStatus {
    Snap(Arc<Snapshot>),
    Off(Sender<()>),
}

/// Must run before any other thread starts, so every thread inherits the
/// blocked stop signals and only the sigwait thread receives them.
pub fn run() -> Result<(), String> {
    let signals = block_stop_signals();
    let dir = runtime_dir();
    DirBuilder::new().recursive(true).mode(0o700).create(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("daemon.lock"))
        .map_err(|e| format!("daemon.lock: {e}"))?;
    // SAFETY: flock on a descriptor we own; the lock lives as long as `lock`.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let e = std::io::Error::last_os_error();
        return if e.kind() == std::io::ErrorKind::WouldBlock { Ok(()) } else { Err(format!("daemon.lock: {e}")) };
    }

    let player = start_player()?;
    let sock = sock_path();
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock).map_err(|e| format!("{}: {e}", sock.display()))?;
    eprintln!("wavelet daemon: listening on {} (build {:?})", sock.display(), BuildId::current());

    let (status_tx, status_rx) = channel::<ToStatus>();
    let mut file = StatusFile::new(dir.join("status.json"));
    thread::Builder::new()
        .name("status".into())
        .spawn(move || {
            while let Ok(msg) = status_rx.recv() {
                let (status, ack) = match msg {
                    ToStatus::Snap(s) => (project(&s), None),
                    ToStatus::Off(ack) => {
                        (file.last().cloned().unwrap_or_else(|| project(&Snapshot::empty())).stopped(), Some(ack))
                    }
                };
                if let Err(e) = file.write(status) {
                    eprintln!("wavelet daemon: could not write status.json: {e}");
                }
                if let Some(ack) = ack {
                    let _ = ack.send(());
                    return;
                }
            }
        })
        .map_err(|e| e.to_string())?;
    let snaps = status_tx.clone();
    player.subscribe(Box::new(move |out| match out {
        Out::Snapshot(s) => snaps.send(ToStatus::Snap(s.clone())).is_ok(),
        Out::Notice(_) => true,
    }));

    let (stop_tx, stop_rx) = channel::<()>();
    let on_signal = stop_tx.clone();
    thread::Builder::new()
        .name("signals".into())
        .spawn(move || {
            let mut sig = 0;
            // SAFETY: `signals` is an initialized set, blocked in every thread.
            if unsafe { libc::sigwait(&signals, &mut sig) } == 0 {
                eprintln!("wavelet daemon: signal {sig}");
                let _ = on_signal.send(());
            }
        })
        .map_err(|e| e.to_string())?;
    let accept_player = player.clone();
    thread::Builder::new()
        .name("accept".into())
        .spawn(move || {
            for conn in listener.incoming() {
                match conn {
                    Ok(c) => {
                        if let Err(e) = serve(c, &accept_player, &stop_tx) {
                            eprintln!("wavelet daemon: client setup failed: {e}");
                        }
                    }
                    Err(e) => eprintln!("wavelet daemon: accept: {e}"),
                }
            }
        })
        .map_err(|e| e.to_string())?;

    let _ = stop_rx.recv();
    STOPPING.store(true, Ordering::SeqCst);
    player.shutdown(Duration::from_secs(3));
    let (ack, written) = channel();
    if status_tx.send(ToStatus::Off(ack)).is_ok() {
        let _ = written.recv_timeout(Duration::from_secs(2));
    }
    let _ = std::fs::remove_file(&sock);
    eprintln!("wavelet daemon: stopped");
    drop(lock);
    std::process::exit(0)
}

fn block_stop_signals() -> libc::sigset_t {
    // SAFETY: sigemptyset initializes the set before any other use.
    unsafe {
        let mut set = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
        libc::sigemptyset(set.as_mut_ptr());
        let mut set = set.assume_init();
        for s in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::sigaddset(&mut set, s);
        }
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        set
    }
}

fn start_player() -> Result<PlayerHandle, String> {
    let (cfg, issues) = config::load_or_init();
    for i in issues {
        eprintln!("wavelet daemon: {i}");
    }
    let store = Store::at_xdg().ok_or("no home directory for XDG paths")?;
    let (mut library, warning) = store.load_library();
    if let Some(w) = warning {
        eprintln!("wavelet daemon: {w}");
    }
    if !library.radio_atlas_checked {
        if let Some(dir) = dirs::data_dir().map(|d| d.join("radio-atlas"))
            && let Ok(stations) = import_radio_atlas(&dir)
        {
            let n = library.merge_stations(stations);
            eprintln!("wavelet daemon: imported {n} stations from Radio Atlas");
        }
        library.radio_atlas_checked = true;
        if let Err(e) = store.save_library(&library) {
            eprintln!("wavelet daemon: could not save favorites: {e}");
        }
    }
    let restore = Restore { library, session: store.load_session(), initial_volume: cfg.volume };
    let opts = PlayerOpts {
        restore,
        audio_device: cfg.audio_device,
        runtime_dir: runtime_dir(),
        store,
        http: sources::http()?,
    };
    let player = player::spawn(opts).map_err(|e| e.to_string())?;
    mpris::spawn(player.clone());
    Ok(player)
}

/// One client: a writer thread fed by the subscription, and a reader thread
/// that turns lines into commands.
fn serve(conn: UnixStream, player: &PlayerHandle, stop: &Sender<()>) -> std::io::Result<()> {
    let (tx, rx) = sync_channel::<Out>(CLIENT_BACKLOG);
    let mut out = conn.try_clone()?;
    thread::Builder::new().name("client-writer".into()).spawn(move || {
        let hello = ServerLine::Hello { build: BuildId::current().clone() };
        if out.write_all(line(&hello).as_bytes()).is_err() {
            return;
        }
        for o in rx {
            if out.write_all(line(&ServerLine::Out(o)).as_bytes()).is_err() {
                return;
            }
        }
        if !STOPPING.load(Ordering::SeqCst) {
            let _ = out.shutdown(std::net::Shutdown::Both);
        }
    })?;
    player.subscribe(Box::new(move |o| tx.try_send(o.clone()).is_ok()));
    let (player, stop) = (player.clone(), stop.clone());
    thread::Builder::new().name("client-reader".into()).spawn(move || {
        for l in BufReader::new(conn).lines() {
            let Ok(l) = l else { return };
            match serde_json::from_str::<Command>(&l) {
                Ok(Command::Shutdown) => {
                    let _ = stop.send(());
                }
                Ok(cmd) => player.send(cmd),
                Err(e) => eprintln!("wavelet daemon: ignoring a bad command: {e}"),
            }
        }
    })?;
    Ok(())
}
