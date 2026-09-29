//! The one filesystem pass behind `coder::find-relevant`, its egress gates
//! and the previews the judge navigates by.
//!
//! Ported from dzhng/jevgrep (MIT, Copyright (c) 2026 David Zhang), commit
//! 82ef1fd: `packages/core/src/filesystem.ts` (name and content policy),
//! `retrieve.ts` `previewDirectory`/`previewFile` and `source.ts`
//! `splitSource`.
//!
//! Instead of jevgrep's lazy per-directory cursors, one `ignore` walk builds
//! an in-memory children map up front. Every gate runs there, so a pruned
//! entry never shows up in a preview either:
//! - the jail's protections: operator denylist, `non_accessible_globs`
//!   (REDACTION INVARIANT), `default_exclude_globs`;
//! - `.gitignore`/`.ignore` (also outside a Git checkout), hidden entries,
//!   symlinks and special files;
//! - jevgrep's dependency directories and credential-like names;
//! - the caller's `exclude_globs`.
//!
//! Content gates (control bytes, invalid UTF-8, a PRIVATE KEY block) run on
//! every read: such a file is never scored, admitted or returned.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};

use super::prompts::{DirectoryPreview, FilePreview, Kind, PreviewEntry};
use crate::code::path::PathResolver;

/// jevgrep `filesystemDefaults.dependencyDirectories`.
const DEPENDENCY_DIRECTORIES: [&str; 13] = [
    "node_modules",
    "vendor",
    "venv",
    ".venv",
    ".tox",
    "__pycache__",
    "dist",
    "build",
    "coverage",
    "target",
    ".next",
    ".nuxt",
    ".turbo",
];
/// jevgrep `filesystemDefaults.sensitiveNames` / `sensitiveSuffixes`.
const SENSITIVE_NAMES: [&str; 12] = [
    "credentials",
    "credentials.json",
    "secrets.json",
    "secrets.yaml",
    "secrets.yml",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    ".netrc",
    ".npmrc",
    ".pypirc",
];
const SENSITIVE_SUFFIXES: [&str; 4] = [".pem", ".key", ".p12", ".pfx"];

/// Entries one ask may walk.
pub const MAX_ENTRIES: usize = 100_000;
/// jevgrep `maxFileBytes`.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const PREVIEW_ENTRIES: usize = 64;
const PREVIEW_ENTRY_BYTES: usize = 4096;
const PREVIEW_FILE_BYTES: usize = 16_384;
const PREVIEW_JSON_BYTES: usize = 24_000;

