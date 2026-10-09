//! The client end of `wavelet.sock`, shared by the TUI and `wavelet ctl`.

use std::fs::File;
use std::io::{BufRead, BufReader, Lines, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use crate::daemon::sock_path;
use crate::daemon::wire::{BuildId, ServerLine, line};
use crate::player::api::{Command, Out};

const START_WITHIN: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum Error {
    /// No daemon answered, and the caller did not ask to start one.
    NotRunning,
    Failed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotRunning => f.write_str("the wavelet daemon is not running"),
            Error::Failed(e) => f.write_str(e),
        }
    }
}

/// The sending half of a connection.
pub struct Remote {
    conn: UnixStream,
}

impl Remote {
    /// A dead daemon shows up as the end of `Updates`, so a failed write is ignored here.
    pub fn send(&self, cmd: Command) {
        let _ = (&self.conn).write_all(line(&cmd).as_bytes());
    }
}

/// The receiving half: every `Out` the daemon sends, a `Snapshot` first. Ends
/// when the daemon closes the connection.
pub struct Updates {
    conn: UnixStream,
    lines: Lines<BufReader<UnixStream>>,
}

impl Iterator for Updates {
    type Item = Out;
    fn next(&mut self) -> Option<Out> {
        loop {
            match serde_json::from_str::<ServerLine>(&self.lines.next()?.ok()?) {
                Ok(ServerLine::Out(out)) => return Some(out),
                Ok(ServerLine::Hello { .. }) | Err(_) => continue,
            }
        }
    }
}

impl Updates {
    /// Drains until the daemon closes the connection. False if it is still open after `within`.
    pub fn wait_closed(mut self, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || self.conn.set_read_timeout(Some(left)).is_err() {
                return false;
            }
            match self.lines.next() {
                None => return true,
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    return !matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut);
                }
            }
        }
    }
}

/// Connects to the daemon. With `autostart`, starts one when none answers and
/// replaces one built from a different executable. Without it, never starts
/// or stops a daemon.
pub fn connect_or_start(autostart: bool) -> Result<(Remote, Updates), Error> {
    let sock = sock_path();
    match connect(&sock) {
        Ok((remote, updates, build)) => {
            if !autostart || &build == BuildId::current() {
                return Ok((remote, updates));
            }
            eprintln!("wavelet: restarting the daemon, which runs an older build");
            remote.send(Command::Shutdown);
            if !updates.wait_closed(Duration::from_secs(10)) {
                return Err(Error::Failed("the old wavelet daemon did not stop".into()));
            }
        }
        Err(_) if !autostart => return Err(Error::NotRunning),
        Err(_) => {}
    }
    start_and_connect(&sock)
}

fn connect(sock: &Path) -> std::io::Result<(Remote, Updates, BuildId)> {
    let conn = UnixStream::connect(sock)?;
    conn.set_read_timeout(Some(START_WITHIN))?;
    let mut lines = BufReader::new(conn.try_clone()?).lines();
    let first = lines.next().ok_or_else(|| std::io::Error::other("the daemon closed the connection"))??;
    let build = match serde_json::from_str::<ServerLine>(&first) {
        Ok(ServerLine::Hello { build }) => build,
        _ => return Err(std::io::Error::other("the daemon did not say hello")),
    };
    conn.set_read_timeout(None)?;
    let updates = Updates { conn: conn.try_clone()?, lines };
    Ok((Remote { conn }, updates, build))
}

fn start_and_connect(sock: &Path) -> Result<(Remote, Updates), Error> {
    let log = log_path().ok_or_else(|| Error::Failed("no home directory for XDG paths".into()))?;
    let mut child = spawn_daemon(&log)?;
    let deadline = Instant::now() + START_WITHIN;
    loop {
        if let Ok((remote, updates, _)) = connect(sock) {
            return Ok((remote, updates));
        }
        match child.try_wait() {
            // Exit 0 means another daemon held the lock, perhaps one still on
            // its way out after a Shutdown. Try again until one answers.
            Ok(Some(st)) if st.success() => child = spawn_daemon(&log)?,
            Ok(Some(st)) => {
                return Err(Error::Failed(format!("the wavelet daemon exited ({st}); see {}", log.display())));
            }
            _ => {}
        }
        if Instant::now() > deadline {
            return Err(Error::Failed(format!("the wavelet daemon did not answer; see {}", log.display())));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn log_path() -> Option<PathBuf> {
    Some(dirs::state_dir()?.join("wavelet").join("daemon.log"))
}

fn spawn_daemon(log: &Path) -> Result<Child, Error> {
    let fail = |e: std::io::Error| Error::Failed(format!("could not start the wavelet daemon: {e}"));
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    let out = File::options().create(true).append(true).open(log).map_err(fail)?;
    let mut cmd = std::process::Command::new(exe().map_err(fail)?);
    cmd.arg("daemon").stdin(Stdio::null()).stdout(out.try_clone().map_err(fail)?).stderr(out);
    // SAFETY: setsid is async-signal-safe and touches no Rust state. A new
    // session keeps the daemon alive when the terminal that started it closes.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn().map_err(fail)
}

/// The executable's path, even after a reinstall replaced the file under the
/// running process (Linux then reports "<path> (deleted)").
pub fn exe() -> std::io::Result<PathBuf> {
    let p = std::env::current_exe()?;
    match p.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
        Some(live) if !p.exists() => Ok(PathBuf::from(live)),
        _ => Ok(p),
    }
}
