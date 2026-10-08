use std::io::Read;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::player::shell::{AUDIO_CLIENT_NAME, die_with_spawning_thread};
use crate::tui::paint::visualizer::{Analyzer, VisBands};

pub struct Spectrum {
    pub bands: Arc<Mutex<VisBands>>,
    want: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    pub enabled: bool,
}

impl Spectrum {
    /// Spawns the capture thread. It idles until `set_wanted(true)` and
    /// respawns pw-record while wanted, since mpv's node only exists while it
    /// has audio.
    pub fn spawn(enabled: bool) -> Spectrum {
        let s = Spectrum {
            bands: VisBands::shared(),
            want: Arc::new(AtomicBool::new(false)),
            child: Arc::new(Mutex::new(None)),
            enabled,
        };
        let (bands, want, slot) = (s.bands.clone(), s.want.clone(), s.child.clone());
        std::thread::Builder::new().name("capture".into()).spawn(move || capture(bands, want, slot)).ok();
        s
    }

    pub fn set_wanted(&self, playing: bool) {
        self.want.store(self.enabled && playing, Ordering::Relaxed);
    }

    pub fn active(&self) -> bool {
        self.enabled && self.bands.try_lock().is_ok_and(|b| b.is_active)
    }

    pub fn stop(&self) {
        self.want.store(false, Ordering::Relaxed);
        if let Ok(mut slot) = self.child.lock()
            && let Some(mut c) = slot.take()
        {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn capture(bands: Arc<Mutex<VisBands>>, want: Arc<AtomicBool>, slot: Arc<Mutex<Option<Child>>>) {
    let mut an = Analyzer::new(bands, 48000.0);
    loop {
        if !want.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        let mut cmd = std::process::Command::new("pw-record");
        cmd.args([
            "--raw",
            "--target",
            AUDIO_CLIENT_NAME,
            "-P",
            "{ node.dont-fallback = true node.dont-reconnect = true }",
            "--format",
            "f32",
            "--rate",
            "48000",
            "--channels",
            "2",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
        die_with_spawning_thread(&mut cmd);
        let Ok(mut child) = cmd.spawn() else {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        let Some(mut out) = child.stdout.take() else { continue };
        if let Ok(mut s) = slot.lock() {
            *s = Some(child);
        }
        let mut buf = vec![0u8; 4096];
        let mut samples = Vec::with_capacity(1024);
        let mut carry: Vec<u8> = Vec::new();
        while want.load(Ordering::Relaxed) {
            let n = match out.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            carry.extend_from_slice(&buf[..n]);
            let whole = carry.len() / 8 * 8;
            samples.clear();
            samples.extend(carry[..whole].as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)));
            carry.drain(..whole);
            an.push(&samples);
        }
        if let Ok(mut s) = slot.lock()
            && let Some(mut c) = s.take()
        {
            let _ = c.kill();
            let _ = c.wait();
        }
        an.stop();
        std::thread::sleep(Duration::from_millis(400));
    }
}
