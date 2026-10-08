//! InnerTube (music.youtube.com/youtubei/v1), anonymous WEB_REMIX client.

use std::collections::HashSet;
use std::time::Duration;

use serde_json::{Value, json};
use url::Url;

use crate::domain::{ArtistRef, Track, VideoId};
use crate::sources::Http;

const BASE: &str = "https://music.youtube.com/youtubei/v1";
const SONGS: &str = "EgWKAQIIAWoKEAkQBRAKEAMQBA%3D%3D";

fn post(http: &Http, endpoint: &str, mut body: Value) -> Result<Value, String> {
    body["context"] = json!({"client": {"clientName": "WEB_REMIX", "clientVersion": "1.20250101.01.00", "hl": "en"}});
    http.post(format!("{BASE}/{endpoint}?prettyPrint=false"))
        .header("Content-Type", "application/json")
        .header("Origin", "https://music.youtube.com")
        .body(body.to_string())
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json())
        .map_err(|e| e.to_string())
}

pub fn search(http: &Http, query: &str) -> Result<Vec<Track>, String> {
    post(http, "search", json!({"query": query, "params": SONGS})).map(|v| parse_search(&v))
}

pub fn radio(http: &Http, seed: &VideoId) -> Result<Vec<Track>, String> {
    let body = json!({"videoId": seed.as_str(), "playlistId": format!("RDAMVM{}", seed.as_str()), "isAudioOnly": true});
    post(http, "next", body).map(|v| parse_next(&v))
}

fn collect<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                if k == key {
                    out.push(x);
                } else {
                    collect(x, key, out);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect(x, key, out)),
        _ => {}
    }
}

fn runs(v: &Value) -> &[Value] {
    v["runs"].as_array().map_or(&[], Vec::as_slice)
}

fn text(v: &Value) -> String {
    runs(v).iter().filter_map(|r| r["text"].as_str()).collect()
}

fn segments(v: &Value) -> Vec<&[Value]> {
    runs(v).split(|r| r["text"].as_str() == Some(" • ")).collect()
}

