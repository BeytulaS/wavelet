//! Attribution of mpv's IPC lines to our loads. Pure; runs on the socket
//! reader thread and is tested on a recorded transcript.
//!
//! mpv 0.41 facts this relies on: `loadfile` sent with `request_id = LoadId`
//! is answered with `{"data":{"playlist_entry_id":N},"request_id":R}` before the
//! matching `start-file`; `start-file` and `end-file` carry `playlist_entry_id`;
//! a replaced file sends `end-file reason=stop` after the new loadfile's reply;
//! `time-pos` carries no id, so it belongs to the last started entry.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;

use crate::domain::LoadId;
use crate::player::core::{EndReason, EngineEvent};

#[derive(Debug, Default)]
pub struct Attribution {
    bound: HashMap<i64, LoadId>,
    /// The entry of the last `start-file`, until its `end-file`.
    playing: Option<Playing>,
}

#[derive(Debug)]
struct Playing {
    entry: i64,
    load: LoadId,
    began: bool,
    last_pos: Duration,
    /// Reported before first audio; handed over in `Began`.
    duration: Option<Duration>,
    icy: Option<String>,
}

fn seconds(v: &Value) -> Option<Duration> {
    v.as_f64().filter(|s| s.is_finite()).map(|s| Duration::from_secs_f64(s.max(0.0)))
}

pub fn attribute(a: &mut Attribution, line: &Value) -> Option<EngineEvent> {
    let entry = line["playlist_entry_id"].as_i64();
    if let (Some(rid), Some(e)) = (line["request_id"].as_u64(), line["data"]["playlist_entry_id"].as_i64()) {
        a.bound.insert(e, LoadId::from_request_id(rid));
        return None;
    }
    match line["event"].as_str()? {
        "start-file" => {
            let e = entry?;
            let load = *a.bound.get(&e)?;
            a.playing =
                Some(Playing { entry: e, load, began: false, last_pos: Duration::ZERO, duration: None, icy: None });
            None
        }
        "end-file" => {
            let e = entry?;
            let load = a.bound.remove(&e)?;
            if a.playing.as_ref().is_some_and(|p| p.entry == e) {
                a.playing = None;
            }
            let reason = match line["reason"].as_str()? {
                "eof" => EndReason::Eof,
                "error" => EndReason::Error(line["file_error"].as_str().unwrap_or("playback error").to_string()),
                _ => return None,
            };
            Some(EngineEvent::Ended { load, reason })
        }
        "property-change" => {
            let p = a.playing.as_mut()?;
            let data = &line["data"];
            match line["name"].as_str()? {
                "time-pos" => {
                    let pos = seconds(data)?;
                    if !p.began {
                        if pos.is_zero() {
                            return None;
                        }
                        p.began = true;
                        p.last_pos = pos;
                        return Some(EngineEvent::Began { load: p.load, duration: p.duration, icy: p.icy.take() });
                    }
                    let last = std::mem::replace(&mut p.last_pos, pos);
                    (pos < last || pos.as_secs() != last.as_secs())
                        .then_some(EngineEvent::Position { load: p.load, pos })
                }
                "duration" => {
                    let d = seconds(data);
                    if p.began {
                        d.map(|duration| EngineEvent::Duration { load: p.load, duration })
                    } else {
                        p.duration = d;
                        None
                    }
                }
                "metadata/by-key/icy-title" => {
                    let title = data.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
                    if p.began {
                        Some(EngineEvent::Icy { load: p.load, title })
                    } else {
                        p.icy = title;
                        None
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(lines: impl IntoIterator<Item = Value>) -> Vec<EngineEvent> {
        let mut a = Attribution::default();
        lines.into_iter().filter_map(|l| attribute(&mut a, &l)).collect()
    }

    fn load(n: u64) -> LoadId {
        LoadId::from_request_id(n)
    }

    /// Recorded by tools/record_mpv.py: two loads of a 1 s sine (the second
    /// replaces the first mid-play and reaches eof), then a missing file.
    #[test]
    fn the_recorded_transcript_attributes_to_the_right_loads() {
        let text = include_str!("../../fixtures/mpv_transcript.jsonl");
        let events = run(text.lines().map(|l| serde_json::from_str(l).expect("transcript line")));
        let d = Some(Duration::from_secs_f64(0.998458));
        assert_eq!(
            events,
            vec![
                EngineEvent::Began { load: load(101), duration: d, icy: None },
                EngineEvent::Began { load: load(102), duration: d, icy: None },
                EngineEvent::Ended { load: load(102), reason: EndReason::Eof },
                EngineEvent::Ended { load: load(103), reason: EndReason::Error("loading failed".into()) },
            ],
            "the replaced load's end-file stop and every pre-audio time-pos are dropped"
        );
    }

    #[test]
    fn positions_are_throttled_to_whole_seconds_and_seeks_back_pass() {
        let tp = |s: f64| json!({"event":"property-change","name":"time-pos","data":s});
        let events = run([
            json!({"request_id":7,"data":{"playlist_entry_id":1}}),
            json!({"event":"start-file","playlist_entry_id":1}),
            json!({"event":"property-change","name":"metadata/by-key/icy-title","data":"Artist - Song"}),
            tp(0.0),
            tp(1.2),
            tp(1.5),
            tp(2.1),
            tp(0.4),
            json!({"event":"property-change","name":"metadata/by-key/icy-title","data":""}),
        ]);
        assert_eq!(
            events,
            vec![
                EngineEvent::Began { load: load(7), duration: None, icy: Some("Artist - Song".into()) },
                EngineEvent::Position { load: load(7), pos: Duration::from_secs_f64(2.1) },
                EngineEvent::Position { load: load(7), pos: Duration::from_secs_f64(0.4) },
                EngineEvent::Icy { load: load(7), title: None },
            ]
        );
    }

    #[test]
    fn events_for_an_unbound_entry_are_dropped() {
        let events = run([
            json!({"event":"start-file","playlist_entry_id":9}),
            json!({"event":"property-change","name":"time-pos","data":3.0}),
            json!({"event":"end-file","reason":"eof","playlist_entry_id":9}),
        ]);
        assert!(events.is_empty());
    }
}
