//! `wavelet plugin install`: the Omarchy bar widget.

use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use crate::daemon::client::exe;

const ID: &str = "bsmail.wavelet";
const MANIFEST: &str = include_str!("../plugin/bsmail.wavelet/manifest.json");
const WIDGET: &str = include_str!("../plugin/bsmail.wavelet/BarWidget.qml");
const PLACEHOLDER: &str = "@WAVELET_BIN@";
const PLACEMENT: &str = r#"{"section":"right","after":"akshar.radio-atlas"}"#;

/// The widget with the full path to this executable, because the bar's PATH
/// has no ~/.cargo/bin. Escaped for a QML string literal.
pub fn widget_for(bin: &Path) -> String {
    let bin = bin.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
    WIDGET.replace(PLACEHOLDER, &bin)
}

pub fn install() -> Result<(), String> {
    let bin = exe().map_err(|e| format!("cannot find this executable: {e}"))?;
    let dir = dirs::config_dir().ok_or("no config directory")?.join("omarchy").join("plugins").join(ID);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (name, text) in [("manifest.json", MANIFEST.to_string()), ("BarWidget.qml", widget_for(&bin))] {
        let path = dir.join(name);
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    shell(&["rescanPlugins"])?;
    place()?;
    println!("installed the bar widget in {}", dir.display());
    Ok(())
}

/// The shell scans plugins in the background after a rescan and answers
/// "not ready" for an id it has not reached yet, so placement is retried.
fn place() -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let out = Command::new("omarchy-shell")
            .args(["shell", "putBarWidget", ID, PLACEMENT])
            .output()
            .map_err(|e| format!("omarchy-shell: {e}"))?;
        let reply = String::from_utf8_lossy(&out.stdout).trim().to_string();
        match reply.as_str() {
            "ok" => return Ok(()),
            "not ready" if Instant::now() < deadline => thread::sleep(Duration::from_millis(250)),
            _ => return Err(format!("the bar did not add the widget: {reply}")),
        }
    }
}

fn shell(args: &[&str]) -> Result<(), String> {
    let status = Command::new("omarchy-shell")
        .args(["-q", "shell"])
        .args(args)
        .status()
        .map_err(|e| format!("omarchy-shell: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("omarchy-shell shell {} failed ({status})", args[0])) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_widget_names_the_binary_by_full_path_everywhere() {
        assert!(WIDGET.contains(PLACEHOLDER), "BarWidget.qml must carry {PLACEHOLDER} for the installer");
        let out = widget_for(Path::new("/home/me/.cargo/bin/wavelet"));
        assert!(!out.contains(PLACEHOLDER));
        assert_eq!(out.matches("/home/me/.cargo/bin/wavelet").count(), WIDGET.matches(PLACEHOLDER).count());
    }

    #[test]
    fn a_quote_or_backslash_in_the_path_cannot_end_the_qml_string() {
        let out = widget_for(Path::new(r#"/opt/we"ird\bin/wavelet"#));
        assert!(out.contains(r#"/opt/we\"ird\\bin/wavelet"#), "{out}");
    }

    #[test]
    fn the_manifest_is_the_widget_it_installs() {
        let m: serde_json::Value = serde_json::from_str(MANIFEST).expect("manifest is JSON");
        assert_eq!(m["id"], ID);
        assert_eq!(m["entryPoints"]["barWidget"], "BarWidget.qml");
    }
}
