//! The data folder:
//!
//! ```text
//! data/stories/
//!   compiler/                       the materialized Node compiler
//!   <workspace>/
//!     lines/<key>/index.json        one built line (worktree, worktree.prev, <sha>)
//!     lines/<key>/manifest.json     served path -> { sha256, size, content_type }
//!     files/<sha256>                content-addressed build outputs and story inputs
//!     renders/<hash>/               screenshot + tree of one (line, state, args, viewport)
//!     history/<project>__<id>.jsonl versions seen per component
//!     worktrees/<key>/              temporary checkouts while a ref builds
//!     tmp/<build>/                  vite output before import
//! ```
//!
//! Every write goes through a temp file and a rename so a crash never leaves
//! a half-written index behind.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::{ChangeKind, Index, LineInfo, LineKind, sha256_hex};

pub const WORKTREE_KEY: &str = "worktree";
pub const PREV_KEY: &str = "worktree.prev";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileEntry {
    pub sha256: String,
    pub size: u64,
    pub content_type: String,
}

pub type Manifest = BTreeMap<String, FileEntry>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HistoryEntry {
    pub version: String,
    pub line: String,
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<ChangeKind>,
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Store {
    pub root: PathBuf,
}

pub fn content_type_of(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "txt" | "md" => "text/plain; charset=utf-8",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        _ => "application/octet-stream",
    }
}

