//! The one filesystem pass behind `coder::find-relevant`, its egress gates
//! and the previews the judge navigates by.
//!
//! Ported from dzhng/jevgrep (MIT, Copyright (c) 2026 David Zhang), commit
//! 82ef1fd: `packages/core/src/filesystem.ts` (name and content policy),
//! `retrieve.ts` `previewDirectory`/`previewFile` and `source.ts`
//! `splitSource`.
//!
//! A directory is listed only when navigation reaches it (jevgrep's
//! per-directory cursors), by a one-level `ignore` walk that loads the
//! ignore files of every ancestor, as one whole-tree walk would (inside a
//! Git work tree, only those up to its top, as git reads them). Every gate
//! runs there, so a pruned entry never shows up in a preview either:
//! - the jail's protections: operator denylist, `non_accessible_globs`
//!   (REDACTION INVARIANT), `default_exclude_globs`;
//! - `.gitignore`/`.ignore` (also outside a Git checkout), hidden entries
//!   (any dot-name below the walk root, even one an ignore file
//!   whitelists), symlinks and special files;
//! - jevgrep's dependency directories and credential-like names;
//! - the caller's `exclude_globs`.
//!
//! The walk root itself is checked once, before the walk ([`ignored`] and
//! the caller's hidden check).
//!
//! Content gates (control bytes, invalid UTF-8, a PRIVATE KEY block, a
//! PuTTY or age secret key) run on every read: such a file is never scored,
//! admitted or returned, though its name still shows in its folder's preview
//! (as in jevgrep, which previews names only).
//!
//! Nothing under a `.git` directory is ever walked or read, and a read
//! re-checks that its path still resolves, through no symlink, to the file
//! it opened: a directory swapped for a link after the walk cannot leak
//! bytes from outside the root.

use std::cmp::Ordering;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};

use super::passes;
use super::prompts::{Declaration, DirectoryPreview, FilePreview, Kind, PreviewEntry, SourceRange};
use super::units::{self, MAX_PARSE_BYTES};
use crate::code::functions::search::relative_to;
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
/// jevgrep `filesystemDefaults.sensitiveNames` / `sensitiveSuffixes`, plus
/// PuTTY keys, Terraform state, Java keystores and KeePass databases.
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
const SENSITIVE_SUFFIXES: [&str; 10] = [
    ".pem",
    ".key",
    ".p12",
    ".pfx",
    ".ppk",
    ".tfstate",
    ".tfstate.backup",
    ".jks",
    ".keystore",
    ".kdbx",
];

/// Entries one ask may list (jevgrep's `entriesSeen` cap), charged as
/// discovery lists a directory; a preview or lookup lists at most this many.
pub const MAX_ENTRIES: usize = 100_000;
/// jevgrep `maxFileBytes`.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const PREVIEW_ENTRIES: usize = 64;
const PREVIEW_ENTRY_BYTES: usize = 4096;
const PREVIEW_FILE_BYTES: usize = 16_384;
const PREVIEW_JSON_BYTES: usize = 24_000;
/// jevgrep's declaration-index cap; a small judge window lowers it.
pub const PREVIEW_INDEX_JSON_BYTES: usize = 32_000;

static CONTROL: Lazy<regex::bytes::Regex> = Lazy::new(|| {
    regex::bytes::Regex::new(r"[\x00-\x08\x0b\x0e-\x1f\x7f]").expect("control-byte regex")
});
static PRIVATE_KEY: Lazy<regex::Regex> = Lazy::new(|| {
    // jevgrep's pattern, plus the ` BLOCK` of an ASCII-armored PGP key.
    regex::Regex::new(r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?-----")
        .expect("private-key regex")
});
/// Unarmored secret keys by shape, not a mention of their format: a PuTTY
/// key file header and an age identity (bech32, post-quantum too).
/// Unanchored, so a key pasted into a `.env` or YAML value still counts.
static SECRET_KEY: Lazy<regex::Regex> = Lazy::new(|| {
    regex::Regex::new(r"PuTTY-User-Key-File-\d+: |AGE-SECRET-KEY-(?:PQ-)?1[02-9AC-HJ-NP-Z]{58,}")
        .expect("secret-key regex")
});

/// jevgrep `isSensitive`.
pub fn is_sensitive(name: &str) -> bool {
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

/// jevgrep sorts with ICU `localeCompare`: case-insensitive first, lowercase
/// before uppercase on a tie.
// ponytail: approximation, not ICU collation (punctuation and accents order
// differently); use an ICU collator if non-ASCII batch order ever matters.
pub fn locale_cmp(a: &str, b: &str) -> Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| b.cmp(a))
}

