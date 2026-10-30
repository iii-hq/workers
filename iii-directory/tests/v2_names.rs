//! The worker's sources, shipped skills, prompts, agent profiles, console
//! page and README name the observe v2 functions that carry each event.
//! Each entry pairs a v1 trigger type or stream name with the function that
//! carries its events in observe v2.

use std::path::{Path, PathBuf};

const RETIRED: &[(&str, &str)] = &[
    ("functions-available", "engine::on-functions-changed"),
    (
        "agent::events",
        "claude::on-agent-event / pi::on-agent-event",
    ),
    ("console:script", "console::ui::asset"),
    ("console:style", "console::ui::asset"),
    ("console:module", "console::ui::asset"),
];

const SCANNED: &[&str] = &["rs", "md", "ts", "tsx", "mjs", "css", "yaml", "json"];

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    // `agents/` holds local agent profiles and is not in the repository, so a
    // fresh checkout has none.
    if !dir.exists() {
        return;
    }
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            if name != "node_modules" && name != "dist" {
                collect(&path, out);
            }
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| SCANNED.contains(&e))
        {
            out.push(path);
        }
    }
}

#[test]
fn shipped_text_names_v2_functions() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![root.join("README.md"), root.join("iii.worker.yaml")];
    for dir in ["src", "skills", "prompts", "agents", "ui"] {
        collect(&root.join(dir), &mut files);
    }
    for expected in [
        "skills/agent-delegation.md",
        "src/configuration.rs",
        "ui/styles.css",
    ] {
        assert!(
            files.iter().any(|f| f.ends_with(expected)),
            "{expected} is not scanned"
        );
    }

    let mut hits = Vec::new();
    for file in &files {
        let text =
            std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        for (n, line) in text.lines().enumerate() {
            for (v1, v2) in RETIRED {
                if line.contains(v1) {
                    hits.push(format!(
                        "{}:{}: `{v1}` (observe v2: {v2})",
                        file.strip_prefix(&root).unwrap_or(file).display(),
                        n + 1
                    ));
                }
            }
        }
    }
    assert!(hits.is_empty(), "v1 names:\n{}", hits.join("\n"));
}
