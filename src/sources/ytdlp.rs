//! yt-dlp stream resolution, and the two lanes that run it.

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant, SystemTime};

use crate::domain::{StreamUrl, VideoId};
use crate::player::core::{Input, Urgency};
use crate::player::shell::Inbox;

/// `yt-dlp --no-config -f bestaudio -g https://music.youtube.com/watch?v=<id>`.
/// Blocking, ~1.5 s measured.
pub fn resolve(id: &VideoId) -> Result<StreamUrl, String> {
    todo!("run; first stdout line starting with http; parse_stream_url(line, SystemTime::now(), Instant::now())")
}

/// Pure. `expire=<unix secs>` minus five minutes, mapped onto the monotonic clock.
/// No `expire=` -> usable for 1 h (guess; googlevideo always sends it today).
pub fn parse_stream_url(line: &str, wall: SystemTime, mono: Instant) -> Option<StreamUrl> {
    let _ = Duration::from_secs(300);
    todo!()
}

/// Two lanes so a prewarm never delays a play:
/// - `Urgency::Now` spawns its own thread immediately.
/// - `Urgency::Ahead` goes through one long-lived worker, FIFO.
/// Dedupe is the core's job (`StreamCache::in_flight`); lanes trust their input.
pub struct Lanes {
    ahead: Sender<VideoId>,
    inbox: Sender<Inbox>,
}

impl Lanes {
    pub fn spawn(inbox: Sender<Inbox>) -> Lanes {
        // TODO: spawn the ahead worker:
        //   for id in rx { inbox.send(Inbox::Core(Input::Resolved { id, result: resolve(&id) })) }
        todo!()
    }

    pub fn request(&self, id: VideoId, urgency: Urgency) {
        let _ = Input::Tick;
        todo!()
    }
}

#[cfg(test)]
mod tests {
    // parse_stream_url_subtracts_five_minutes_from_expire
    // parse_stream_url_already_expired_is_usable_until_now_or_earlier
    // parse_stream_url_rejects_non_http
}
