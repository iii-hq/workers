//! `coder::list-folder` — paginated single-folder listing. Sorted by
//! name (directories and files mixed). Non-accessible entries are still
//! returned, flagged `non_accessible: true`, so callers can see they
//! exist even though they can't be read or written.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::code::config::CoderConfig;
use crate::code::error::{err_to_string, CoderError};
use crate::code::path::PathResolver;

// examples are wire-contract; goldens pin them.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(example = "example_list_folder_input")]
pub struct ListFolderInput {
    /// Folder to list. Relative to the primary allowed root, or an absolute
    /// path inside any allowed root. Defaults to `.` (the primary root
    /// itself). Call `coder::info` to see the allowed roots. Paths outside
    /// every allowed root are rejected — use the shell worker's
    /// `shell::fs::*` for host paths outside the jail.
    #[serde(default = "default_path")]
    pub path: String,
    #[serde(default = "default_page")]
    pub page: u32,
    /// Capped by `config.list_max_page_size`; falls back to
    /// `config.list_default_page_size` when omitted.
    #[serde(default)]
    pub page_size: Option<u32>,
    /// Internal harness filesystem scope; omitted from published schema.
    #[serde(default)]
    #[schemars(skip)]
    pub fs_scope: Option<crate::fs::FsScope>,
}

fn default_path() -> String {
    ".".to_string()
}
fn default_page() -> u32 {
    1
}

