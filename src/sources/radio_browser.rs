//! radio-browser.info. Host from all.api.radio-browser.info (pick one at random,
//! as the docs ask; v1 may just use the `all.` name like the prototype did).

use serde::Deserialize;

use crate::domain::{Station, StationId};

/// Wire type. Only the fields we map.
#[derive(Deserialize)]
struct RbStation {
    stationuuid: String,
    name: String,
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

/// `/json/stations/search?name=..&limit=40&hidebroken=true&order=clickcount&reverse=true`
pub fn search(http: &reqwest::blocking::Client, query: &str) -> Result<Vec<Station>, String> {
    todo!()
}

/// `/json/stations/topvote/40?hidebroken=true`
pub fn top_voted(http: &reqwest::blocking::Client) -> Result<Vec<Station>, String> {
    todo!()
}

/// Fire and forget.
pub fn count_click(http: &reqwest::blocking::Client, id: &StationId) {
    todo!("GET /json/url/<uuid>, ignore result")
}

/// Pure. Drops stations whose uuid or url fail to parse. Trims names.
pub fn parse_stations(body: &str) -> Result<Vec<Station>, String> {
    let _: Vec<RbStation>;
    todo!()
}

#[cfg(test)]
mod tests {
    // fixtures/rb_search.json (capture once from a "lofi" search).
    //
    // parse_stations_fixture_maps_uuid_name_url_codec_bitrate_tags
    // parse_stations_drops_rows_with_bad_uuid_or_url
    // parse_stations_zero_bitrate_is_none_and_empty_codec_is_none
}