static CONTROL: Lazy<regex::bytes::Regex> = Lazy::new(|| {
    regex::bytes::Regex::new(r"[\x00-\x08\x0b\x0e-\x1f\x7f]").expect("control-byte regex")
});
static PRIVATE_KEY: Lazy<regex::Regex> = Lazy::new(|| {
    regex::Regex::new(r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY-----").expect("private-key regex")
});

/// jevgrep `isSensitive`.
fn is_sensitive(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower == ".env"
        || lower.starts_with(".env.")
        || SENSITIVE_NAMES.contains(&lower.as_str())
        || SENSITIVE_SUFFIXES.iter().any(|s| lower.ends_with(s))
}

/// Node's `path.extname` of a name.
pub fn extname(name: &str) -> &str {
    let base = name.rsplit('/').next().unwrap_or(name);
    match base.rfind('.') {
        None | Some(0) => "",
        Some(i) => &base[i..],
    }
}

/// `dir` + `name` in the walk's root-relative form (`.` is the root).
pub fn join(dir: &str, name: &str) -> String {
    if dir == "." {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}

/// The eligible tree under one walk root.
#[derive(Debug)]
pub struct Tree {
    /// Canonical absolute walk root; never sent to the judge.
    pub root: PathBuf,
    /// Root-relative directory (`.` = root) → its eligible children, sorted
    /// by name.
    pub children: HashMap<String, Vec<Entry>>,
    /// The walk stopped at [`MAX_ENTRIES`].
    pub truncated: bool,
    /// Per-file read ceiling: jevgrep's 16 MiB capped by `max_read_bytes`.
    pub max_file_bytes: u64,
}

/// Walk `root` once, applying every name gate (module docs).
pub fn walk(
    resolver: &Arc<PathResolver>,
    root: &Path,
    exclude: Option<globset::GlobSet>,
    max_read_bytes: u64,
) -> Tree {
    // Naming an excluded folder as the walk root disables the default
    // excludes for that walk, as in coder::search and coder::tree.
    let use_default_excludes = !resolver.is_default_excluded_dir(root);
    let mut walker = ignore::WalkBuilder::new(root);
    walker
        .follow_links(false)
        .hidden(true)
        .parents(true)
        .ignore(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .sort_by_file_name(|a, b| a.cmp(b));
    let filter_resolver = resolver.clone();
    let filter_root = root.to_path_buf();
    walker.filter_entry(move |e| {
        if e.depth() == 0 {
            return true;
        }
        let Some(file_type) = e.file_type() else {
            return false;
        };
        let (is_dir, is_file) = (file_type.is_dir(), file_type.is_file());
        if !is_dir && !is_file {
            return false; // symlinks and special files
        }
        let abs = e.path();
        let name = e.file_name().to_string_lossy();
        if filter_resolver.is_denied(abs)
            || filter_resolver.is_non_accessible(abs)
            || is_sensitive(&name)
            || (is_dir && DEPENDENCY_DIRECTORIES.contains(&name.as_ref()))
        {
            return false;
        }
        if use_default_excludes
            && if is_dir {
                filter_resolver.is_default_excluded_dir(abs)
            } else {
                filter_resolver.is_default_excluded(abs)
            }
        {
            return false;
        }
        match (&exclude, abs.strip_prefix(&filter_root)) {
            (Some(set), Ok(rel)) => !set.is_match(rel),
            _ => true,
        }
    });

    let mut tree = Tree {
        root: root.to_path_buf(),
        children: HashMap::from([(".".to_string(), Vec::new())]),
        truncated: false,
        max_file_bytes: MAX_FILE_BYTES.min(max_read_bytes),
    };
    let mut seen = 0usize;
    for entry in walker.build().filter_map(Result::ok) {
        if entry.depth() == 0 {
            continue;
        }
        if seen >= MAX_ENTRIES {
            tree.truncated = true;
            break;
        }
        seen += 1;
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let (parent, name) = match rel.rsplit_once('/') {
            Some((parent, name)) => (parent.to_string(), name.to_string()),
            None => (".".to_string(), rel.clone()),
        };
        let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
        if is_dir {
            tree.children.entry(rel).or_default();
        }
        tree.children
            .entry(parent)
            .or_default()
            .push(Entry { name, is_dir });
    }
    tree
}

/// One read of an eligible file (jevgrep `Snapshot`).
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub path: String,
    pub source: String,
    pub content_hash: String,
}

pub enum Snap {
    Ok(Snapshot),
    /// Content gate: never scored, admitted or returned.
    Excluded,
    /// Counted as an issue of this kind.
    Issue(&'static str),
}

/// jevgrep `readSnapshot` over the walked tree: no symlink is followed, and
/// the content gates run on the exact bytes read.
pub fn read(tree: &Tree, path: &str) -> Snap {
    let abs = tree.root.join(path);
    let mut file = match open_no_follow(&abs) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Snap::Issue("changed"),
        Err(_) => return Snap::Issue("unreadable"),
    };
    match file.metadata() {
        Ok(md) if md.is_file() && md.len() <= tree.max_file_bytes => {}
        Ok(md) if md.is_file() => return Snap::Issue("unreadable"),
        _ => return Snap::Issue("changed"),
    }
    let mut bytes = Vec::new();
    if (&mut file)
        .take(tree.max_file_bytes + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Snap::Issue("unreadable");
    }
    if bytes.len() as u64 > tree.max_file_bytes {
        return Snap::Issue("unreadable");
    }
    if CONTROL.is_match(&bytes) {
        return Snap::Excluded;
    }
    let content_hash = format!("{:x}", Sha256::digest(&bytes));
    let Ok(source) = String::from_utf8(bytes) else {
        return Snap::Excluded;
    };
    if PRIVATE_KEY.is_match(&source) {
        return Snap::Excluded;
    }
    Snap::Ok(Snapshot {
        path: path.to_string(),
        source,
        content_hash,
    })
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    // NONBLOCK: a file swapped for a FIFO after the walk cannot hang the read.
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(not(unix))]
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(std::io::Error::from(std::io::ErrorKind::NotFound));
    }
    std::fs::File::open(path)
}

