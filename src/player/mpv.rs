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
use crate::player::core::EngineEvent;

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
    last_pos: Option<Duration>,
}

pub fn attribute(a: &mut Attribution, line: &Value) -> Option<EngineEvent> {
    let _ = (a, line);
    todo!()
}
