//! The player thread against a real mpv. Ignored by default because it plays
//! audio; run it into a null sink:
//!
//!   mod=$(pactl load-module module-null-sink sink_name=wavelet_test)
//!   WAVELET_TEST_AUDIO_DEVICE=pipewire/wavelet_test cargo test --test player_live -- --ignored
//!   pactl unload-module $mod

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use wavelet::domain::{Station, StationId, Volume};
use wavelet::player::api::{Level, Now, StationView};
use wavelet::player::core::{Library, Restore};
use wavelet::player::store::Store;
use wavelet::player::{Command, Out, PlayerOpts};

/// 16-bit mono PCM sine, `secs` long.
fn write_wav(path: &Path, secs: u32) {
    let rate = 8000u32;
    let samples: Vec<i16> = (0..rate * secs)
        .map(|i| ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 8000.0) as i16)
        .collect();
    let data_len = samples.len() as u32 * 2;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    for v in [16u32, 1 | (1 << 16), rate, rate * 2, 2 | (16 << 16)] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    samples.iter().for_each(|s| b.extend_from_slice(&s.to_le_bytes()));
    std::fs::write(path, b).expect("write wav");
}

fn wait_for(rx: &Receiver<Out>, what: &str, mut f: impl FnMut(&Out) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while let Ok(out) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        if f(&out) {
            return;
        }
    }
    panic!("timed out waiting for {what}");
}

#[test]
#[ignore]
fn a_station_plays_retries_once_at_eof_then_stops_and_shutdown_reaps_mpv() {
    let device = std::env::var("WAVELET_TEST_AUDIO_DEVICE").expect("set WAVELET_TEST_AUDIO_DEVICE to a null sink");
    let dir = std::env::temp_dir().join(format!("wavelet-live-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let wav = dir.join("tone.wav");
    write_wav(&wav, 2);
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, Into::into);
    let runtime = runtime.join(format!("wavelet-live-{}", std::process::id()));

    let player = wavelet::player::spawn(PlayerOpts {
        restore: Restore { library: Library::default(), session: None, initial_volume: Volume::new(50) },
        audio_device: Some(device),
        runtime_dir: runtime.clone(),
        store: Store::at(dir.join("data"), dir.join("state")),
        http: wavelet::sources::http().expect("http"),
    })
    .expect("player starts");
    let (tx, rx) = channel();
    player.subscribe(Box::new(move |o| tx.send(o.clone()).is_ok()));

    let station = Arc::new(Station {
        id: StationId::parse("00000000-0000-0000-0000-00000000beef").expect("uuid"),
        name: "Tone FM".into(),
        stream: url::Url::from_file_path(&wav).expect("file url"),
        homepage: None,
        favicon: None,
        country_code: None,
        tags: vec![],
        codec: None,
        bitrate_kbps: None,
    });
    player.send(Command::PlayStations { stations: vec![station.clone()], start: station.id.clone() });
    let on_air = |o: &Out| matches!(o, Out::Snapshot(s) if matches!(&s.now, Some(Now::Station { stage: StationView::OnAir { .. }, .. })));
    wait_for(&rx, "on air", on_air);
    wait_for(
        &rx,
        "reconnect after the stream ended",
        |o| matches!(o, Out::Snapshot(s) if matches!(&s.now, Some(Now::Station { stage: StationView::Connecting, .. }))),
    );
    wait_for(&rx, "on air again", on_air);
    wait_for(&rx, "off the air notice", |o| matches!(o, Out::Notice(n) if n.level == Level::Error));

    player.shutdown(Duration::from_secs(3));
    let ps = std::process::Command::new("pgrep").args(["-f", &runtime.display().to_string()]).output().expect("pgrep");
    assert!(ps.stdout.is_empty(), "mpv survived shutdown: {}", String::from_utf8_lossy(&ps.stdout));
    assert!(dir.join("state/session.json").exists(), "shutdown saved the session");
    let _ = std::fs::remove_dir_all(&dir);
}
