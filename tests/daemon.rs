use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Private XDG directories, so the test never meets a real wavelet daemon,
/// its config, or its library. The runtime dir is short because sun_path
/// holds 108 bytes.
struct Env {
    runtime: PathBuf,
    home: PathBuf,
}

impl Env {
    fn new() -> Env {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let id = format!("{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let base = ["/run/user", &unsafe { libc::getuid() }.to_string()].iter().collect::<PathBuf>();
        let base = if base.is_dir() { base } else { std::env::temp_dir() };
        let runtime = base.join(format!("wl-{id}"));
        let home = std::env::temp_dir().join(format!("wavelet-daemon-test-{id}"));
        for d in [&runtime, &home] {
            let _ = std::fs::remove_dir_all(d);
            std::fs::create_dir_all(d).expect("temp dir");
        }
        Env { runtime, home }
    }

    fn wavelet(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_wavelet"));
        c.args(args)
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("XDG_CONFIG_HOME", self.home.join("config"))
            .env("XDG_DATA_HOME", self.home.join("data"))
            .env("XDG_STATE_HOME", self.home.join("state"))
            // No session bus: the test daemon must not claim the MPRIS name.
            .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", self.runtime.join("no-bus").display()))
            .env("WAVELET_AUDIO_DEVICE", "null");
        c
    }

    fn ctl(&self, args: &[&str]) -> Output {
        let mut all = vec!["ctl"];
        all.extend(args);
        self.wavelet(&all).output().expect("run wavelet ctl")
    }

    fn dir(&self) -> PathBuf {
        self.runtime.join("wavelet")
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.runtime);
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn wait_exit(child: &mut Child, within: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if let Some(st) = child.try_wait().expect("try_wait") {
            return Some(st);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

fn wait_for(what: &str, within: Duration, f: impl Fn() -> bool) {
    let deadline = Instant::now() + within;
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn status_json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).expect("status.json")).expect("status.json is JSON")
}

#[test]
fn one_daemon_runs_answers_ctl_and_quits_cleanly() {
    let env = Env::new();
    let sock = env.dir().join("wavelet.sock");
    let status = env.dir().join("status.json");

    let mut daemon = env.wavelet(&["daemon"]).spawn().expect("start the daemon");
    let guard = KillOnDrop(&mut daemon);
    wait_for("the socket", Duration::from_secs(10), || sock.exists());

    let mut second = env.wavelet(&["daemon"]).spawn().expect("start a second daemon");
    let st = wait_exit(&mut second, Duration::from_secs(5)).expect("the second daemon exits at once");
    assert!(st.success(), "the second daemon exits 0, got {st}");
    assert!(guard.0.try_wait().expect("try_wait").is_none(), "the first daemon still runs");

    let out = env.ctl(&["status"]);
    assert!(out.status.success(), "{out:?}");
    let printed: serde_json::Value = serde_json::from_slice(&out.stdout).expect("ctl status prints JSON");
    assert_eq!(printed["state"], "idle", "{printed}");
    wait_for("status.json", Duration::from_secs(5), || status.exists());
    assert_eq!(status_json(&status)["state"], "idle");

    let out = env.ctl(&["quit"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(status_json(&status)["state"], "off", "written before ctl quit returns");
    assert!(!sock.exists(), "the socket is removed");
    let st = wait_exit(guard.0, Duration::from_secs(5)).expect("the daemon exits after quit");
    assert!(st.success(), "the daemon exits 0, got {st}");

    let out = env.ctl(&["status"]);
    let printed: serde_json::Value = serde_json::from_slice(&out.stdout).expect("ctl status prints JSON");
    assert_eq!(printed["state"], "off", "{printed}");
    assert!(!sock.exists(), "ctl status never starts a daemon");
    assert_eq!(env.ctl(&["volume", "loud"]).status.code(), Some(2), "usage errors exit 2");
}

#[test]
fn ctl_status_with_no_daemon_offers_the_saved_favorite_stations() {
    let env = Env::new();
    let library = env.home.join("data/wavelet/library.json");
    std::fs::create_dir_all(library.parent().expect("parent")).expect("data dir");
    let station = serde_json::json!({
        "id": "8b700f14-99e7-4f77-84eb-3ff9d0ad69a0",
        "name": "Radio The Voice",
        "stream": "https://example.com/stream",
        "homepage": null,
        "favicon": null,
        "country_code": null,
        "tags": [],
        "codec": null,
        "bitrate_kbps": null,
    });
    let file = serde_json::json!({ "version": 1, "tracks": [], "stations": [station], "radio_atlas_checked": true });
    std::fs::write(&library, file.to_string()).expect("library.json");

    let out = env.ctl(&["status"]);
    assert!(out.status.success(), "{out:?}");
    let printed: serde_json::Value = serde_json::from_slice(&out.stdout).expect("ctl status prints JSON");
    assert_eq!(printed["state"], "off", "{printed}");
    assert_eq!(
        printed["stations"],
        serde_json::json!([{ "id": "8b700f14-99e7-4f77-84eb-3ff9d0ad69a0", "name": "Radio The Voice" }])
    );
    assert!(!env.dir().join("wavelet.sock").exists(), "ctl status never starts a daemon");
}

struct KillOnDrop<'a>(&'a mut Child);

impl Drop for KillOnDrop<'_> {
    fn drop(&mut self) {
        if let Ok(None) = self.0.try_wait() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