// examples are wire-contract; goldens pin them.
fn example_list_folder_input() -> serde_json::Value {
    serde_json::json!({
        "path": "src",
        "page": 1,
        "page_size": 50
    })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListFolderOutput {
    /// Canonical absolute path of the listed folder (resolved through the
    /// jail). Entries carry only `name`; derive an entry's absolute path
    /// by joining: entry path = this path + "/" + name. Operations on
    /// derived paths re-validate through the jail.
    pub path: String,
    pub entries: Vec<DirEntry>,
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
    pub has_more: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DirEntry {
    /// Entry basename. The absolute path is derivable from the response's
    /// `path`: entry path = folder path + "/" + name.
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    pub mtime: i64,
    /// True if this entry matches `non_accessible_globs` — caller cannot
    /// read/write/delete it via `coder::*` even though it shows up here.
    pub non_accessible: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

pub async fn handle(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    req: ListFolderInput,
) -> Result<ListFolderOutput, String> {
    // Offload the synchronous directory read to a blocking thread so a very
    // large folder can't stall the shared runtime (shell::exec/jobs/reload).
    tokio::task::spawn_blocking(move || inner(&resolver, &cfg, req).map_err(err_to_string))
        .await
        .map_err(|e| format!("list-folder task join failed: {e}"))?
}

fn inner(
    resolver: &PathResolver,
    cfg: &CoderConfig,
    req: ListFolderInput,
) -> Result<ListFolderOutput, CoderError> {
    // list-folder uses `resolve`, not `require_writable`, so callers can
    // list a directory that contains non-accessible *children* even if the
    // directory itself happened to match a glob.
    let abs = resolver.resolve_opt(crate::fs::scope_root(req.fs_scope.as_ref()), &req.path)?;
    // NotFound is intercepted with the wire path in scope so the C211
    // message names the path the caller supplied (standardized wording —
    // REDACTION INVARIANT: identical to the glob-denied message).
    let md = std::fs::metadata(&abs).map_err(|e| CoderError::io_for_path(e, &req.path))?;
    if !md.is_dir() {
        return Err(CoderError::BadInput(format!(
            "not a directory: {}",
            req.path
        )));
    }

    let page_size = req
        .page_size
        .unwrap_or(cfg.list_default_page_size)
        .min(cfg.list_max_page_size)
        .max(1);
    let page = req.page.max(1);

    let mut all: Vec<DirEntry> = Vec::new();
    let read_dir = std::fs::read_dir(&abs).map_err(|e| CoderError::io_for_path(e, &req.path))?;
    for entry in read_dir {
        let e = entry?;
        let name = e.file_name().to_string_lossy().into_owned();
        let entry_md = match e.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let abs_entry = e.path();
        all.push(DirEntry {
            name,
            kind: classify(&entry_md),
            size: entry_md.len(),
            mtime: unix_mtime(&entry_md),
            non_accessible: resolver.is_non_accessible(&abs_entry),
        });
    }
    all.sort_by(|a, b| a.name.cmp(&b.name));

    let total = all.len() as u64;
    let start = ((page.saturating_sub(1)) as usize).saturating_mul(page_size as usize);
    let end = start.saturating_add(page_size as usize).min(all.len());
    let entries = if start < all.len() {
        all[start..end].to_vec()
    } else {
        Vec::new()
    };
    let has_more = (end as u64) < total;

    Ok(ListFolderOutput {
        path: abs.display().to_string(),
        entries,
        total,
        page,
        page_size,
        has_more,
    })
}

fn classify(md: &std::fs::Metadata) -> EntryKind {
    let ft = md.file_type();
    if ft.is_symlink() {
        EntryKind::Symlink
    } else if ft.is_dir() {
        EntryKind::Dir
    } else if ft.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

fn unix_mtime(md: &std::fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Clone for DirEntry {
    fn clone(&self) -> Self {
        DirEntry {
            name: self.name.clone(),
            kind: match self.kind {
                EntryKind::File => EntryKind::File,
                EntryKind::Dir => EntryKind::Dir,
                EntryKind::Symlink => EntryKind::Symlink,
                EntryKind::Other => EntryKind::Other,
            },
            size: self.size,
            mtime: self.mtime,
            non_accessible: self.non_accessible,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, Arc<PathResolver>, Arc<CoderConfig>) {
        let tmp = tempdir().unwrap();
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec!["**/.env".to_string()],
            list_default_page_size: 100,
            list_max_page_size: 1000,
            ..CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        (tmp, resolver, cfg)
    }

    #[tokio::test]
    async fn lists_mixed_entries_sorted() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("b.txt"), "x").unwrap();
        std::fs::write(tmp.path().join("a.txt"), "x").unwrap();
        std::fs::create_dir(tmp.path().join("dir")).unwrap();
        let out = handle(
            r,
            c,
            ListFolderInput {
                path: ".".into(),
                page: 1,
                page_size: None,
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let names: Vec<_> = out.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a.txt", "b.txt", "dir"]);
        assert_eq!(out.total, 3);
        assert!(!out.has_more);
        // The folder path is canonical-absolute (decision D2-eng);
        // entries carry only names.
        let base = std::fs::canonicalize(tmp.path()).unwrap();
        assert_eq!(out.path, base.display().to_string());
        // WIRE-CONTRACT PIN: the documented derivation rule (entry path =
        // folder path + "/" + name) must reproduce the real fs path.
        let derived = format!("{}/{}", out.path, out.entries[0].name);
        assert_eq!(derived, base.join("a.txt").display().to_string());
    }

    #[tokio::test]
    async fn pagination_works() {
        let (tmp, r, c) = setup();
        for i in 0..5 {
            std::fs::write(tmp.path().join(format!("f{i}.txt")), "x").unwrap();
        }
        let out = handle(
            r,
            c,
            ListFolderInput {
                path: ".".into(),
                page: 2,
                page_size: Some(2),
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let names: Vec<_> = out.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["f2.txt", "f3.txt"]);
        assert_eq!(out.total, 5);
        assert!(out.has_more);
    }

    #[tokio::test]
    async fn page_size_caps_at_max() {
        let (tmp, r, _c) = setup();
        std::fs::write(tmp.path().join("x.txt"), "x").unwrap();
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            list_default_page_size: 50,
            list_max_page_size: 5,
            ..CoderConfig::default()
        });
        let out = handle(
            r,
            cfg,
            ListFolderInput {
                path: ".".into(),
                page: 1,
                page_size: Some(9999),
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(out.page_size, 5);
    }

    #[tokio::test]
    async fn non_accessible_entries_still_listed_with_flag() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join(".env"), "x").unwrap();
        std::fs::write(tmp.path().join("a.txt"), "x").unwrap();
        let out = handle(
            r,
            c,
            ListFolderInput {
                path: ".".into(),
                page: 1,
                page_size: None,
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let dot = out
            .entries
            .iter()
            .find(|e| e.name == ".env")
            .expect("listed");
        assert!(dot.non_accessible);
        let a = out
            .entries
            .iter()
            .find(|e| e.name == "a.txt")
            .expect("listed");
        assert!(!a.non_accessible);
    }

    #[tokio::test]
    async fn rejects_path_that_is_not_a_directory() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "x").unwrap();
        let err = handle(
            r,
            c,
            ListFolderInput {
                path: "a.txt".into(),
                page: 1,
                page_size: None,
                fs_scope: None,
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("C210"));
    }
}
