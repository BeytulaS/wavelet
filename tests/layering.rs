use std::path::{Path, PathBuf};

const FORBIDDEN: [&str; 3] = ["ratatui", "crossterm", "tui::"];

fn rust_files(p: &Path, out: &mut Vec<PathBuf>) {
    if p.is_dir() {
        for e in std::fs::read_dir(p).expect("read_dir") {
            rust_files(&e.expect("dir entry").path(), out);
        }
    } else if p.extension().is_some_and(|e| e == "rs") {
        out.push(p.to_path_buf());
    }
}

#[test]
fn player_sources_and_domain_never_name_ui_crates() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    for p in ["player", "sources", "domain.rs"] {
        rust_files(&root.join(p), &mut files);
    }
    assert!(files.len() >= 8, "layering scan found too few files: {files:?}");
    let mut hits = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).expect("read source");
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if FORBIDDEN.iter().any(|w| code.contains(w)) {
                hits.push(format!("{}:{}: {}", f.display(), n + 1, line.trim()));
            }
        }
    }
    assert!(hits.is_empty(), "UI names below the player boundary:\n{}", hits.join("\n"));
}
