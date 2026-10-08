//! radio-browser.info through the `all.` round-robin name, as the prototype did.

use serde::Deserialize;
use url::Url;

use crate::domain::{Station, StationId};
use crate::sources::Http;

const API: &str = "https://all.api.radio-browser.info/json";

/// Wire type. Only the fields we map.
#[derive(Deserialize)]
struct RbStation {
    stationuuid: String,
    name: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    url_resolved: String,
    #[serde(default)]
    homepage: String,
    #[serde(default)]
    favicon: String,
    #[serde(default)]
    countrycode: String,
    #[serde(default)]
    tags: String,
    #[serde(default)]
    codec: String,
    #[serde(default)]
    bitrate: u32,
}

pub fn search(http: &Http, query: &str) -> Result<Vec<Station>, String> {
    let mut url = Url::parse(&format!("{API}/stations/search")).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("name", query)
        .append_pair("limit", "40")
        .append_pair("hidebroken", "true")
        .append_pair("order", "clickcount")
        .append_pair("reverse", "true");
    get(http, url.as_str())
}

pub fn top_voted(http: &Http) -> Result<Vec<Station>, String> {
    get(http, &format!("{API}/stations/topvote/40?hidebroken=true"))
}

/// Counts a play, as radio-browser asks clients to. Fire and forget.
pub fn count_click(http: &Http, id: &StationId) {
    let _ = http.get(format!("{API}/url/{}", id.as_str())).send();
}

fn get(http: &Http, url: &str) -> Result<Vec<Station>, String> {
    let body =
        http.get(url).send().and_then(|r| r.error_for_status()).and_then(|r| r.text()).map_err(|e| e.to_string())?;
    parse_stations(&body)
}

/// Pure. Drops stations whose uuid or stream url fail to parse.
pub fn parse_stations(body: &str) -> Result<Vec<Station>, String> {
    let raw: Vec<RbStation> = serde_json::from_str(body).map_err(|e| e.to_string())?;
    Ok(raw.into_iter().filter_map(station).collect())
}

fn station(r: RbStation) -> Option<Station> {
    let stream = [&r.url_resolved, &r.url].into_iter().find_map(|u| Url::parse(u.trim()).ok())?;
    Some(Station {
        id: StationId::parse(&r.stationuuid)?,
        name: r.name.trim().to_string(),
        stream,
        homepage: Url::parse(r.homepage.trim()).ok(),
        favicon: Url::parse(r.favicon.trim()).ok(),
        country_code: non_empty(r.countrycode),
        tags: split_tags(&r.tags),
        codec: non_empty(r.codec),
        bitrate_kbps: (r.bitrate > 0).then_some(r.bitrate),
    })
}

pub fn non_empty(s: String) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

pub fn split_tags(s: &str) -> Vec<String> {
    s.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../fixtures/rb_search.json");

    #[test]
    fn the_lofi_fixture_maps_every_field_of_the_first_station() {
        let s = parse_stations(FIXTURE).expect("fixture parses");
        assert_eq!(s.len(), 35);
        let first = &s[0];
        assert_eq!(first.id.as_str(), "4260f1f5-d5b3-44d5-9666-355da4da0b21");
        assert_eq!(first.name, "Lofi 24/7");
        assert_eq!(first.bitrate_kbps, Some(128));
        assert!(first.stream.scheme().starts_with("http"));
        assert!(s.iter().all(|x| !x.name.is_empty()));
    }

    #[test]
    fn rows_with_a_bad_uuid_or_url_are_dropped_and_zero_and_empty_become_none() {
        let body = r#"[
            {"stationuuid":"nope","name":"a","url_resolved":"http://a.test/"},
            {"stationuuid":"00000000-0000-0000-0000-000000000001","name":"b","url_resolved":"not a url"},
            {"stationuuid":"00000000-0000-0000-0000-000000000002","name":" c ","url_resolved":"","url":"http://c.test/",
             "codec":"","bitrate":0,"tags":"jazz, ,lofi","countrycode":""}
        ]"#;
        let s = parse_stations(body).expect("parses");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].name, "c");
        assert_eq!(s[0].stream.as_str(), "http://c.test/");
        assert_eq!((s[0].codec.clone(), s[0].bitrate_kbps, s[0].country_code.clone()), (None, None, None));
        assert_eq!(s[0].tags, vec!["jazz", "lofi"]);
    }
}