/// jevgrep `hardExcluded`: git metadata never leaves the host.
pub fn in_git_dir(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == ".git")
}

/// The top of the Git work tree holding `path`: its nearest ancestor (or
/// itself) with a `.git` entry, a linked worktree's `.git` file included.
pub fn git_top(path: &Path) -> Option<&Path> {
    path.ancestors().find(|dir| dir.join(".git").exists())
}

/// Whether `top` is a linked worktree: its `.git` is a regular file whose
/// `gitdir:` admin folder in a main repository outside `top` points back at
/// it (a submodule, a symlink, a planted pointer or an admin folder planted
/// inside `top` is not one).
pub fn linked_worktree(top: &Path) -> bool {
    std::fs::symlink_metadata(top.join(".git")).is_ok_and(|md| md.is_file())
        && crate::exec::confine::repo_git_dir(top).is_some_and(|common| !common.starts_with(top))
}

/// Whether `top` is the work tree of a repository: its git dir holds a
/// `HEAD` file and an `objects` folder (an empty `.git` is not one).
pub fn repository(top: &Path) -> bool {
    crate::exec::confine::repo_git_dir(top)
        .is_some_and(|git| git.join("HEAD").is_file() && git.join("objects").is_dir())
}

/// A one-level walk of `dir` under its ignore files and every ancestor's;
/// `in_git` stops at the work tree's top, as git does.
fn one_level(dir: &Path, in_git: bool) -> ignore::WalkBuilder {
    let mut walker = ignore::WalkBuilder::new(dir);
    walker
        .max_depth(Some(1))
        .follow_links(false)
        .hidden(true)
        .parents(true)
        .ignore(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(in_git);
    walker
}

/// True when ignore rules leave out `path` or a folder between it and
/// `bound`, each folder judged as a listing of its parent would (inside a
/// work tree or not): an ignore pattern matches only the entry it names,
/// so a walk started inside an ignored folder would see none of it.
/// Blocking.
pub fn ignored(bound: &Path, path: &Path) -> bool {
    path.ancestors()
        .take_while(|dir| *dir != bound && dir.starts_with(bound))
        .any(|dir| {
            let (Some(parent), Some(name)) = (dir.parent(), dir.file_name()) else {
                return false;
            };
            let name = name.to_os_string();
            let mut walker = one_level(parent, git_top(parent).is_some());
            walker
                .hidden(false)
                .filter_entry(move |e| e.depth() == 0 || e.file_name() == name);
            // Errors before the parent itself come from ancestors' ignore
            // files (a line git reads but globset cannot parse; the rest
            // still apply). A parent that cannot be listed proves nothing;
            // the walk reports what it cannot read.
            let mut opened = false;
            !walker.build().any(|e| match e {
                Ok(e) => {
                    opened |= e.depth() == 0;
                    e.depth() == 1
                }
                Err(_) => opened,
            })
        })
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

/// The eligible tree under one walk root, listed a directory at a time.
pub struct Tree {
    /// Canonical absolute walk root; never sent to the judge.
    pub root: PathBuf,
    resolver: Arc<PathResolver>,
    exclude: Option<globset::GlobSet>,
    /// What `exclude` matches relative to outside every configured root,
    /// as in coder::search.
    anchor: PathBuf,
    /// The root is inside a Git work tree ([`one_level`]).
    in_git: bool,
    /// Naming an excluded folder as the walk root disables the default
    /// excludes for that walk, as in coder::search and coder::tree.
    default_excludes: bool,
    /// Per-file read ceiling: jevgrep's 16 MiB capped by `max_read_bytes`.
    pub max_file_bytes: u64,
}

/// One directory's eligible children.
#[derive(Debug, Default)]
pub struct Listing {
    /// Sorted by name.
    pub entries: Vec<Entry>,
    /// The listing stopped at its limit with entries left unread.
    pub truncated: bool,
    /// The directory could not be listed, or not all of it.
    pub unreadable: bool,
}

impl Tree {
    pub fn new(
        resolver: &Arc<PathResolver>,
        root: &Path,
        exclude: Option<globset::GlobSet>,
        anchor: &Path,
        max_read_bytes: u64,
    ) -> Self {
        Self {
            root: root.to_path_buf(),
            resolver: resolver.clone(),
            exclude,
            anchor: anchor.to_path_buf(),
            in_git: git_top(root).is_some(),
            default_excludes: !resolver.is_default_excluded_dir(root),
            max_file_bytes: MAX_FILE_BYTES.min(max_read_bytes),
        }
    }

    /// The eligible children of `dir` (root-relative, `.` = the root) under
    /// every name gate (module docs): the first `limit` in directory order,
    /// sorted by name. Blocking.
    pub fn list(&self, dir: &str, limit: usize) -> Listing {
        if dir == "." {
            self.list_path(&self.root, limit)
        } else {
            self.list_path(&self.root.join(dir), limit)
        }
    }

    /// [`Tree::list`] of a canonical absolute folder, which may sit above
    /// the root. Blocking.
    pub fn list_path(&self, path: &Path, limit: usize) -> Listing {
        let mut walker = one_level(path, self.in_git);
        let (resolver, exclude, anchor) = (
            self.resolver.clone(),
            self.exclude.clone(),
            self.anchor.clone(),
        );
        let default_excludes = self.default_excludes;
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
            // filesystem.ts 319: a dot-name is hidden whatever the ignore
            // files say (the walker's own hidden check yields to a
            // whitelist such as `!.kube/`); this also covers `.git`.
            if name.starts_with('.')
                || resolver.is_denied(abs)
                || resolver.is_non_accessible(abs)
                || is_sensitive(&name)
                || (is_dir && DEPENDENCY_DIRECTORIES.contains(&name.as_ref()))
            {
                return false;
            }
            if default_excludes
                && if is_dir {
                    resolver.is_default_excluded_dir(abs)
                } else {
                    resolver.is_default_excluded(abs)
                }
            {
                return false;
            }
            // coder::search's relative form; a directory also matches as
            // `rel/`, so `gen/` and `gen/**` prune the folder itself, not
            // just its contents.
            let Some(set) = &exclude else {
                return true;
            };
            resolver
                .relative(abs)
                .or_else(|| relative_to(&anchor, abs))
                .is_none_or(|rel| {
                    !(set.is_match(&rel) || is_dir && set.is_match(format!("{rel}/")))
                })
        });

        let mut listing = Listing::default();
        // Errors before the directory itself come from ancestors' ignore
        // files; after it, from reading the directory.
        let mut opened = false;
        for entry in walker.build() {
            let Ok(entry) = entry else {
                listing.unreadable |= opened;
                continue;
            };
            if entry.depth() == 0 {
                opened = true;
                continue;
            }
            if listing.entries.len() >= limit {
                listing.truncated = true;
                break;
            }
            listing.entries.push(Entry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir: entry.file_type().is_some_and(|t| t.is_dir()),
            });
        }
        listing.unreadable |= !opened;
        listing.entries.sort_by(|a, b| locale_cmp(&a.name, &b.name));
        listing
    }
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

