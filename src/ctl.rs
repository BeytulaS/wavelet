//! `wavelet ctl`: one command per run, for the bar widget, keybinds, and scripts.

use std::process::ExitCode;
use std::time::Duration;

use crate::daemon::client::{self, connect_or_start};
use crate::daemon::status::{OFF_JSON, project};
use crate::domain::{Playable, StationId, Volume};
use crate::player::api::{Command, Out, Snapshot, VolumeTo};
use crate::player::queue::EntryId;

#[derive(Clone, PartialEq, Debug)]
pub enum Ctl {
    Status,
    Toggle,
    Play,
    Pause,
    Next,
    Prev,
    Volume(VolumeTo),
    Station(StationId),
    Jump(EntryId),
    Quit,
}

#[derive(PartialEq, Debug)]
pub struct Usage(pub String);

pub const USAGE: &str = "\
usage: wavelet ctl <command>
  status              print the bar status as JSON
  toggle              play or pause; starts the first favorite station when nothing is queued
  play | pause
  next | prev
  volume N|+N|-N      set or change the volume (0-130)
  station ID          play a favorite station
  jump ENTRY          play a queue entry
  quit                stop the daemon";

pub fn parse(args: &[String]) -> Result<Ctl, Usage> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["status"] => Ok(Ctl::Status),
        ["toggle"] => Ok(Ctl::Toggle),
        ["play"] => Ok(Ctl::Play),
        ["pause"] => Ok(Ctl::Pause),
        ["next"] => Ok(Ctl::Next),
        ["prev"] => Ok(Ctl::Prev),
        ["quit"] => Ok(Ctl::Quit),
        ["volume", v] => parse_volume(v).map(Ctl::Volume).ok_or_else(|| Usage(format!("bad volume '{v}'"))),
        ["station", id] => {
            StationId::parse(id).map(Ctl::Station).ok_or_else(|| Usage(format!("bad station id '{id}'")))
        }
        ["jump", e] => {
            e.parse().map(|n| Ctl::Jump(EntryId::from_raw(n))).map_err(|_| Usage(format!("bad entry '{e}'")))
        }
        [] => Err(Usage("missing command".into())),
        [cmd, ..] => Err(Usage(format!("unknown command or arguments for '{cmd}'"))),
    }
}

fn parse_volume(v: &str) -> Option<VolumeTo> {
    let by = |s: &str| s.parse::<u8>().ok().map(|n| n.min(i8::MAX as u8) as i8);
    if let Some(n) = v.strip_prefix('+') {
        by(n).map(VolumeTo::By)
    } else if let Some(n) = v.strip_prefix('-') {
        by(n).map(|n| VolumeTo::By(-n))
    } else {
        v.parse::<u16>().ok().map(|n| VolumeTo::At(Volume::new(n.into())))
    }
}

