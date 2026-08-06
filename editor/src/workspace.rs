//! The workspace: a project root, the buffers open against it, and which
//! folders are expanded.
//!
//! This is the model the whole worker exists to hold, and it deliberately does
//! not live in the browser. A workspace kept in a page is a workspace only that
//! page can see: an agent cannot tell what you have open, a reload loses it,
//! and two surfaces onto the same project disagree. Keeping it in the `state`
//! worker instead makes "what is open" a fact on the bus, which is what lets a
//! human and an agent share one editor rather than two views that happen to
//! read the same disk.
//!
//! Nothing here does I/O. The functions are pure transforms over the record so
//! the rules that are easy to get wrong — above all the path remap on a move —
//! are testable without an engine.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// State scope every key below lives under.
pub const SCOPE: &str = "editor";
/// Key holding the active root, so a fresh surface can find its way back.
pub const ACTIVE_ROOT_KEY: &str = "active-root";

/// Per-project session key. Keyed by root path exactly like the editor this
/// follows: opening a second project must not overwrite the first one's tabs.
pub fn session_key(root: &str) -> String {
    format!("session:{root}")
}

/// Key holding the recent-changes log.
///
/// The log lives here rather than in a page for the same reason the workspace
/// does: a record kept in a browser tab is a record only that tab can see. A
/// page that was not open while an agent worked had no way to learn what it
/// did, and a reload threw away what it had seen — so the one question the
/// feed exists to answer went unanswered exactly when it mattered.
pub const CHANGES_KEY: &str = "changes";

/// How many changes are kept. The log is a recent-activity feed, not history:
/// the durable record of a change is the file and its git history.
pub const MAX_CHANGES: usize = 100;

/// Fold a change into the log: newest first, one entry per path, bounded.
///
/// Collapsing per path is what keeps a save loop from burying every other
/// change. Pure so the rule is testable without a bus.
pub fn record_change<T: Clone>(log: &[T], entry: T, path_of: impl Fn(&T) -> String) -> Vec<T> {
    let path = path_of(&entry);
    let mut next = Vec::with_capacity(log.len() + 1);
    next.push(entry);
    next.extend(log.iter().filter(|e| path_of(e) != path).cloned());
    next.truncate(MAX_CHANGES);
    next
}

/// Normalize a workspace root to an absolute path with no trailing slash.
///
/// Every filesystem call the workspace makes goes through the shell worker,
/// whose fs jail rejects relative paths outright ("path must be absolute").
/// A relative root — including the historical default of "." — therefore
/// cannot work; resolve it against `cwd` at the door so the stored root, the
/// per-project session key, and every joined path are absolute and stable.
/// `cwd` is a parameter so this stays pure and testable.
pub fn normalize_root(root: &str, cwd: &str) -> String {
    let trimmed = root.trim();
    let base = match trimmed {
        "" | "." | "./" => cwd.to_string(),
        rel if !rel.starts_with('/') => {
            let rel = rel.strip_prefix("./").unwrap_or(rel);
            format!("{}/{rel}", cwd.trim_end_matches('/'))
        }
        abs => abs.to_string(),
    };
    let out = base.trim_end_matches('/');
    if out.is_empty() {
        "/".to_string()
    } else {
        out.to_string()
    }
}

/// One open file. `mtime` is what the conflict guard compares against, so it
/// is part of the shared record rather than per-surface bookkeeping — two
/// surfaces editing one file must agree on which version they started from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Buffer {
    /// Path relative to the workspace root.
    pub path: String,
    /// Last-modified time this buffer was read at, Unix seconds.
    pub mtime: i64,
    /// Opaque version of the content this buffer was read at — the same fact
    /// `mtime` carries, in the form that survives two writes inside one second.
    /// A surface saves against it by sending it as `expected_version`.
    ///
    /// Defaulted so a session persisted before this field existed still loads.
    /// Empty means "unknown": a surface holding an empty version has only the
    /// mtime to save against, which is the behaviour it had all along.
    #[serde(default)]
    pub version: String,
    /// Monaco language id for the path.
    pub language: String,
}

/// Everything remembered about one project.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Session {
    /// Open buffers, in the order they were opened.
    #[serde(default)]
    pub buffers: Vec<Buffer>,
    /// Root-relative folder paths currently expanded in the tree.
    #[serde(default)]
    pub expanded: Vec<String>,
}

impl Session {
    /// Record a buffer, replacing any existing one for the same path.
    ///
    /// Re-opening a file must not stack duplicate tabs, and it must refresh
    /// the mtime — the second open is by definition the newer read.
    pub fn upsert(&mut self, buffer: Buffer) {
        match self.buffers.iter_mut().find(|b| b.path == buffer.path) {
            Some(existing) => *existing = buffer,
            None => self.buffers.push(buffer),
        }
    }