fn safe_segment(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// A served path is a `/`-joined list of safe segments.
pub fn safe_served_path(path: &str) -> bool {
    !path.is_empty() && path.split('/').all(safe_segment)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

impl Store {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn compiler_dir(&self) -> PathBuf {
        self.root.join("compiler")
    }

    pub fn workspace_dir(&self, workspace: &str) -> PathBuf {
        self.root.join(workspace)
    }

    pub fn line_dir(&self, workspace: &str, key: &str) -> PathBuf {
        self.workspace_dir(workspace).join("lines").join(key)
    }

    pub fn files_dir(&self, workspace: &str) -> PathBuf {
        self.workspace_dir(workspace).join("files")
    }

    pub fn renders_dir(&self, workspace: &str) -> PathBuf {
        self.workspace_dir(workspace).join("renders")
    }

    pub fn worktrees_dir(&self, workspace: &str) -> PathBuf {
        self.workspace_dir(workspace).join("worktrees")
    }

    pub fn tmp_dir(&self, workspace: &str, build_id: &str) -> PathBuf {
        self.workspace_dir(workspace).join("tmp").join(build_id)
    }

    fn history_file(&self, workspace: &str, project: &str, id: &str) -> PathBuf {
        let name = format!("{}__{}.jsonl", project.replace(['/', '\\'], "_"), id);
        self.workspace_dir(workspace).join("history").join(name)
    }

    pub fn has_line(&self, workspace: &str, key: &str) -> bool {
        self.line_dir(workspace, key).join("index.json").is_file()
    }

    pub fn read_index(&self, workspace: &str, key: &str) -> Option<Index> {
        let raw = std::fs::read(self.line_dir(workspace, key).join("index.json")).ok()?;
        serde_json::from_slice(&raw).ok()
    }

    /// Only the `line` header of an index: listing lines must not pay for
    /// deserializing every component and state.
    pub fn read_line_info(&self, workspace: &str, key: &str) -> Option<LineInfo> {
        #[derive(Deserialize)]
        struct Head {
            line: LineInfo,
        }
        let raw = std::fs::read(self.line_dir(workspace, key).join("index.json")).ok()?;
        serde_json::from_slice::<Head>(&raw)
            .ok()
            .map(|head| head.line)
    }

    pub fn read_manifest(&self, workspace: &str, key: &str) -> Option<Manifest> {
        let raw = std::fs::read(self.line_dir(workspace, key).join("manifest.json")).ok()?;
        serde_json::from_slice(&raw).ok()
    }

    /// Every built line of a workspace, newest first. The key is the folder
    /// name, so a rotated working-tree build reads as `worktree.prev`.
    pub fn list_lines(&self, workspace: &str) -> Vec<LineInfo> {
        let mut lines: Vec<LineInfo> =
            std::fs::read_dir(self.workspace_dir(workspace).join("lines"))
                .map(|entries| {
                    entries
                        .flatten()
                        .filter_map(|entry| {
                            let key = entry.file_name().to_string_lossy().into_owned();
                            self.read_index(workspace, &key)
                                .map(|index| LineInfo { key, ..index.line })
                        })
                        .collect()
                })
                .unwrap_or_default();
        lines.sort_by(|a, b| b.built_at.cmp(&a.built_at));
        lines
    }

    /// Store bytes under their hash; returns the hash.
    pub fn put_blob(&self, workspace: &str, bytes: &[u8]) -> std::io::Result<String> {
        let sha = sha256_hex(bytes);
        let target = self.files_dir(workspace).join(&sha);
        if !target.is_file() {
            write_atomic(&target, bytes)?;
        }
        Ok(sha)
    }

    pub fn blob_path(&self, workspace: &str, sha: &str) -> PathBuf {
        self.files_dir(workspace).join(sha)
    }

    pub fn read_blob(&self, workspace: &str, sha: &str) -> Option<Vec<u8>> {
        std::fs::read(self.blob_path(workspace, sha)).ok()
    }

    /// Import a vite output folder: every file lands in `files/` and the
    /// manifest gains `<prefix>/<relative path>` entries.
    pub fn import_dist(
        &self,
        workspace: &str,
        dist: &Path,
        prefix: &str,
        manifest: &mut Manifest,
    ) -> std::io::Result<()> {
        fn walk(
            store: &Store,
            workspace: &str,
            base: &Path,
            dir: &Path,
            prefix: &str,
            manifest: &mut Manifest,
        ) -> std::io::Result<()> {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    walk(store, workspace, base, &path, prefix, manifest)?;
                    continue;
                }
                let rel = path
                    .strip_prefix(base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                if rel.starts_with(".vite/") {
                    continue;
                }
                let bytes = std::fs::read(&path)?;
                let sha256 = store.put_blob(workspace, &bytes)?;
                let served = if prefix.is_empty() || prefix == "." {
                    rel.clone()
                } else {
                    format!("{prefix}/{rel}")
                };
                manifest.insert(
                    served.clone(),
                    FileEntry {
                        sha256,
                        size: bytes.len() as u64,
                        content_type: content_type_of(&served).to_string(),
                    },
                );
            }
            Ok(())
        }
        walk(self, workspace, dist, dist, prefix, manifest)
    }

    pub fn write_line(
        &self,
        workspace: &str,
        index: &Index,
        manifest: &Manifest,
    ) -> std::io::Result<()> {
        let dir = self.line_dir(workspace, &index.line.key);
        write_atomic(&dir.join("manifest.json"), &serde_json::to_vec(manifest)?)?;
        write_atomic(&dir.join("index.json"), &serde_json::to_vec_pretty(index)?)
    }

    /// `lines/worktree` becomes `lines/worktree.prev` (replacing the old
    /// one); the moved index is relabelled so it reads as the previous build.
    pub fn rotate_worktree(&self, workspace: &str) -> std::io::Result<()> {
        let current = self.line_dir(workspace, WORKTREE_KEY);
        if !current.is_dir() {
            return Ok(());
        }
        let prev = self.line_dir(workspace, PREV_KEY);
        if prev.is_dir() {
            std::fs::remove_dir_all(&prev)?;
        }
        std::fs::rename(&current, &prev)?;
        if let Some(mut index) = self.read_index(workspace, PREV_KEY) {
            index.line.key = PREV_KEY.to_string();
            index.line.kind = LineKind::Prev;
            index.line.label = "previous build".to_string();
            write_atomic(
                &prev.join("index.json"),
                &serde_json::to_vec_pretty(&index)?,
            )?;
        }
        Ok(())
    }

    pub fn remove_line(&self, workspace: &str, key: &str) -> std::io::Result<()> {
        let dir = self.line_dir(workspace, key);
        if dir.is_dir() {
            std::fs::remove_dir_all(dir)?;
        }
        Ok(())
    }

    /// Bytes and content type of a served path inside one line.
    pub fn serve(&self, workspace: &str, key: &str, path: &str) -> Option<(Vec<u8>, String)> {
        if !safe_served_path(path) {
            return None;
        }
        let manifest = self.read_manifest(workspace, key)?;
        let entry = manifest.get(path)?;
        let bytes = self.read_blob(workspace, &entry.sha256)?;
        Some((bytes, entry.content_type.clone()))
    }

    pub fn append_history(
        &self,
        workspace: &str,
        project: &str,
        id: &str,
        entry: &HistoryEntry,
    ) -> std::io::Result<()> {
        use std::io::Write;
        let path = self.history_file(workspace, project, id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(file, "{}", serde_json::to_string(entry)?)
    }

    pub fn read_history(&self, workspace: &str, project: &str, id: &str) -> Vec<HistoryEntry> {
        std::fs::read_to_string(self.history_file(workspace, project, id))
            .map(|raw| {
                raw.lines()
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Drop the oldest ref/turn lines beyond `keep`, then every blob no
    /// manifest references. Returns the number of lines removed.
    pub fn prune(&self, workspace: &str, keep: usize) -> usize {
        let mut removable: Vec<LineInfo> = self
            .list_lines(workspace)
            .into_iter()
            .filter(|line| matches!(line.kind, LineKind::Ref | LineKind::Turn))
            .collect();
        let mut removed = 0;
        while removable.len() > keep {
            if let Some(oldest) = removable.pop()
                && self.remove_line(workspace, &oldest.key).is_ok()
            {
                removed += 1;
            }
        }
        self.gc_files(workspace);
        removed
    }

    pub fn gc_files(&self, workspace: &str) {
        let Ok(entries) = std::fs::read_dir(self.files_dir(workspace)) else {
            return;
        };
        let mut referenced = std::collections::HashSet::new();
        for line in self.list_lines(workspace) {
            if let Some(manifest) = self.read_manifest(workspace, &line.key) {
                referenced.extend(manifest.into_values().map(|entry| entry.sha256));
            }
            if let Some(index) = self.read_index(workspace, &line.key) {
                for component in index.components {
                    referenced.extend(component.inputs.into_iter().map(|input| input.sha256));
                }
            }
        }
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !referenced.contains(&name) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::LineKind;

    fn index(key: &str, kind: LineKind, built_at: &str) -> Index {
        Index {
            workspace: "ws".into(),
            line: LineInfo {
                key: key.into(),
                kind,
                label: key.into(),
                sha: None,
                dirty: None,
                built_at: built_at.into(),
            },
            projects: vec![],
            components: vec![],
            warnings: vec![],
        }
    }

    #[test]
    fn import_serve_rotate_and_prune() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("data"));
        let dist = dir.path().join("dist");
        std::fs::create_dir_all(dist.join("assets")).unwrap();
        std::fs::create_dir_all(dist.join(".vite")).unwrap();
        std::fs::write(dist.join("index.html"), "<html>").unwrap();
        std::fs::write(dist.join("assets/a.js"), "js").unwrap();
        std::fs::write(dist.join(".vite/manifest.json"), "{}").unwrap();

        let mut manifest = Manifest::new();
        store
            .import_dist("ws", &dist, "app", &mut manifest)
            .unwrap();
        assert_eq!(manifest.len(), 2);
        assert_eq!(
            manifest["app/assets/a.js"].content_type,
            "text/javascript; charset=utf-8"
        );
        store
            .write_line(
                "ws",
                &index(WORKTREE_KEY, LineKind::Worktree, "2026-01-02T00:00:00Z"),
                &manifest,
            )
            .unwrap();
        let (bytes, ty) = store.serve("ws", WORKTREE_KEY, "app/index.html").unwrap();
        assert_eq!(bytes, b"<html>");
        assert!(ty.starts_with("text/html"));
        assert!(
            store
                .serve("ws", WORKTREE_KEY, "../app/index.html")
                .is_none()
        );

        store.rotate_worktree("ws").unwrap();
        assert!(store.has_line("ws", PREV_KEY));
        assert!(!store.has_line("ws", WORKTREE_KEY));
        let prev = store.read_index("ws", PREV_KEY).unwrap();
        assert_eq!(
            (prev.line.key.as_str(), prev.line.kind),
            (PREV_KEY, LineKind::Prev)
        );
        assert_eq!(store.list_lines("ws")[0].key, PREV_KEY);

        for (i, key) in ["a".repeat(40), "b".repeat(40), "c".repeat(40)]
            .iter()
            .enumerate()
        {
            store
                .write_line(
                    "ws",
                    &index(key, LineKind::Ref, &format!("2026-01-0{}T00:00:00Z", i + 3)),
                    &Manifest::new(),
                )
                .unwrap();
        }
        assert_eq!(store.prune("ws", 2), 1);
        assert!(
            !store.has_line("ws", &"a".repeat(40)),
            "the oldest ref line goes first"
        );
        assert!(
            store.has_line("ws", PREV_KEY),
            "worktree lines are never pruned"
        );
        // Blobs are still referenced by the prev manifest.
        assert_eq!(std::fs::read_dir(store.files_dir("ws")).unwrap().count(), 2);
        store.remove_line("ws", PREV_KEY).unwrap();
        store.gc_files("ws");
        assert_eq!(std::fs::read_dir(store.files_dir("ws")).unwrap().count(), 0);
    }

    #[test]
    fn history_appends_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let entry = HistoryEntry {
            version: "v1".into(),
            line: "worktree".into(),
            at: "t".into(),
            change: Some(ChangeKind::Direct),
            files: vec!["a".into()],
        };
        store
            .append_history("ws", "app", "ui-button", &entry)
            .unwrap();
        store
            .append_history("ws", "app", "ui-button", &entry)
            .unwrap();
        assert_eq!(store.read_history("ws", "app", "ui-button").len(), 2);
        assert!(store.read_history("ws", "app", "nope").is_empty());
    }

    #[test]
    fn served_paths_are_validated() {
        assert!(safe_served_path("app/node_modules/.stories/x.html"));
        assert!(!safe_served_path("app/../x"));
        assert!(!safe_served_path("/app/x"));
        assert!(!safe_served_path("app//x"));
        assert_eq!(content_type_of("x.woff2"), "font/woff2");
    }
}
