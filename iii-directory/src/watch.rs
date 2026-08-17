//! Filesystem watch: external changes under the skills roots fire the
//! matching `directory::*::on-change` with `{op: "external"}`.
//!
//! Reads are scan-on-demand and never stale — this is purely the push
//! signal for open subscribers (directory tabs, agent bindings) when a
//! file is pasted, edited, deleted, or renamed outside the worker.
//! Doorbell, not ledger: same shape as the database worker's sqlite
//! watch. Events are debounced into at most one fire per kind per
//! window; worker-mediated writes are suppressed via the recent-writes
//! set `write_file_atomic` feeds. No fallback tick — a missed fs event
//! (NFS and friends) leaves an open tab stale until its next
//! interaction; nothing is lost because every read re-scans.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::event::{AccessKind, AccessMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::fs_source::{classify_rel_path, SourceKind};
use crate::sources::take_self_write;

/// Coalescing window: every event inside it folds into one fire per kind.
const DEBOUNCE: Duration = Duration::from_millis(300);

/// Keep the watch alive; dropping the handle stops it.
pub struct FsWatchHandle {
    _watcher: RecommendedWatcher,
}

/// Watch `roots` recursively; call `sink(kind)` at most once per kind per
/// debounce window when an external `*.md` change lands under one. Missing
/// roots are created rather than skipped: on a fresh install neither
/// default root exists yet, and both are restart-required config, so
/// skipping would leave the watch off for the whole process lifetime (the
/// write path already creates parents per file).
pub fn spawn_fs_watch(
    roots: Vec<PathBuf>,
    sink: impl Fn(SourceKind) + Send + 'static,
) -> Result<FsWatchHandle, String> {
    let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(tx).map_err(|e| format!("fs watch: {e}"))?;
    let mut watched: Vec<PathBuf> = Vec::new();
    for root in roots {
        let _ = std::fs::create_dir_all(&root);
        if root.is_dir() {
            watcher
                .watch(&root, RecursiveMode::Recursive)
                .map_err(|e| format!("fs watch {}: {e}", root.display()))?;
            watched.push(root);
        }
    }
    if watched.is_empty() {
        return Err("fs watch: no usable roots to watch".into());
    }
    std::thread::Builder::new()
        .name("directory-fs-watch".into())
        .spawn(move || debounce_loop(&rx, &watched, DEBOUNCE, sink))
        .map_err(|e| format!("fs watch thread: {e}"))?;
    Ok(FsWatchHandle { _watcher: watcher })
}

/// Block on the event channel; after the first relevant event, keep
/// collecting for `window`, then fire each collected kind once.
fn debounce_loop(
    rx: &mpsc::Receiver<notify::Result<notify::Event>>,
    roots: &[PathBuf],
    window: Duration,
    sink: impl Fn(SourceKind),
) {
    while let Ok(first) = rx.recv() {
        let mut kinds: HashSet<SourceKind> = HashSet::new();
        // One rename is reported twice (`Name(To)` on the new path, then a
        // `Name(Both)` pairing it with the old), so judge each path once
        // per window: the second sighting would find the self-write mark
        // already consumed and fire our own write back at subscribers.
        // Not airtight — a pair split across the deadline lands in two
        // windows, each with its own `seen`. Marking before the rename (see
        // `write_file_atomic`) is what makes that rare rather than routine.
        let mut seen: HashSet<PathBuf> = HashSet::new();
        collect(first, roots, &mut seen, &mut kinds);
        let deadline = Instant::now() + window;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(left) {
                Ok(ev) => collect(ev, roots, &mut seen, &mut kinds),
                Err(_) => break,
            }
        }
        for kind in kinds {
            sink(kind);
        }
    }
}