fn json_len<T: serde::Serialize + ?Sized>(value: &T) -> usize {
    serde_json::to_vec(value)
        .map(|v| v.len())
        .unwrap_or(usize::MAX)
}

/// retrieve.ts `previewDirectory`.
pub fn preview_directory(tree: &Tree, path: &str) -> DirectoryPreview {
    let mut preview = DirectoryPreview::default();
    let mut bytes = 0;
    for entry in tree
        .children
        .get(path)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let child = PreviewEntry {
            name: entry.name.clone(),
            kind: if entry.is_dir {
                Kind::Directory
            } else {
                Kind::File
            },
        };
        let size = json_len(&child);
        if preview.entries.len() >= PREVIEW_ENTRIES || bytes + size > PREVIEW_ENTRY_BYTES {
            preview.truncated = true;
            break;
        }
        preview.entries.push(child);
        bytes += size;
        if entry.is_dir {
            preview.sampled_directories += 1;
        } else {
            preview.sampled_files += 1;
            let extension = match extname(&entry.name) {
                "" => "[no extension]",
                extension => extension,
            };
            *preview
                .sampled_extensions
                .entry(extension.to_string())
                .or_default() += 1;
        }
    }
    preview
}

/// The longest prefix of `text` within `units` UTF-16 code units, never
/// splitting a surrogate pair (JavaScript's `slice` + high-surrogate check).
fn utf16_prefix(text: &str, units: usize) -> &str {
    let mut used = 0;
    for (index, c) in text.char_indices() {
        used += c.len_utf16();
        if used > units {
            return &text[..index];
        }
    }
    text
}

/// retrieve.ts `previewFile`: the strict-UTF-8 opening bytes, shrunk until
/// their JSON fits.
pub fn preview_file(snapshot: &Snapshot) -> FilePreview {
    let bytes = snapshot.source.as_bytes();
    let head = &bytes[..bytes.len().min(PREVIEW_FILE_BYTES)];
    // The source is valid UTF-8, so only the cut can split a character.
    let mut text = match std::str::from_utf8(head) {
        Ok(text) => text,
        Err(e) => std::str::from_utf8(&head[..e.valid_up_to()]).unwrap_or_default(),
    };
    let mut truncated = bytes.len() > PREVIEW_FILE_BYTES;
    while json_len(text) > PREVIEW_JSON_BYTES {
        let units = text.encode_utf16().count();
        text = utf16_prefix(text, units * 3 / 4);
        truncated = true;
    }
    // Later stages: the Python opening sampler (`pythonPreview`) and the
    // declaration index (`inspect`) refine truncated previews here.
    FilePreview {
        size_bytes: bytes.len(),
        extension: extname(&snapshot.path).to_string(),
        text: text.to_string(),
        preview_bytes: text.len(),
        truncated,
        range: "opening bytes".into(),
        declarations: Some(Vec::new()),
        declaration_index_truncated: Some(false),
    }
}

/// A line-aligned slice of a source (jevgrep `SourceUnit`, text mode).
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
    pub byte_start: usize,
    pub byte_end: usize,
    pub partial: bool,
}

