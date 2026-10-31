//! The worker's docs name the functions observers observe (contract §1.6):
//! `session::on-*` and `session::store::on-events`, never the event ids the
//! worker emitted as trigger types in 0.25.

use std::path::{Path, PathBuf};

/// The 0.25 trigger-type ids of the event functions.
const TRIGGER_TYPE_IDS: &[&str] = &[
    "session::created",
    "session::message-added",
    "session::message-updated",
    "session::status-changed",
    "session::meta-updated",
    "session::deleted",
    "session::store::events",
];

fn doc_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![root.join("README.md"), root.join("iii.worker.yaml")];
    let mut architecture: Vec<PathBuf> = std::fs::read_dir(root.join("architecture"))
        .expect("architecture/ exists")
        .map(|entry| entry.expect("readable dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    architecture.sort();
    files.extend(architecture);
    files
}

#[test]
fn docs_name_the_event_functions() {
    let mut stale = Vec::new();
    for file in doc_files() {
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        for (n, line) in text.lines().enumerate() {
            for id in TRIGGER_TYPE_IDS {
                if line.contains(id) {
                    stale.push(format!("{}:{}: {id}: {line}", file.display(), n + 1));
                }
            }
        }
    }
    assert!(
        stale.is_empty(),
        "docs name 0.25 trigger-type ids; name the session::on-* functions:\n{}",
        stale.join("\n")
    );
}
