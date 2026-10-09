use std::path::{Path, PathBuf};

use ratatui_image::picker::ProtocolType;

use crate::domain::Volume;
use crate::tui::LayoutKind;
use crate::tui::input::{Action, KeySeq};
use crate::tui::look::ThemeMode;
use crate::tui::paint::cover::parse_protocol;

#[derive(Debug, Clone)]
pub struct Config {
    pub layout: LayoutKind,
    pub theme: ThemeMode,
    pub audio_device: Option<String>,
    pub volume: Volume,
    pub spectrum: bool,
    /// `None` detects the terminal's image protocol.
    pub images: Option<ProtocolType>,
    pub keys: Vec<(KeySeq, Action)>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            layout: LayoutKind::Myx,
            theme: ThemeMode::Omarchy,
            audio_device: None,
            volume: Volume::new(70),
            spectrum: true,
            images: None,
            keys: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub file: PathBuf,
    pub line: Option<usize>,
    pub reason: String,
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name =
            self.file.file_name().map_or_else(|| self.file.display().to_string(), |n| n.to_string_lossy().into());
        match self.line {
            Some(l) => write!(f, "{name}:{l}: {}", self.reason),
            None => write!(f, "{name}: {}", self.reason),
        }
    }
}

const DEFAULT_CONFIG: &str = r#"# wavelet settings. Uncomment a line to change it; a bad value falls back to
# its default with a warning.

# Screen layout: "myx", "stage", or "list". `w` cycles while running.
# layout = "myx"

# Colors: "omarchy", "album", "omarchy+album", or a built-in theme
# ("tokyonight", "catppuccin", "rosepine", "gruvbox"). `t` cycles.
# theme = "omarchy"

# mpv --audio-device, e.g. "pipewire/alsa_output.usb-…". Empty means the default.
# audio_device = ""

# Starting volume, 0-130, when no session is restored.
# volume = 70

# Spectrum analyzer (needs pw-record). `v` toggles.
# spectrum = true

# Album art: "auto", "kitty", "sixel", "iterm2", or "halfblocks" (text cells).
# Inside herdr, auto uses halfblocks because herdr shows images only when its
# own terminal speaks kitty; set "kitty" if yours is kitty or ghostty.
# images = "auto"
"#;

const DEFAULT_KEYMAP: &str = r#"# wavelet key overrides. Keys are space-separated sequences ("g q", "C-d",
# "space", "enter", "tab"); values are action names. "unbind" removes a default.
# Press ? in wavelet for every action and its keys.

[keys]
# "x" = "toggle-pause"
# "g g" = "unbind"
"#;

pub fn config_dir() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("wavelet"))
}

pub fn load_or_init() -> (Config, Vec<Issue>) {
    let Some(dir) = config_dir() else { return (Config::default(), vec![]) };
    let mut issues = Vec::new();
    let cfg_path = dir.join("config.toml");
    let keys_path = dir.join("keymap.toml");
    for (path, default) in [(&cfg_path, DEFAULT_CONFIG), (&keys_path, DEFAULT_KEYMAP)] {
        if !path.exists()
            && let Err(e) = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(path, default))
        {
            issues.push(Issue { file: path.clone(), line: None, reason: format!("could not write defaults: {e}") });
        }
    }
    let mut cfg = std::fs::read_to_string(&cfg_path)
        .map_or_else(|_| Config::default(), |t| parse_config(&t, &cfg_path, &mut issues));
    if let Ok(text) = std::fs::read_to_string(&keys_path) {
        cfg.keys = parse_keymap(&text, &keys_path, &mut issues);
    }
    if let Ok(dev) = std::env::var("WAVELET_AUDIO_DEVICE") {
        cfg.audio_device = Some(dev).filter(|d| !d.is_empty());
    }
    (cfg, issues)
}

fn line_of(text: &str, key: &str) -> Option<usize> {
    text.lines()
        .position(|l| {
            let l = l.trim_start();
            let k = l.split('=').next().unwrap_or("").trim().trim_matches('"');
            !l.starts_with('#') && k == key
        })
        .map(|i| i + 1)
}

fn show(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => format!("\"{s}\""),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        other => format!("a {}", other.type_str()),
    }
}