/// source.ts `textUnits`: `[start_line, end_line]` of `source` as chunks of
/// at most `max_bytes`, cut after a newline when one fits.
pub fn text_units(
    source: &str,
    start_line: usize,
    end_line: usize,
    name: &str,
    max_bytes: usize,
    partial: bool,
) -> Vec<Unit> {
    let raw = source.as_bytes();
    let mut offsets = vec![0usize];
    let mut line_count = 0;
    for line in source.split('\n') {
        offsets.push(offsets[offsets.len() - 1] + line.len() + 1);
        line_count += 1;
    }
    let first = raw.len().min(
        start_line
            .checked_sub(1)
            .and_then(|i| offsets.get(i).copied())
            .unwrap_or(raw.len()),
    );
    let end = raw
        .len()
        .min(offsets.get(end_line).copied().unwrap_or(raw.len()));
    let mut units: Vec<Unit> = Vec::new();
    let (mut start, mut line) = (first, start_line);
    while start < end {
        let mut finish = end.min(start + max_bytes);
        if finish < end {
            while finish > start && (raw[finish] & 0xc0) == 0x80 {
                finish -= 1;
            }
            if let Some(newline) = raw[start..finish].iter().rposition(|b| *b == b'\n') {
                finish = start + newline + 1;
            }
        }
        let part = &source[start..finish];
        let newlines = part.matches('\n').count();
        units.push(Unit {
            name: name.to_string(),
            start_line: line,
            end_line: line + newlines - usize::from(part.ends_with('\n')),
            byte_start: start,
            byte_end: finish,
            partial: partial || first != start || finish != end,
        });
        line += newlines;
        start = finish;
    }
    // The final empty line has no bytes but still belongs to the snapshot.
    if raw.last() == Some(&b'\n') && end_line == line_count {
        if let Some(last) = units.last_mut() {
            last.end_line = line_count;
        }
    }
    units
}

/// source.ts `splitSource`: the whole file as `source` chunks.
pub fn split_source(source: &str, max_bytes: usize) -> Vec<Unit> {
    let line_count = source.split('\n').count();
    text_units(source, 1, line_count, "source", max_bytes, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_names_follow_jevgrep() {
        for name in [
            ".env",
            ".ENV.local",
            "credentials",
            "Secrets.yml",
            "id_ed25519",
            "x.pem",
            "a.KEY",
            ".npmrc",
        ] {
            assert!(is_sensitive(name), "{name}");
        }
        for name in ["env", ".envrc", "keys.rs", "id_rsa.pub", "secrets.toml"] {
            assert!(!is_sensitive(name), "{name}");
        }
    }

    #[test]
    fn extname_matches_node() {
        assert_eq!(extname("a/b.rs"), ".rs");
        assert_eq!(extname(".bashrc"), "");
        assert_eq!(extname("Makefile"), "");
        assert_eq!(extname("a.tar.gz"), ".gz");
    }

    #[test]
    fn split_source_cuts_after_newlines_and_keeps_the_trailing_line() {
        let source = "aaaa\nbbbb\ncccc\n";
        let units = split_source(source, 10);
        let spans: Vec<_> = units
            .iter()
            .map(|u| (u.start_line, u.end_line, &source[u.byte_start..u.byte_end]))
            .collect();
        assert_eq!(spans, [(1, 2, "aaaa\nbbbb\n"), (3, 4, "cccc\n")]);
        assert!(units.iter().all(|u| u.partial));
        let whole = split_source("x\ny", 100);
        assert_eq!((whole[0].start_line, whole[0].end_line), (1, 2));
        assert!(!whole[0].partial);
        // a multibyte character is never split
        let wide = "ééééé";
        assert!(split_source(wide, 5)
            .iter()
            .all(|u| wide.is_char_boundary(u.byte_start) && wide.is_char_boundary(u.byte_end)));
    }

    #[test]
    fn file_previews_shrink_until_their_json_fits() {
        let snapshot = |source: String| Snapshot {
            path: "a.rs".into(),
            source,
            content_hash: String::new(),
        };
        let small = preview_file(&snapshot("fn a() {}\n".into()));
        assert!(!small.truncated);
        assert_eq!(small.text, "fn a() {}\n");
        assert_eq!(small.extension, ".rs");
        // 16 KiB of quotes escapes to 32 KiB of JSON: shrunk by 3/4 steps
        let quotes = preview_file(&snapshot("\"".repeat(20_000)));
        assert!(quotes.truncated);
        assert!(json_len(&quotes.text) <= PREVIEW_JSON_BYTES);
        assert_eq!(quotes.text.len(), 16_384 * 3 / 4 * 3 / 4);
        // a cut through a multibyte character drops the partial character
        let wide = preview_file(&snapshot("é".repeat(9_000)));
        assert_eq!(wide.text.len(), 16_384);
        assert_eq!(wide.preview_bytes, 16_384);
    }
}
