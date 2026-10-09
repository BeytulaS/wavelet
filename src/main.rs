use std::process::ExitCode;

use wavelet::daemon::client::connect_or_start;
use wavelet::player::api::{Level, Notice};
use wavelet::{config, ctl, daemon, sources, tui};

const USAGE: &str = "\
usage: wavelet                  open the player (starts the daemon if needed)
       wavelet ctl <command>    control the daemon; `wavelet ctl help` lists commands
       wavelet daemon           run the playback daemon in the foreground";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None => tui(),
        Some("daemon") if args.len() == 1 => daemon::run(),
        Some("ctl") => return ctl::run(&args[1..]),
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some(_) => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("wavelet: {e}");
            ExitCode::FAILURE
        }
    }
}

fn tui() -> Result<(), String> {
    let (cfg, issues) = config::load_or_init();
    let notices = issues.iter().map(|i| Notice { level: Level::Warn, text: i.to_string() }).collect();
    let (remote, updates) = connect_or_start(true).map_err(|e| e.to_string())?;
    let term = tui::Term::enter(cfg.images).map_err(|e| e.to_string())?;
    let http = sources::http()?;
    tui::run(term, cfg, notices, remote, updates, http).map_err(|e| e.to_string())
}