    pub fn close(&mut self, path: &str) -> bool {
        let before = self.buffers.len();
        self.buffers.retain(|b| b.path != path);
        before != self.buffers.len()
    }

    pub fn expand(&mut self, path: &str) {
        if !self.expanded.iter().any(|p| p == path) {
            self.expanded.push(path.to_string());
        }
    }

    pub fn collapse(&mut self, path: &str) {
        // Collapsing a folder collapses what is under it; leaving descendants
        // marked expanded would re-open them the moment the parent reopens.
        self.expanded.retain(|p| p != path && !is_under(p, path));
    }

    /// Rewrite every path at or under `from` to sit under `to`.
    ///
    /// This is the single reason moves go through one function. A buffer left
    /// pointing at the old path saves its contents back to where the file used
    /// to be, recreating the folder that was just moved — the failure is
    /// silent, and it is data loss. Renaming a folder invalidates paths in
    /// bulk, so the remap has to cover descendants, not just exact matches.
    pub fn remap(&mut self, from: &str, to: &str) -> usize {
        let mut changed = 0;
        for buffer in self.buffers.iter_mut() {
            if let Some(next) = remapped(&buffer.path, from, to) {
                buffer.path = next;
                changed += 1;
            }
        }
        for path in self.expanded.iter_mut() {
            if let Some(next) = remapped(path, from, to) {
                *path = next;
                changed += 1;
            }
        }
        changed
    }
}

/// `true` when `path` sits inside directory `dir` (not equal to it).
fn is_under(path: &str, dir: &str) -> bool {
    path.len() > dir.len() && path.starts_with(dir) && path.as_bytes()[dir.len()] == b'/'
}