/// Fold one notify event into the per-kind set: markdown files only
/// (which also drops our `.md.tmp` staging — its extension is `tmp`),
/// self-writes suppressed, paths classified relative to whichever root
/// contains them.
fn collect(
    ev: notify::Result<notify::Event>,
    roots: &[PathBuf],
    seen: &mut HashSet<PathBuf>,
    kinds: &mut HashSet<SourceKind>,
) {
    let Ok(ev) = ev else { return };
    // Opens and read-closes are not changes, and inotify reports both.
    // Every list/get re-scans the tree, so firing on a read would loop:
    // fire → subscriber re-lists → fire. `Close(Write)` is kept — an
    // mmap-style editor save can emit `IN_CLOSE_WRITE` with no preceding
    // `IN_MODIFY`, which is the external-editor case this feature is for.
    match ev.kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => {}
        EventKind::Access(_) => return,
        _ => {}
    }
    for path in ev.paths {
        if path.extension().map(|e| e != "md").unwrap_or(true) {
            continue;
        }
        if !seen.insert(path.clone()) {
            continue;
        }
        if take_self_write(&path) {
            continue;
        }
        let Some(rel) = roots.iter().find_map(|r| path.strip_prefix(r).ok()) else {
            continue;
        };
        kinds.insert(classify_rel_path(rel));
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::fs_source::SourceKind;

    #[test]
    fn external_md_write_fires_kind_once_and_self_write_is_suppressed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("system-prompts")).unwrap();
        std::fs::create_dir_all(tmp.path().join("ns/prompts")).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let _h = super::spawn_fs_watch(vec![tmp.path().to_path_buf()], move |k| {
            let _ = tx.send(k);
        })
        .unwrap();

        // External paste → exactly one SystemPrompt fire for the burst.
        std::fs::write(
            tmp.path().join("system-prompts/pasted.md"),
            "---\ndescription: x\n---\nB\n",
        )
        .unwrap();
        let kind = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("external event");
        assert_eq!(kind, SourceKind::SystemPrompt);
        assert!(
            rx.recv_timeout(Duration::from_millis(700)).is_err(),
            "debounce must coalesce the create+write burst into one fire"
        );

        // Non-md noise is ignored.
        std::fs::write(tmp.path().join("system-prompts/noise.txt"), b"x").unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(700)).is_err());

        // Worker-mediated write (goes through write_file_atomic): suppressed.
        crate::sources::write_file_atomic(
            &tmp.path().join("ns/prompts/own.md"),
            b"---\ndescription: y\n---\nB\n",
        )
        .unwrap();
        assert!(
            rx.recv_timeout(Duration::from_millis(900)).is_err(),
            "self-write must not fire"
        );

        // Reading a watched file is not a change. inotify reports opens and
        // closes too, and every list/get re-scans — firing on those would
        // loop: fire → subscriber re-lists → fire.
        let _ = std::fs::read_to_string(tmp.path().join("system-prompts/pasted.md")).unwrap();
        assert!(
            rx.recv_timeout(Duration::from_millis(700)).is_err(),
            "reads must not fire"
        );

        // Closing positive: the three silences above must mean "suppressed",
        // not "delivery died after the first fire".
        std::fs::write(tmp.path().join("ns/plain.md"), "# S\n").unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(3))
                .expect("watch still delivers"),
            SourceKind::Skill
        );
    }

    /// Production watches two roots (global `skills_folder` + local), and
    /// each path must classify against the root that contains it.
    #[test]
    fn two_roots_each_classify_against_their_own_root() {
        let global = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(global.path().join("ns/prompts")).unwrap();
        std::fs::create_dir_all(local.path().join("ns")).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let _h = super::spawn_fs_watch(
            vec![global.path().to_path_buf(), local.path().to_path_buf()],
            move |k| {
                let _ = tx.send(k);
            },
        )
        .unwrap();

        std::fs::write(global.path().join("ns/prompts/cmd.md"), "x").unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(3)).expect("root 1"),
            SourceKind::Prompt
        );

        std::fs::write(local.path().join("ns/guide.md"), "x").unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(3)).expect("root 2"),
            SourceKind::Skill
        );
    }

    #[test]
    fn take_self_write_consumes_the_mark() {
        let p = std::path::Path::new("/nonexistent/x.md");
        assert!(!crate::sources::take_self_write(p));
        crate::sources::mark_self_write_for_tests(p);
        assert!(crate::sources::take_self_write(p));
        assert!(!crate::sources::take_self_write(p), "consumed");
    }
}