fn browse_id(run: &Value) -> Option<&str> {
    run["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str()
}

fn artists(segs: &[&[Value]]) -> Vec<ArtistRef> {
    let Some(first) = segs.first() else { return vec![] };
    first
        .iter()
        .filter_map(|r| {
            let name = r["text"].as_str()?.trim();
            let joiner = matches!(name, "&" | "," | "and" | "x" | "");
            (!joiner).then(|| ArtistRef { name: name.to_string(), browse_id: browse_id(r).map(str::to_string) })
        })
        .collect()
}

/// An album run links to an `MPRE…` browse id; a view count or year never does.
fn album(segs: &[&[Value]]) -> Option<String> {
    segs.iter().skip(1).flat_map(|s| s.iter()).find(|r| browse_id(r).is_some_and(|b| b.starts_with("MPRE")))?["text"]
        .as_str()
        .map(str::to_string)
}

fn parse_clock(s: &str) -> Option<Duration> {
    let parts: Vec<u64> = s.trim().split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    Some(Duration::from_secs(parts.iter().fold(0, |acc, p| acc * 60 + p)))
}

fn thumb(thumbnails: &Value) -> Option<Url> {
    let url = thumbnails.as_array()?.last()?["url"].as_str()?;
    Url::parse(&big_thumb(url)).ok()
}

/// `=w60-h60` -> `=w544-h544` so the art is worth drawing.
fn big_thumb(url: &str) -> String {
    if let Some(p) = url.rfind("=w") {
        let after_w = url[p + 2..].trim_start_matches(|c: char| c.is_ascii_digit());
        if let Some(h) = after_w.strip_prefix("-h") {
            let tail = h.trim_start_matches(|c: char| c.is_ascii_digit());
            return format!("{}=w544-h544{}", &url[..p], tail);
        }
    }
    url.to_string()
}

pub fn parse_search(root: &Value) -> Vec<Track> {
    let mut rows = Vec::new();
    collect(root, "musicResponsiveListItemRenderer", &mut rows);
    let mut seen = HashSet::new();
    rows.into_iter()
        .filter_map(|r| {
            let col = |i: usize| &r["flexColumns"][i]["musicResponsiveListItemFlexColumnRenderer"]["text"];
            let id = r["playlistItemData"]["videoId"]
                .as_str()
                .or_else(|| {
                    r["overlay"]["musicItemThumbnailOverlayRenderer"]["content"]["musicPlayButtonRenderer"]
                        ["playNavigationEndpoint"]["watchEndpoint"]["videoId"]
                        .as_str()
                })
                .or_else(|| runs(col(0)).first()?["navigationEndpoint"]["watchEndpoint"]["videoId"].as_str())
                .and_then(VideoId::parse)?;
            let segs = segments(col(1));
            let duration =
                segs.last().and_then(|s| parse_clock(&s.iter().filter_map(|r| r["text"].as_str()).collect::<String>()));
            Some(Track {
                title: text(col(0)),
                artists: artists(&segs),
                album: album(&segs),
                duration,
                art: thumb(&r["thumbnail"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"]),
                id,
            })
        })
        .filter(|t| seen.insert(t.id.clone()))
        .collect()
}

pub fn parse_next(root: &Value) -> Vec<Track> {
    let mut rows = Vec::new();
    collect(root, "playlistPanelVideoRenderer", &mut rows);
    let mut seen = HashSet::new();
    rows.into_iter()
        .filter_map(|r| {
            let segs = segments(&r["longBylineText"]);
            Some(Track {
                id: r["videoId"].as_str().and_then(VideoId::parse)?,
                title: text(&r["title"]),
                artists: artists(&segs),
                album: album(&segs),
                duration: parse_clock(&text(&r["lengthText"])),
                art: thumb(&r["thumbnail"]["thumbnails"]),
            })
        })
        .filter(|t| seen.insert(t.id.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let path = format!("{}/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("fixture json")
    }

    fn names(t: &Track) -> Vec<&str> {
        t.artists.iter().map(|a| a.name.as_str()).collect()
    }

    #[test]
    fn search_fixture_yields_songs_with_exact_fields() {
        let tracks = parse_search(&fixture("ytm_search.json"));
        assert_eq!(tracks.len(), 20);
        let t = &tracks[0];
        assert_eq!(t.id.as_str(), "Jb6gcoR266U");
        assert_eq!(t.title, "Around the World");
        assert_eq!(names(t), ["Daft Punk"]);
        assert_eq!(t.artists[0].browse_id.as_deref(), Some("UCRr1xG_2WIDs18a6cIiCxeA"));
        assert_eq!(t.album.as_deref(), Some("Homework"));
        assert_eq!(t.duration, Some(Duration::from_secs(7 * 60 + 10)));
        assert!(t.art.as_ref().is_some_and(|u| u.as_str().contains("w544-h544")));
        assert_eq!(tracks[1].id.as_str(), "bmjsUsS8QuM");
        assert_eq!(tracks[2].title, "Around the World (Radio Edit)");
        assert_eq!(tracks[2].duration, Some(Duration::from_secs(4 * 60 + 2)));
    }

    #[test]
    fn next_fixture_yields_radio_tracks_in_order_with_joined_artists() {
        let tracks = parse_next(&fixture("ytm_next.json"));
        assert_eq!(tracks.len(), 50);
        assert_eq!(tracks[0].id.as_str(), "Jb6gcoR266U", "the seed comes first");
        assert_eq!(tracks[1].title, "One More Time (Radio Edit)");
        assert_eq!(tracks[1].album.as_deref(), Some("One More Time"));
        assert_eq!(tracks[1].duration, Some(Duration::from_secs(5 * 60 + 21)));
        assert_eq!(names(&tracks[3]), ["Gorillaz", "De La Soul"]);
        assert_eq!(tracks[3].album.as_deref(), Some("Demon Days"));
    }

    #[test]
    fn a_view_count_is_not_an_album_and_duplicate_ids_collapse() {
        let row = |id: &str| {
            json!({"playlistPanelVideoRenderer": {
                "videoId": id, "title": {"runs": [{"text": "t"}]},
                "longBylineText": {"runs": [{"text": "Someone"}, {"text": " • "}, {"text": "137M views"}]},
                "lengthText": {"runs": [{"text": "4:02"}]}}})
        };
        let tracks = parse_next(&json!({"contents": [row("K0HSD_i2DvA"), row("K0HSD_i2DvA"), row("short")]}));
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].album, None);
        assert_eq!(names(&tracks[0]), ["Someone"]);
    }

    #[test]
    fn empty_input_returns_nothing() {
        assert!(parse_search(&json!({})).is_empty());
        assert!(parse_next(&Value::Null).is_empty());
    }

    #[test]
    fn clock_strings_parse_and_garbage_does_not() {
        assert_eq!(parse_clock("1:02:03"), Some(Duration::from_secs(3723)));
        assert_eq!(parse_clock("137M views"), None);
        assert_eq!(parse_clock("42"), None);
    }
}