/// The rewritten path, or `None` when `path` is unaffected by the move.
///
/// Matching is on whole segments: moving `src/app` must not touch
/// `src/application.rs`, which a plain `starts_with` would rewrite into
/// nonsense.
fn remapped(path: &str, from: &str, to: &str) -> Option<String> {
    if path == from {
        return Some(to.to_string());
    }
    if is_under(path, from) {
        return Some(format!("{to}{}", &path[from.len()..]));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(path: &str) -> Buffer {
        Buffer {
            path: path.to_string(),
            mtime: 1,
            version: "1-0000000000000001".to_string(),
            language: "rust".to_string(),
        }
    }

    #[test]
    fn normalize_root_resolves_the_relative_defaults() {
        assert_eq!(normalize_root(".", "/home/me/rig"), "/home/me/rig");
        assert_eq!(normalize_root("", "/home/me/rig"), "/home/me/rig");
        assert_eq!(normalize_root("./", "/home/me/rig"), "/home/me/rig");
    }

    #[test]
    fn normalize_root_absolutizes_relative_paths_against_cwd() {
        assert_eq!(normalize_root("proj", "/home/me"), "/home/me/proj");
        assert_eq!(normalize_root("./proj", "/home/me"), "/home/me/proj");
        assert_eq!(normalize_root("proj", "/home/me/"), "/home/me/proj");
    }

    #[test]
    fn normalize_root_keeps_absolute_paths_and_trims_trailing_slash() {
        assert_eq!(normalize_root("/repo", "/elsewhere"), "/repo");
        assert_eq!(normalize_root("/repo/", "/elsewhere"), "/repo");
        assert_eq!(normalize_root(" /repo ", "/elsewhere"), "/repo");
        assert_eq!(normalize_root("/", "/elsewhere"), "/");
    }

    #[test]
    fn session_key_is_per_project() {
        assert_ne!(session_key("/a"), session_key("/b"));
        assert!(session_key("/a/b").contains("/a/b"));
    }

    #[test]
    fn reopening_a_file_replaces_rather_than_duplicates() {
        let mut s = Session::default();
        s.upsert(buf("a.rs"));
        let mut newer = buf("a.rs");
        newer.mtime = 99;
        newer.version = "9-000000000000009f".to_string();
        s.upsert(newer);
        assert_eq!(s.buffers.len(), 1);
        assert_eq!(s.buffers[0].mtime, 99, "the newer read wins");
        assert_eq!(
            s.buffers[0].version, "9-000000000000009f",
            "the version travels with the mtime it was read at"
        );
    }

    /// A session written before buffers carried a version must still load,
    /// with the version reading as unknown rather than failing the parse.
    #[test]
    fn a_buffer_without_a_version_still_loads() {
        let back: Session =
            serde_json::from_str(r#"{"buffers":[{"path":"a.rs","mtime":7,"language":"rust"}]}"#)
                .expect("an older buffer record parses");
        assert_eq!(back.buffers[0].mtime, 7);
        assert!(back.buffers[0].version.is_empty());
    }

    #[test]
    fn close_reports_whether_anything_was_open() {
        let mut s = Session::default();
        s.upsert(buf("a.rs"));
        assert!(s.close("a.rs"));
        assert!(!s.close("a.rs"));
    }

    #[test]
    fn collapsing_a_folder_collapses_its_descendants() {
        let mut s = Session::default();
        s.expand("src");
        s.expand("src/app");
        s.expand("src/app/deep");
        s.expand("tests");
        s.collapse("src");
        assert_eq!(s.expanded, vec!["tests".to_string()]);
    }

    /// Collapsing takes descendants with it, and only descendants. `src/app`
    /// must not collapse `src/application` — the same whole-segment rule the
    /// remap has, and the same silent wrongness when it is missing.
    #[test]
    fn collapse_matches_whole_segments_only() {
        let mut s = Session::default();
        s.expand("src/app");
        s.expand("src/app/deep");
        s.expand("src/application");
        s.collapse("src/app");
        assert_eq!(s.expanded, vec!["src/application".to_string()]);
    }

    #[test]
    fn expand_is_idempotent() {
        let mut s = Session::default();
        s.expand("src");
        s.expand("src");
        assert_eq!(s.expanded.len(), 1);
    }

    /// Re-opening a file must not move its tab: a surface renders buffers in
    /// this order, so an upsert that pushed the re-read file to the end would
    /// shuffle the tab bar on every save.
    #[test]
    fn upsert_replaces_in_place_and_keeps_the_order() {
        let mut s = Session::default();
        s.upsert(buf("a.rs"));
        s.upsert(buf("b.rs"));
        let mut again = buf("a.rs");
        again.mtime = 42;
        s.upsert(again);

        let paths: Vec<&str> = s.buffers.iter().map(|b| b.path.as_str()).collect();
        assert_eq!(paths, vec!["a.rs", "b.rs"]);
        assert_eq!(
            s.buffers[0].mtime, 42,
            "the newer read replaced the older one"
        );
    }

    #[test]
    fn moving_a_file_remaps_its_buffer() {
        let mut s = Session::default();
        s.upsert(buf("old.rs"));
        assert_eq!(s.remap("old.rs", "new.rs"), 1);
        assert_eq!(s.buffers[0].path, "new.rs");
    }

    #[test]
    fn moving_a_folder_remaps_every_path_under_it() {
        let mut s = Session::default();
        s.upsert(buf("src/app/a.rs"));
        s.upsert(buf("src/app/deep/b.rs"));
        s.upsert(buf("other.rs"));
        s.expand("src/app");
        s.expand("src/app/deep");

        assert_eq!(
            s.remap("src/app", "src/ui"),
            4,
            "two buffers and two expanded folders were rewritten"
        );

        let paths: Vec<&str> = s.buffers.iter().map(|b| b.path.as_str()).collect();
        assert_eq!(paths, vec!["src/ui/a.rs", "src/ui/deep/b.rs", "other.rs"]);
        assert_eq!(s.expanded, vec!["src/ui", "src/ui/deep"]);
    }

    #[test]
    fn remap_matches_whole_segments_only() {
        // `src/app` must not swallow `src/application.rs`, in either list: a
        // plain `starts_with` rewrites both into nonsense.
        let mut s = Session::default();
        s.upsert(buf("src/application.rs"));
        s.upsert(buf("src/app/x.rs"));
        s.expand("src/application");
        s.expand("src/app");

        assert_eq!(
            s.remap("src/app", "src/ui"),
            2,
            "only the buffer and the folder actually under src/app move"
        );
        let paths: Vec<&str> = s.buffers.iter().map(|b| b.path.as_str()).collect();
        assert_eq!(paths, vec!["src/application.rs", "src/ui/x.rs"]);
        assert_eq!(s.expanded, vec!["src/application", "src/ui"]);
    }

    #[test]
    fn remap_leaves_unrelated_paths_alone() {
        let mut s = Session::default();
        s.upsert(buf("a.rs"));
        assert_eq!(s.remap("b.rs", "c.rs"), 0);
        assert_eq!(s.buffers[0].path, "a.rs");
    }

    #[test]
    fn session_round_trips_through_json() {
        let mut s = Session::default();
        s.upsert(buf("a.rs"));
        s.expand("src");
        let json = serde_json::to_string(&s).unwrap();
        let back: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn missing_fields_default_so_an_old_record_still_loads() {
        let back: Session = serde_json::from_str("{}").unwrap();
        assert!(back.buffers.is_empty());
        assert!(back.expanded.is_empty());
    }
}