pub fn run(args: &[String]) -> ExitCode {
    if let [h] = args
        && ["help", "-h", "--help"].contains(&h.as_str())
    {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let ctl = match parse(args) {
        Ok(c) => c,
        Err(Usage(why)) => {
            eprintln!("wavelet ctl: {why}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match execute(&ctl) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("wavelet ctl: {e}");
            ExitCode::from(1)
        }
    }
}

fn execute(ctl: &Ctl) -> Result<(), String> {
    let (remote, mut updates) = match connect_or_start(!matches!(ctl, Ctl::Status | Ctl::Quit)) {
        Ok(conn) => conn,
        Err(client::Error::NotRunning) => {
            if *ctl == Ctl::Status {
                println!("{OFF_JSON}");
            }
            return Ok(());
        }
        Err(e) => return Err(e.to_string()),
    };
    let Some(Out::Snapshot(snap)) = updates.next() else { return Err("the daemon closed the connection".into()) };
    if *ctl == Ctl::Status {
        println!("{}", serde_json::to_string(&project(&snap)).map_err(|e| e.to_string())?);
        return Ok(());
    }
    for cmd in plan(ctl, &snap)? {
        remote.send(cmd);
    }
    // Returning only after the daemon closed the connection means status.json
    // already says "off" and the socket is gone.
    if *ctl == Ctl::Quit && !updates.wait_closed(Duration::from_secs(10)) {
        return Err("the daemon did not stop".into());
    }
    Ok(())
}

/// The commands to send, decided against the daemon's current snapshot.
pub fn plan(ctl: &Ctl, snap: &Snapshot) -> Result<Vec<Command>, String> {
    let favorites = || -> Vec<_> {
        snap.favorites
            .items
            .iter()
            .filter_map(|i| match i {
                Playable::Station(s) => Some(s.clone()),
                Playable::Track(_) => None,
            })
            .collect()
    };
    Ok(match ctl {
        Ctl::Status => vec![],
        Ctl::Toggle => {
            let stations = favorites();
            match stations.first() {
                Some(first) if snap.now.is_none() && snap.queue.is_empty() => {
                    let start = first.id.clone();
                    vec![Command::PlayStations { stations, start }]
                }
                _ => vec![Command::TogglePause],
            }
        }
        Ctl::Play => vec![Command::SetPaused(false)],
        Ctl::Pause => vec![Command::SetPaused(true)],
        Ctl::Next => vec![Command::Next],
        Ctl::Prev => vec![Command::Prev],
        Ctl::Volume(v) => vec![Command::Volume(*v)],
        Ctl::Station(id) => {
            let stations = favorites();
            if !stations.iter().any(|s| &s.id == id) {
                return Err(format!("station {} is not a favorite", id.as_str()));
            }
            vec![Command::PlayStations { stations, start: id.clone() }]
        }
        Ctl::Jump(e) => vec![Command::Jump(*e)],
        Ctl::Quit => vec![Command::Shutdown],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::api::{Favorites, QueueRow};
    use crate::player::queue::tests::{station, track, vid};

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    fn with_favorites(items: Vec<Playable>) -> Snapshot {
        Snapshot { favorites: Favorites { items }, ..Snapshot::empty() }
    }

    #[test]
    fn volume_takes_an_absolute_or_a_signed_step() {
        assert_eq!(parse(&args("volume 60")), Ok(Ctl::Volume(VolumeTo::At(Volume::new(60)))));
        assert_eq!(parse(&args("volume +5")), Ok(Ctl::Volume(VolumeTo::By(5))));
        assert_eq!(parse(&args("volume -5")), Ok(Ctl::Volume(VolumeTo::By(-5))));
        assert_eq!(parse(&args("volume 900")), Ok(Ctl::Volume(VolumeTo::At(Volume::new(130)))), "clamped");
        assert_eq!(parse(&args("volume +200")), Ok(Ctl::Volume(VolumeTo::By(127))), "clamped to a step");
        assert!(parse(&args("volume loud")).is_err());
        assert!(parse(&args("volume")).is_err());
        assert!(parse(&args("volume +-3")).is_err());
    }

    #[test]
    fn every_command_parses_and_junk_is_usage() {
        let id = station(1).id.clone();
        assert_eq!(parse(&args(&format!("station {}", id.as_str()))), Ok(Ctl::Station(id)));
        assert_eq!(parse(&args("jump 12")), Ok(Ctl::Jump(EntryId::from_raw(12))));
        for (a, c) in [
            ("status", Ctl::Status),
            ("toggle", Ctl::Toggle),
            ("play", Ctl::Play),
            ("pause", Ctl::Pause),
            ("next", Ctl::Next),
            ("prev", Ctl::Prev),
            ("quit", Ctl::Quit),
        ] {
            assert_eq!(parse(&args(a)), Ok(c));
        }
        for bad in ["", "dance", "station nope", "jump -1", "next now"] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn toggle_with_nothing_queued_plays_the_first_favorite_station() {
        let snap = with_favorites(vec![
            Playable::Track(track(&vid('a'))),
            Playable::Station(station(2)),
            Playable::Station(station(1)),
        ]);
        assert_eq!(
            plan(&Ctl::Toggle, &snap),
            Ok(vec![Command::PlayStations { stations: vec![station(2), station(1)], start: station(2).id.clone() }])
        );
    }

    #[test]
    fn toggle_pauses_when_a_queue_exists_or_no_station_is_a_favorite() {
        let mut restored = with_favorites(vec![Playable::Station(station(2))]);
        restored.queue =
            vec![QueueRow { id: EntryId::from_raw(1), item: Playable::Track(track(&vid('a'))), from_radio: false }];
        restored.cursor = Some(0);
        assert_eq!(plan(&Ctl::Toggle, &restored), Ok(vec![Command::TogglePause]));
        let no_stations = with_favorites(vec![Playable::Track(track(&vid('a')))]);
        assert_eq!(plan(&Ctl::Toggle, &no_stations), Ok(vec![Command::TogglePause]));
    }

    #[test]
    fn station_plays_through_every_favorite_and_refuses_a_stranger() {
        let snap = with_favorites(vec![Playable::Station(station(1)), Playable::Station(station(2))]);
        assert_eq!(
            plan(&Ctl::Station(station(2).id.clone()), &snap),
            Ok(vec![Command::PlayStations { stations: vec![station(1), station(2)], start: station(2).id.clone() }])
        );
        let err = plan(&Ctl::Station(station(9).id.clone()), &snap).expect_err("not a favorite");
        assert!(err.contains("not a favorite"), "{err}");
    }

    #[test]
    fn the_simple_commands_map_one_to_one() {
        let s = Snapshot::empty();
        assert_eq!(plan(&Ctl::Play, &s), Ok(vec![Command::SetPaused(false)]));
        assert_eq!(plan(&Ctl::Pause, &s), Ok(vec![Command::SetPaused(true)]));
        assert_eq!(plan(&Ctl::Jump(EntryId::from_raw(3)), &s), Ok(vec![Command::Jump(EntryId::from_raw(3))]));
        assert_eq!(plan(&Ctl::Volume(VolumeTo::By(-5)), &s), Ok(vec![Command::Volume(VolumeTo::By(-5))]));
        assert_eq!(plan(&Ctl::Quit, &s), Ok(vec![Command::Shutdown]));
    }
}
