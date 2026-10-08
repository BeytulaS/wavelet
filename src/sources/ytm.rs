//! InnerTube (music.youtube.com/youtubei/v1), anonymous WEB_REMIX client.
//! Parsers walk the response for renderer objects instead of fixed paths, which
//! survived layout shuffles in the prototype.

use serde_json::Value;

use crate::domain::{Track, VideoId};

/// `search` with the songs filter param. Order preserved, deduped by VideoId.
pub fn search(http: &reqwest::blocking::Client, query: &str) -> Result<Vec<Track>, String> {
    todo!("POST search?prettyPrint=false; parse_search")
}

/// `next` with playlistId = RDAMVM<seed>, isAudioOnly. The seed itself is the
/// first item; callers dedupe against the queue anyway.
pub fn radio(http: &reqwest::blocking::Client, seed: &VideoId) -> Result<Vec<Track>, String> {
    todo!("POST next; parse_next")
}

/// Pure. Collect every `musicResponsiveListItemRenderer`; videoId from
/// playlistItemData, then the overlay play button, then column 0's watchEndpoint.
/// Column 1 runs split on " • " give artists (with browseIds from navigationEndpoint),
/// album when 3+ parts, duration when the last part looks like m:ss.
/// Rows whose videoId fails `VideoId::parse` are dropped.
pub fn parse_search(root: &Value) -> Vec<Track> {
    todo!()
}

/// Pure. Collect every `playlistPanelVideoRenderer`. `longBylineText` gives
/// artist • album • year; a part ending in " views" is not an album.
pub fn parse_next(root: &Value) -> Vec<Track> {
    todo!()
}

/// `=w60-h60` -> `=w544-h544` so the art is worth drawing.
fn big_thumb(url: &str) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    // fixtures/ytm_search.json (copied from the prototype), fixtures/ytm_next.json (capture once).
    //
    // parse_search_fixture_yields_songs_with_ids_titles_artists_durations
    //   asserts exact values for the first three rows, so a renderer rename fails loudly.
    // parse_search_dedupes_repeated_video_ids
    // parse_next_fixture_yields_radio_tracks_in_order
    // parse_next_drops_view_count_as_album
    // parse_on_empty_object_returns_empty_not_panic
}