/// jevgrep `readSnapshot` over the walked tree: no symlink is followed, the
/// path must still name the file read, and the content gates run on the
/// exact bytes read.
pub fn read(tree: &Tree, path: &str) -> Snap {
    let abs = tree.root.join(path);
    let mut file = match open_no_follow(&abs) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Snap::Issue("changed"),
        Err(_) => return Snap::Issue("unreadable"),
    };
    let opened = match file.metadata() {
        Ok(md) if md.is_file() && md.len() <= tree.max_file_bytes => md,
        Ok(md) if md.is_file() => return Snap::Issue("resource_limit"),
        _ => return Snap::Issue("changed"),
    };
    let mut bytes = Vec::new();
    if (&mut file)
        .take(tree.max_file_bytes + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Snap::Issue("unreadable");
    }
    if bytes.len() as u64 > tree.max_file_bytes {
        return Snap::Issue("resource_limit");
    }
    if !stable(&abs, &opened) {
        return Snap::Issue("changed");
    }
    if CONTROL.is_match(&bytes) {
        return Snap::Excluded;
    }
    let content_hash = format!("{:x}", Sha256::digest(&bytes));
    let Ok(source) = String::from_utf8(bytes) else {
        return Snap::Excluded;
    };
    if PRIVATE_KEY.is_match(&source) || SECRET_KEY.is_match(&source) {
        return Snap::Excluded;
    }
    Snap::Ok(Snapshot {
        path: path.to_string(),
        source,
        content_hash,
    })
}

