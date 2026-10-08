//! yt-dlp stream resolution. The shell runs each resolve on its own thread;
//! the core's in-flight set decides how many run.

use std::time::{Instant, SystemTime};

use crate::domain::{StreamUrl, VideoId};

/// `yt-dlp --no-config -f bestaudio -g https://music.youtube.com/watch?v=<id>`.
/// Blocking, ~1.5 s measured.
pub fn resolve(id: &VideoId) -> Result<StreamUrl, String> {
    let _ = id;
    todo!()
}

/// Pure. `expire=<unix secs>` minus five minutes, mapped onto the monotonic clock.
pub fn parse_stream_url(line: &str, wall: SystemTime, mono: Instant) -> Option<StreamUrl> {
    let _ = (line, wall, mono);
    todo!()
}