pub fn parse_config(text: &str, file: &Path, issues: &mut Vec<Issue>) -> Config {
    let mut cfg = Config::default();
    let table: toml::Table = match toml::from_str(text) {
        Ok(t) => t,
        Err(e) => {
            let reason = format!("not valid TOML, using all defaults: {}", e.message());
            issues.push(Issue { file: file.into(), line: None, reason });
            return cfg;
        }
    };
    for (key, value) in &table {
        let mut bad = |reason: String| issues.push(Issue { file: file.into(), line: line_of(text, key), reason });
        match key.as_str() {
            "layout" => match value.as_str().and_then(LayoutKind::parse) {
                Some(l) => cfg.layout = l,
                None => bad(format!("layout must be \"myx\", \"stage\", or \"list\", not {}", show(value))),
            },
            "theme" => match value.as_str().and_then(ThemeMode::parse) {
                Some(t) => cfg.theme = t,
                None => bad(format!("unknown theme {}; using omarchy", show(value))),
            },
            "audio_device" => match value.as_str() {
                Some(d) => cfg.audio_device = Some(d.to_string()).filter(|d| !d.is_empty()),
                None => bad("audio_device must be a string".into()),
            },
            "volume" => match value.as_integer().filter(|v| (0..=Volume::MAX as i64).contains(v)) {
                Some(v) => cfg.volume = Volume::new(v as i32),
                None => bad(format!("volume must be 0-{}, not {}", Volume::MAX, show(value))),
            },
            "spectrum" => match value.as_bool() {
                Some(b) => cfg.spectrum = b,
                None => bad("spectrum must be true or false".into()),
            },
            "images" => match value.as_str().map(|v| (v, parse_protocol(v))) {
                Some(("auto", _)) => cfg.images = None,
                Some((_, Some(p))) => cfg.images = Some(p),
                _ => bad(format!(
                    "images must be \"auto\", \"kitty\", \"sixel\", \"iterm2\", or \"halfblocks\", not {}",
                    show(value)
                )),
            },
            other => bad(format!("unknown key '{other}'")),
        }
    }
    cfg
}

pub fn parse_keymap(text: &str, file: &Path, issues: &mut Vec<Issue>) -> Vec<(KeySeq, Action)> {
    let table: toml::Table = match toml::from_str(text) {
        Ok(t) => t,
        Err(e) => {
            issues.push(Issue { file: file.into(), line: None, reason: format!("not valid TOML: {}", e.message()) });
            return vec![];
        }
    };
    let Some(keys) = table.get("keys").and_then(|k| k.as_table()) else { return vec![] };
    keys.iter()
        .filter_map(|(seq, action)| {
            let parsed = KeySeq::parse(seq).and_then(|s| {
                let name = action.as_str().ok_or("the action must be a string")?;
                Ok((s, name.parse::<Action>()?))
            });
            parsed.map_err(|reason| issues.push(Issue { file: file.into(), line: line_of(text, seq), reason })).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bad_key_falls_back_alone_and_reports_its_line() {
        let text = "layout = \"stage\"\nvolume = 900\nspectrum = false\nshuffle = true\n";
        let mut issues = vec![];
        let cfg = parse_config(text, Path::new("config.toml"), &mut issues);
        assert_eq!(cfg.layout, LayoutKind::Stage);
        assert_eq!(cfg.volume, Volume::new(70), "bad volume keeps the default");
        assert!(!cfg.spectrum);
        assert_eq!(issues.len(), 2, "{issues:?}");
        let text: Vec<String> = issues.iter().map(|i| i.to_string()).collect();
        assert!(text.iter().any(|t| t.starts_with("config.toml:2: volume must be")), "{text:?}");
        assert!(text.iter().any(|t| t == "config.toml:4: unknown key 'shuffle'"), "{text:?}");
    }

    #[test]
    fn images_accepts_auto_or_a_protocol_and_rejects_the_rest() {
        let parse = |text: &str| {
            let mut issues = vec![];
            (parse_config(text, Path::new("c"), &mut issues).images, issues.len())
        };
        assert_eq!(parse("images = \"kitty\""), (Some(ProtocolType::Kitty), 0));
        assert_eq!(parse("images = \"auto\""), (None, 0));
        assert_eq!(parse("images = \"png\""), (None, 1));
    }

    #[test]
    fn the_written_defaults_parse_cleanly() {
        let mut issues = vec![];
        parse_config(DEFAULT_CONFIG, Path::new("c"), &mut issues);
        assert_eq!(parse_keymap(DEFAULT_KEYMAP, Path::new("k"), &mut issues), vec![]);
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn keymap_overrides_parse_and_bad_entries_become_issues() {
        let text = "[keys]\n\"x\" = \"toggle-pause\"\n\"g g\" = \"unbind\"\n\"y\" = \"fly\"\n";
        let mut issues = vec![];
        let keys = parse_keymap(text, Path::new("keymap.toml"), &mut issues);
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&(KeySeq::parse("x").expect("seq"), Action::TogglePause)));
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].line, Some(4));
    }
}