/// jevgrep `stable`: `abs` (under the canonical root) still resolves to
/// itself, so no ancestor became a symlink, and still names the opened
/// file. The walk's gates were checked on this exact path, so passing this
/// means they still hold.
fn stable(abs: &Path, opened: &std::fs::Metadata) -> bool {
    if std::fs::canonicalize(abs).ok().as_deref() != Some(abs) {
        return false;
    }
    #[cfg(unix)]
    let same = {
        use std::os::unix::fs::MetadataExt;
        std::fs::symlink_metadata(abs)
            .is_ok_and(|now| now.dev() == opened.dev() && now.ino() == opened.ino())
    };
    #[cfg(not(unix))]
    let same = {
        let _ = opened;
        true
    };
    same
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

pub fn json_len<T: serde::Serialize + ?Sized>(value: &T) -> usize {
    serde_json::to_vec(value)
        .map(|v| v.len())
        .unwrap_or(usize::MAX)
}

/// retrieve.ts `previewDirectory`: `None` when `path` cannot be listed.
/// The entries are the first by name, not jevgrep's first page in directory
/// order. Blocking.
pub fn preview_directory(tree: &Tree, path: &str) -> Option<DirectoryPreview> {
    let listing = tree.list(path, MAX_ENTRIES);
    if listing.unreadable {
        return None;
    }
    let mut preview = DirectoryPreview {
        truncated: listing.truncated,
        ..DirectoryPreview::default()
    };
    let mut bytes = 0;
    for entry in &listing.entries {
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
    Some(preview)
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
/// their JSON fits, or for a truncated Python file the `query`-aware
/// sample ([`passes::python_preview`]); a truncated source file also lists
/// its declarations while the preview's JSON stays within `index_bytes`
/// ([`PREVIEW_INDEX_JSON_BYTES`] in jevgrep).
pub fn preview_file(snapshot: &Snapshot, query: &str, index_bytes: usize) -> FilePreview {
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
    let mut preview = FilePreview {
        size_bytes: bytes.len(),
        extension: extname(&snapshot.path).to_string(),
        text: text.to_string(),
        preview_bytes: text.len(),
        truncated,
        range: "opening bytes".into(),
        declarations: Some(Vec::new()),
        declaration_index_truncated: Some(false),
    };
    if truncated && units::is_python(&snapshot.path) && bytes.len() <= MAX_PARSE_BYTES {
        let sampled =
            passes::python_preview(&snapshot.source, query, PREVIEW_FILE_BYTES).filter(|text| {
                !text.is_empty()
                    && text.len() <= PREVIEW_FILE_BYTES
                    && json_len(text.as_str()) <= PREVIEW_JSON_BYTES
            });
        if let Some(text) = sampled {
            preview.preview_bytes = text.len();
            preview.text = text;
            preview.range = "sampled source ranges".into();
        }
    }
    if truncated && units::supported(&snapshot.path) && bytes.len() <= MAX_PARSE_BYTES {
        let syntax = units::inspect(
            &snapshot.path,
            &snapshot.source,
            bytes.len().max(4),
            MAX_PARSE_BYTES,
        );
        let all: Vec<Declaration> = syntax
            .units
            .into_iter()
            .filter(|unit| !unit.partial)
            .map(|unit| Declaration {
                name: unit.name,
                start_line: unit.start_line,
                end_line: unit.end_line,
            })
            .collect();
        let mut fits = |keep: usize, truncated: bool| {
            preview.declarations = Some(all[..keep].to_vec());
            preview.declaration_index_truncated = Some(truncated);
            json_len(&preview) <= index_bytes
        };
        if !fits(all.len(), false) && !all.is_empty() {
            // retrieve.ts pops one entry at a time until the preview fits
            // (quadratic: 40k declarations took 23 s). The JSON grows with
            // the kept prefix, so a binary search keeps the same prefix:
            // `lo` is kept (an empty index stops the pops), `hi` is not.
            let (mut lo, mut hi) = (0, all.len());
            while hi - lo > 1 {
                let mid = (lo + hi) / 2;
                if fits(mid, true) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            fits(lo, true);
        }
    }
    preview
}

/// A line-aligned slice of a source (jevgrep `SourceUnit`).
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
    pub byte_start: usize,
    pub byte_end: usize,
    pub partial: bool,
    /// Line ranges of the enclosing declarations' headers.
    pub owner_headers: Vec<SourceRange>,
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
            owner_headers: Vec::new(),
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
            "deploy.ppk",
            "terraform.tfstate",
            "terraform.tfstate.backup",
            "release.jks",
            "debug.keystore",
            "vault.kdbx",
        ] {
            assert!(is_sensitive(name), "{name}");
        }
        for name in ["env", ".envrc", "keys.rs", "id_rsa.pub", "secrets.toml"] {
            assert!(!is_sensitive(name), "{name}");
        }
    }

    #[test]
    fn an_unparsable_ignore_line_leaves_the_other_rules_in_force() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().canonicalize().unwrap();
        for (path, text) in [
            (".git/HEAD", "ref: refs/heads/main\n"),
            // git reads `tmp{` as a literal; globset cannot parse it
            (".gitignore", "tmp{\n"),
            ("app/.gitignore", "build/\n"),
            ("app/build/keys/k.txt", "x"),
        ] {
            std::fs::create_dir_all(repo.join(path).parent().unwrap()).unwrap();
            std::fs::write(repo.join(path), text).unwrap();
        }
        assert!(ignored(&repo, &repo.join("app/build/keys")));
        assert!(!ignored(&repo, &repo.join("app")));
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
        let small = preview_file(
            &snapshot("fn a() {}\n".into()),
            "",
            PREVIEW_INDEX_JSON_BYTES,
        );
        assert!(!small.truncated);
        assert_eq!(small.text, "fn a() {}\n");
        assert_eq!(small.extension, ".rs");
        // 16 KiB of quotes escapes to 32 KiB of JSON: shrunk by 3/4 steps
        let quotes = preview_file(&snapshot("\"".repeat(20_000)), "", PREVIEW_INDEX_JSON_BYTES);
        assert!(quotes.truncated);
        assert!(json_len(&quotes.text) <= PREVIEW_JSON_BYTES);
        assert_eq!(quotes.text.len(), 16_384 * 3 / 4 * 3 / 4);
        // a cut through a multibyte character drops the partial character
        let wide = preview_file(&snapshot("é".repeat(9_000)), "", PREVIEW_INDEX_JSON_BYTES);
        assert_eq!(wide.text.len(), 16_384);
        assert_eq!(wide.preview_bytes, 16_384);
    }

    #[test]
    fn truncated_source_previews_index_their_declarations_within_their_budget() {
        let snapshot = |path: &str, source: String| Snapshot {
            path: path.into(),
            source,
            content_hash: String::new(),
        };
        let small = preview_file(
            &snapshot("a.rs", "fn a() {}\n".into()),
            "",
            PREVIEW_INDEX_JSON_BYTES,
        );
        assert_eq!(small.declarations, Some(Vec::new()));
        let source: String = (0..2_000).map(|i| format!("fn f{i:04}() {{}}\n")).collect();
        let big = preview_file(
            &snapshot("a.rs", source.clone()),
            "",
            PREVIEW_INDEX_JSON_BYTES,
        );
        let declarations = big.declarations.as_ref().unwrap();
        assert_eq!(
            declarations[1],
            Declaration {
                name: "f0001".into(),
                start_line: 2,
                end_line: 2
            }
        );
        assert!(declarations.len() < 2_000);
        assert_eq!(big.declaration_index_truncated, Some(true));
        assert!(json_len(&big) <= PREVIEW_INDEX_JSON_BYTES);
        // the longest prefix that fits, as retrieve.ts's pop loop keeps
        let mut longer = big.clone();
        longer.declarations.as_mut().unwrap().push(Declaration {
            name: format!("f{:04}", declarations.len()),
            start_line: declarations.len() + 1,
            end_line: declarations.len() + 1,
        });
        assert!(json_len(&longer) > PREVIEW_INDEX_JSON_BYTES);
        // a smaller budget keeps a shorter prefix
        let small = preview_file(&snapshot("a.rs", source.clone()), "", 20_000);
        let kept = small.declarations.as_ref().unwrap().len();
        assert!(0 < kept && kept < declarations.len());
        assert!(json_len(&small) <= 20_000);
        assert_eq!(small.declarations.unwrap()[..], declarations[..kept]);
        // unsupported languages and text fallbacks list none
        let text = preview_file(&snapshot("a.txt", source), "", PREVIEW_INDEX_JSON_BYTES);
        assert_eq!(text.declarations, Some(Vec::new()));
    }
}
