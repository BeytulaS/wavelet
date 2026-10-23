//! LRCLIB (lrclib.net): free, keyless lyrics with line timestamps.

use std::time::Duration;

use serde_json::Value;
use url::Url;

use crate::domain::{Lyrics, Track};
use crate::sources::Http;

const BASE: &str = "https://lrclib.net/api";
/// LRCLIB's own tolerance for matching a recording by length.
const DURATION_SLACK: Duration = Duration::from_secs(5);

/// `Ok(None)` when LRCLIB has nothing for this track.
pub fn fetch(http: &Http, track: &Track) -> Result<Option<Lyrics>, String> {
    let artist = track.artists.first().map(|a| a.name.as_str()).unwrap_or_default();
    let mut q = vec![("track_name", track.title.clone()), ("artist_name", artist.to_string())];
    if let Some(album) = &track.album {
        q.push(("album_name", album.clone()));
    }
    if let Some(d) = track.duration {
        q.push(("duration", d.as_secs().to_string()));
    }
    if let Some(exact) = get(http, "get", &q)? {
        return Ok(parse(&exact));
    }
    q.retain(|(k, _)| matches!(*k, "track_name" | "artist_name"));
    let Some(list) = get(http, "search", &q)? else { return Ok(None) };
    Ok(closest(&list, track.duration).and_then(parse))
}

fn get(http: &Http, endpoint: &str, query: &[(&str, String)]) -> Result<Option<Value>, String> {
    let url = Url::parse_with_params(&format!("{BASE}/{endpoint}"), query).map_err(|e| e.to_string())?;
    let resp = http.get(url).send().map_err(|e| e.to_string())?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    resp.error_for_status().and_then(|r| r.json()).map(Some).map_err(|e| e.to_string())
}

/// The search hit whose length is nearest `want`, or the first hit when the length is unknown.
/// Hits more than `DURATION_SLACK` off are a different recording.
fn closest(list: &Value, want: Option<Duration>) -> Option<&Value> {
    let hits = list.as_array()?;
    let Some(want) = want else { return hits.first() };
    let off = |h: &Value| Duration::try_from_secs_f64(h["duration"].as_f64()?).ok().map(|d| d.abs_diff(want));
    hits.iter()
        .filter_map(|h| Some((off(h)?, h)))
        .filter(|(o, _)| *o <= DURATION_SLACK)
        .min_by_key(|(o, _)| *o)
        .map(|(_, h)| h)
}

/// Synced lyrics win over plain ones; a hit with neither is no hit.
pub fn parse(v: &Value) -> Option<Lyrics> {
    if v["instrumental"].as_bool() == Some(true) {
        return Some(Lyrics::Instrumental);
    }
    let synced = v["syncedLyrics"].as_str().map(parse_lrc).filter(|l| !l.is_empty());
    if let Some(lines) = synced {
        return Some(Lyrics::Synced(lines));
    }
    let plain = v["plainLyrics"].as_str().filter(|s| !s.trim().is_empty())?;
    Some(Lyrics::Plain(plain.lines().map(|l| l.trim_end().to_string()).collect()))
}

/// `[mm:ss.xx] text`, with several stamps allowed per line. Tag lines such as `[ar: Name]` carry no stamp and drop out.
fn parse_lrc(s: &str) -> Vec<(Duration, String)> {
    let mut out = Vec::new();
    for line in s.lines() {
        let mut rest = line.trim_start();
        let mut stamps = Vec::new();
        while let Some(tail) = rest.strip_prefix('[') {
            let Some((tag, after)) = tail.split_once(']') else { break };
            if let Some(t) = parse_stamp(tag) {
                stamps.push(t);
            }
            rest = after;
        }
        let text = rest.trim().to_string();
        out.extend(stamps.into_iter().map(|t| (t, text.clone())));
    }
    out.sort_by_key(|(t, _)| *t);
    out
}

fn parse_stamp(tag: &str) -> Option<Duration> {
    let (min, sec) = tag.split_once(':')?;
    let secs: f64 = sec.parse().ok()?;
    Duration::try_from_secs_f64(min.parse::<u64>().ok()? as f64 * 60.0 + secs).ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn synced_lyrics_parse_sorted_with_repeated_stamps_and_without_tags() {
        let lrc = "[ar: Daft Punk]\n[00:46.35] One more time\n[00:30.75] Hey\n[00:50.00][01:04.92] Chorus\n";
        assert_eq!(
            parse_lrc(lrc),
            vec![
                (MS(30_750), "Hey".into()),
                (MS(46_350), "One more time".into()),
                (MS(50_000), "Chorus".into()),
                (MS(64_920), "Chorus".into()),
            ]
        );
    }

    #[test]
    fn synced_beats_plain_and_plain_beats_nothing() {
        let both = json!({"syncedLyrics": "[00:01.00] a", "plainLyrics": "a"});
        assert_eq!(parse(&both), Some(Lyrics::Synced(vec![(MS(1000), "a".into())])));
        let plain = json!({"syncedLyrics": null, "plainLyrics": "a\nb  \n"});
        assert_eq!(parse(&plain), Some(Lyrics::Plain(vec!["a".into(), "b".into()])));
        assert_eq!(parse(&json!({"syncedLyrics": "", "plainLyrics": " "})), None);
        assert_eq!(parse(&json!({"instrumental": true})), Some(Lyrics::Instrumental));
    }

    #[test]
    fn search_picks_the_nearest_length_within_slack() {
        let list = json!([{"id": 1, "duration": 200.0}, {"id": 2, "duration": 243.4}, {"id": 3, "duration": 260.0}]);
        let pick = |s| closest(&list, Some(Duration::from_secs(s))).map(|h| h["id"].as_u64());
        assert_eq!(pick(240), Some(Some(2)));
        assert_eq!(pick(100), None);
        assert_eq!(closest(&list, None).map(|h| h["id"].as_u64()), Some(Some(1)));
    }
}
