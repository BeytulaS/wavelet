//! yt-dlp stream resolution. The shell runs each resolve on its own thread;
//! the core's in-flight set decides how many run.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use url::Url;

use crate::domain::{StreamUrl, VideoId};

/// googlevideo URLs stop working at `expire=`; stop using them a bit earlier.
const EXPIRY_MARGIN: Duration = Duration::from_secs(300);
/// Used when a URL has no `expire=` (googlevideo always sends it today).
const DEFAULT_LIFETIME: Duration = Duration::from_secs(3600);

/// `yt-dlp --no-config -f bestaudio -g https://music.youtube.com/watch?v=<id>`.
/// Blocking, ~1.5 s measured.
pub fn resolve(id: &VideoId) -> Result<StreamUrl, String> {
    let out = Command::new("yt-dlp")
        .args(["--no-config", "-f", "bestaudio", "-g"])
        .arg(format!("https://music.youtube.com/watch?v={}", id.as_str()))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("yt-dlp: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout.lines().find(|l| l.starts_with("http"));
    match line.and_then(|l| parse_stream_url(l, SystemTime::now(), Instant::now())) {
        Some(u) => Ok(u),
        None => {
            let err = String::from_utf8_lossy(&out.stderr);
            let last = err.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("no stream url");
            Err(last.trim_start_matches("ERROR: ").to_string())
        }
    }
}

/// Pure. `expire=<unix secs>` minus five minutes, mapped onto the monotonic clock.
pub fn parse_stream_url(line: &str, wall: SystemTime, mono: Instant) -> Option<StreamUrl> {
    let url = Url::parse(line.trim()).ok().filter(|u| matches!(u.scheme(), "http" | "https"))?;
    let expire = url.query_pairs().find(|(k, _)| k == "expire").and_then(|(_, v)| v.parse::<u64>().ok());
    let usable_until = match expire {
        Some(secs) => {
            let deadline = UNIX_EPOCH + Duration::from_secs(secs);
            let left = deadline.duration_since(wall).unwrap_or_default().saturating_sub(EXPIRY_MARGIN);
            mono + left
        }
        None => mono + DEFAULT_LIFETIME,
    };
    Some(StreamUrl { url, usable_until })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn expire_minus_five_minutes_lands_on_the_monotonic_clock() {
        let mono = Instant::now();
        let u =
            parse_stream_url("https://rr1.googlevideo.com/videoplayback?expire=1000000&id=x", at(1000000 - 3600), mono)
                .expect("parses");
        assert_eq!(u.usable_until, mono + Duration::from_secs(3600 - 300));
    }

    #[test]
    fn an_already_expired_url_is_unusable_now() {
        let mono = Instant::now();
        let u = parse_stream_url("https://x.test/v?expire=100", at(5000), mono).expect("parses");
        assert!(u.usable_until <= mono);
    }

    #[test]
    fn non_http_lines_are_rejected() {
        let now = (SystemTime::now(), Instant::now());
        assert!(parse_stream_url("WARNING: something", now.0, now.1).is_none());
        assert!(parse_stream_url("file:///etc/passwd", now.0, now.1).is_none());
    }
}
