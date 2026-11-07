//! Git-tree snapshots of a workspace at turn boundaries.
//!
//! The hooks keep a pre-image only for writes that go through a shell or
//! coder function; a `sed -i`, a `git apply` or a formatter run inside
//! `shell::exec` has none, and a review of it fell back to the last commit.
//! A snapshot of the whole root when a turn starts, and another when it
//! ends, gives every file the turn is credited with an exact before and
//! after, however the bytes got there. Which files a turn is credited with
//! is still decided by the hooks, the watcher and the claims in `turns.rs`;
//! a tree never says who wrote.
//!
//! Snapshots live in a private bare repository per root under the turn
//! store, never in the user's `.git`: `GIT_DIR` points there,
//! `GIT_WORK_TREE` at the root, `GIT_INDEX_FILE` at one index per session,
//! so two sessions in one root never contend for an index lock. The object
//! store borrows the user's objects through `objects/info/alternates`, so
//! an unchanged file costs nothing and a changed one is one blob, later
//! delta-packed by the repository's own `gc --auto`. Trees are pinned under
//! `refs/turns/<session>/<turn>/{before,after}` and dropped with the turn.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[cfg(test)]
use crate::config::DEFAULT_SNAPSHOT_MAX_FILE_BYTES;

/// Untracked paths a snapshot never takes, on top of the root's own
/// ignore rules: dependency trees, build output, and the engine's own
/// runtime state. A file tracked despite a rule here is seeded into the
/// index from the root's HEAD and refreshed like any other.
const EXCLUDES: &[&str] = &[
    ".git/",
    // The engine's runtime state under a compose root, including sparse VM
    // disk images. These are rewritten between turns, so `add -A` re-reads
    // them every time: a 16 GiB `upper.ext4` cost ~50s of hashing per turn.
    // Only the runtime subtree — the rest of `.iii/` is project configuration.
    ".iii/compose/",
    "node_modules/",
    "target/",
    "dist/",
    "build/",
    "out/",
    "vendor/",
    "__pycache__/",
    ".venv/",
    ".DS_Store",
];

/// First line of every repository's `info/exclude`. Changing it migrates
/// every existing repository on its next open (see `apply_rules`), as a
/// change to the rules themselves does.
const RULES_VERSION: &str = "# ide turn snapshots, rules v2";

/// Free space a snapshot needs on the turn store's disk. Below it the snapshot
/// is skipped: a turn without trees beats a disk at 0 B, where every worker of
/// the stack starts failing its writes.
// ponytail: a fixed floor; a share of the disk if small volumes need more.
pub const MIN_FREE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// One side of a file in a tree.
#[derive(Debug)]
pub enum Body {
    /// The path is not in the tree, and the snapshot would have taken it:
    /// it did not exist when the tree was written.
    Missing,
    /// The path is not in the tree because snapshots leave it out (an
    /// exclude rule, or bigger than the snapshot cap), so the tree says
    /// nothing about it. Never read as "did not exist": a revert would
    /// delete a file the turn only modified.
    Excluded,
    /// The blob is larger than the caller's per-file cap.
    TooLarge,
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Before,
    After,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Before => "before",
            Side::After => "after",
        }
    }
}

/// `refs/turns/<session>/<turn>/<side>`, with the ids reduced to
/// characters a ref name accepts.
pub fn ref_name(session_stem: &str, turn_id: &str, side: Side) -> String {
    let safe = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect()
    };
    format!(
        "refs/turns/{}/{}/{}",
        safe(session_stem),
        safe(turn_id),
        side.as_str()
    )
}

/// The private repository that photographs one root.
pub struct SnapshotRepo {
    git_dir: PathBuf,
    root: PathBuf,
    /// Digest of the exclude rules, part of every session index's name: an
    /// index built under other rules still holds paths those rules now
    /// leave out (`add -A` refreshes a path already in the index, ignored
    /// or not), so it is replaced rather than reused.
    rules_tag: String,
}

impl SnapshotRepo {
    /// Open the repository for `root`, creating it on first use. `store_dir`
    /// is the turn store itself, kept out of the picture when it sits under
    /// the root, like the Compose project's runtime state (`III_COMPOSE_DIR`).
    /// None when the root is not a directory or git is not here.
    pub async fn open(repos_dir: &Path, root: &Path, store_dir: &Path) -> Option<Self> {
        let compose_dir = std::env::var_os(iii_worker_paths::COMPOSE_DIR_ENV)
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from);
        Self::open_with(repos_dir, root, store_dir, compose_dir.as_deref()).await
    }

    /// [`Self::open`] with the Compose project directory given.
    async fn open_with(
        repos_dir: &Path,
        root: &Path,
        store_dir: &Path,
        compose_dir: Option<&Path>,
    ) -> Option<Self> {
        let root = tokio::fs::canonicalize(root).await.ok()?;
        if !root.is_dir() {
            return None;
        }
        let store_dir = canonical_or_given(store_dir).await;
        let compose_dir = match compose_dir {
            Some(dir) => Some(canonical_or_given(dir).await),
            None => None,
        };
        let rules = exclude_rules(&root, &store_dir, compose_dir.as_deref());
        let key = format!("{:x}", Sha256::digest(root.to_string_lossy().as_bytes()));
        let repo = Self {
            git_dir: repos_dir.join(&key[..16]),
            root,
            rules_tag: format!("{:x}", Sha256::digest(rules.as_bytes()))[..8].to_string(),
        };
        if !repo.git_dir.join("HEAD").is_file() {
            tokio::fs::create_dir_all(&repo.git_dir).await.ok()?;
            let dir = repo.git_dir.to_string_lossy().into_owned();
            repo.plain(&["init", "-q", "--bare", &dir]).await.ok()?;
            let _ = repo
                .plain(&["--git-dir", &dir, "config", "gc.auto", "256"])
                .await;
        }
        repo.apply_rules(&rules).await;
        // Refreshed on every open: a root may become a repository later.
        if let Ok(out) = repo.plain(&["rev-parse", "--git-path", "objects"]).await {
            let objects = Path::new(out.trim());
            let objects = if objects.is_absolute() {
                objects.to_path_buf()
            } else {
                repo.root.join(objects)
            };
            if objects.is_dir() {
                let _ = tokio::fs::create_dir_all(repo.git_dir.join("objects/info")).await;
                let _ = tokio::fs::write(
                    repo.git_dir.join("objects/info/alternates"),
                    format!("{}\n", objects.to_string_lossy()),
                )
                .await;
            }
        }
        Some(repo)
    }

    /// The canonical root the trees describe.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Write `rules` to `info/exclude` when they changed, so a repository
    /// created by an older build follows the current ones. The session
    /// indexes built under the old rules go with them, and the settings
    /// below are applied on the same change: the blobs of dropped turns
    /// leave at the next `gc --auto` instead of after git's default two
    /// weeks (the hour still covers a tree written but not yet pinned by its
    /// ref), and the untracked cache spares `add -A` re-reading unchanged
    /// directories (~120 → ~80 ms per snapshot of a 12k-file checkout).
    async fn apply_rules(&self, rules: &str) {
        let path = self.git_dir.join("info/exclude");
        if tokio::fs::read_to_string(&path).await.ok().as_deref() == Some(rules) {
            return;
        }
        let _ = tokio::fs::create_dir_all(self.git_dir.join("info")).await;
        let _ = tokio::fs::write(&path, rules).await;
        let dir = self.git_dir.to_string_lossy().into_owned();
        for (key, value) in [
            ("gc.pruneExpire", "1.hour.ago"),
            ("core.untrackedCache", "true"),
        ] {
            let _ = self.plain(&["--git-dir", &dir, "config", key, value]).await;
        }
        let current = format!("-{}", self.rules_tag);
        if let Ok(mut entries) = tokio::fs::read_dir(&self.git_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("index-")
                    && !name.ends_with(&current)
                    && !name.ends_with(".lock")
                {
                    let _ = tokio::fs::remove_file(entry.path()).await;
                }
            }
        }
    }

    /// The session's index under the current exclude rules.
    fn index_path(&self, session_stem: &str) -> PathBuf {
        self.git_dir
            .join(format!("index-{session_stem}-{}", self.rules_tag))
    }

    /// The tree of the root as it is right now, written through the
    /// session's own index. Files over `max_file_bytes` are left out, and
    /// nothing is written while the turn store's disk has less than
    /// `min_free_bytes` free ([`MIN_FREE_BYTES`] outside tests).
    pub async fn snapshot(
        &self,
        session_stem: &str,
        max_file_bytes: u64,
        min_free_bytes: u64,
    ) -> Result<String, String> {
        if let Some(free) = free_bytes(&self.git_dir) {
            if free < min_free_bytes {
                return Err(format!(
                    "skipped: {} MiB free on the turn store's disk, {} MiB needed",
                    free >> 20,
                    min_free_bytes >> 20
                ));
            }
        }
        let index = self.index_path(session_stem);
        if !index.is_file() {
            // Seed from the root's HEAD so files tracked despite an ignore
            // rule are known to the index; `add -A` then refreshes them like
            // any other. A root that is a subdirectory of its repository
            // seeds from that subtree.
            let prefix = self
                .plain(&["rev-parse", "--show-prefix"])
                .await
                .map(|p| p.trim().trim_end_matches('/').to_string())
                .unwrap_or_default();
            let spec = if prefix.is_empty() {
                "HEAD^{tree}".to_string()
            } else {
                format!("HEAD:{prefix}")
            };
            if let Ok(tree) = self.plain(&["rev-parse", &spec]).await {
                let _ = self.git(Some(&index), &["read-tree", tree.trim()]).await;
                // A seeded entry carries no stat data, so the size check's
                // `ls-files -m` would hash every tracked file and `add -A`
                // hash them again. One refresh records the stats instead
                // (exit 1 just says some files differ).
                let _ = self
                    .command(Some(&index))
                    .args(["update-index", "-q", "--refresh"])
                    .stdin(Stdio::null())
                    .output()
                    .await;
            }
        }
        // Files over the cap stay out of the tree, and out of the index too:
        // an entry left there by the HEAD seed or an earlier snapshot would
        // keep its old body in the tree.
        let too_large = self.too_large(&index, max_file_bytes).await;
        let mut pathspecs: Vec<Vec<u8>> = Vec::new();
        if !too_large.is_empty() {
            let literal: Vec<Vec<u8>> = too_large
                .iter()
                .map(|rel| [b":(literal)".as_slice(), rel].concat())
                .collect();
            let removed = self
                .run_with_paths(
                    &index,
                    // `-f`: the index is the snapshot's own, and git otherwise
                    // refuses an entry staged differently from the file.
                    &["rm", "--cached", "-f", "-q", "--ignore-unmatch"],
                    &literal,
                )
                .await?;
            // A failed removal would leave the old bodies in the tree.
            if !removed.status.success() {
                return Err(String::from_utf8_lossy(&removed.stderr).trim().to_string());
            }
            pathspecs.push(b".".to_vec());
            pathspecs.extend(
                too_large
                    .iter()
                    .map(|rel| [b":(exclude,literal)".as_slice(), rel].concat()),
            );
            tracing::debug!(
                files = too_large.len(),
                max_file_bytes,
                "turn snapshot: files over the size cap left out"
            );
        }
        // A path git cannot index (a nested repository with no commit checked
        // out) made `add -A` abort without writing the index, so every later
        // snapshot of the session hashed the whole tree again: 1-2.5 s per
        // hook on a large checkout. `--ignore-errors` writes the index anyway
        // and exits 1; the tree then lacks only the paths git skipped.
        let added = self
            .run_with_paths(&index, &["add", "-A", "--ignore-errors"], &pathspecs)
            .await?;
        let stderr = String::from_utf8_lossy(&added.stderr);
        match added.status.code() {
            Some(0) => {}
            Some(1) => {
                tracing::debug!(skipped = %stderr.trim(), "turn snapshot: paths git could not index")
            }
            _ => return Err(stderr.trim().to_string()),
        }
        let tree = self.git(Some(&index), &["write-tree"]).await?;
        Ok(String::from_utf8_lossy(&tree).trim().to_string())
    }

    pub async fn keep_ref(&self, name: &str, oid: &str) {
        if let Err(e) = self.git(None, &["update-ref", name, oid]).await {
            tracing::warn!(error = %e, name, "turn snapshot: ref not kept");
        }
    }

    pub async fn drop_ref(&self, name: &str) {
        let _ = self.git(None, &["update-ref", "-d", name]).await;
    }

    /// Pack loose objects and drop what no turn pins any more, when the
    /// repository's own thresholds say so.
    pub async fn gc_auto(&self) {
        let _ = self.git(None, &["gc", "--auto", "-q"]).await;
    }

    /// The bodies of `rels` in `tree`, read in two batched calls: one
    /// listing, one `cat-file --batch`. Blobs over `max_bytes` are not read.
    pub async fn bodies(
        &self,
        tree: &str,
        rels: &[String],
        max_bytes: u64,
        snapshot_max_file_bytes: u64,
    ) -> HashMap<String, Body> {
        let mut out: HashMap<String, Body> = HashMap::new();
        let Ok(listing) = self.git(None, &["ls-tree", "-r", "-l", "-z", tree]).await else {
            return out;
        };
        let wanted: HashSet<&str> = rels.iter().map(String::as_str).collect();
        let mut to_read: Vec<(String, String)> = Vec::new();
        for entry in listing.split(|b| *b == 0) {
            let entry = String::from_utf8_lossy(entry);
            let Some((meta, path)) = entry.split_once('\t') else {
                continue;
            };
            if !wanted.contains(path) {
                continue;
            }
            let mut fields = meta.split_whitespace();
            let (Some(_mode), Some(kind), Some(oid), Some(size)) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            if kind != "blob" {
                continue;
            }
            let size: u64 = size.parse().unwrap_or(0);
            if size > max_bytes {
                out.insert(path.to_string(), Body::TooLarge);
            } else {
                to_read.push((path.to_string(), oid.to_string()));
            }
        }
        let absent: Vec<&String> = rels
            .iter()
            .filter(|rel| !out.contains_key(*rel) && !to_read.iter().any(|(p, _)| p == *rel))
            .collect();
        let excluded = self.left_out(&absent, snapshot_max_file_bytes).await;
        for rel in absent {
            let body = if excluded.contains(rel.as_str()) {
                Body::Excluded
            } else {
                Body::Missing
            };
            out.insert(rel.clone(), body);
        }
        if to_read.is_empty() {
            return out;
        }
        let mut child = match self
            .command(None)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                tracing::warn!(error = %e, "turn snapshot: cat-file failed to start");
                return out;
            }
        };
        let request: String = to_read.iter().map(|(_, oid)| format!("{oid}\n")).collect();
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(request.as_bytes()).await;
        }
        let Ok(result) = child.wait_with_output().await else {
            return out;
        };
        let mut blobs: HashMap<String, Vec<u8>> = HashMap::new();
        let mut cursor = 0usize;
        let bytes = &result.stdout;
        while cursor < bytes.len() {
            let Some(nl) = bytes[cursor..].iter().position(|b| *b == b'\n') else {
                break;
            };
            let header = String::from_utf8_lossy(&bytes[cursor..cursor + nl]).into_owned();
            cursor += nl + 1;
            let mut fields = header.split_whitespace();
            let (Some(oid), Some(kind), size) = (fields.next(), fields.next(), fields.next())
            else {
                break;
            };
            if kind == "missing" {
                continue;
            }
            let size: usize = size.and_then(|s| s.parse().ok()).unwrap_or(0);
            let end = (cursor + size).min(bytes.len());
            blobs.insert(oid.to_string(), bytes[cursor..end].to_vec());
            cursor = end + 1;
        }
        for (path, oid) in to_read {
            let body = match blobs.remove(&oid) {
                Some(bytes) => Body::Bytes(bytes),
                None => Body::Missing,
            };
            out.insert(path, body);
        }
        out
    }

    /// Which of `rels`, absent from a tree, snapshots leave out: matched by
    /// the exclude rules (`check-ignore --no-index`, the rules `add -A`
    /// applies) or bigger than the cap on disk now. A path that is big now
    /// may have been small and absent then; calling it excluded only costs a
    /// revert that reports it unavailable instead of deleting it.
    async fn left_out(&self, rels: &[&String], max_bytes: u64) -> HashSet<String> {
        if rels.is_empty() {
            return HashSet::new();
        }
        let input: Vec<u8> = rels
            .iter()
            .map(|rel| rel.as_bytes())
            .collect::<Vec<_>>()
            .join(&0u8);
        let mut out: HashSet<String> = match self
            .run_with_stdin(
                None,
                &["check-ignore", "-z", "--stdin", "--no-index"],
                Some(input),
            )
            .await
        {
            Ok(done) => done
                .stdout
                .split(|b| *b == 0)
                .filter(|rel| !rel.is_empty())
                .map(|rel| String::from_utf8_lossy(rel).into_owned())
                .collect(),
            Err(e) => {
                tracing::debug!(error = %e, "turn snapshot: exclude check failed");
                HashSet::new()
            }
        };
        for rel in rels {
            let big = std::fs::symlink_metadata(self.root.join(rel.as_str()))
                .is_ok_and(|meta| meta.is_file() && meta.len() > max_bytes);
            if big {
                out.insert(rel.to_string());
            }
        }
        out
    }

    /// The paths bigger than `max_bytes` that the tree would hold: every
    /// file in the index (seeded from HEAD or kept from an earlier snapshot)
    /// and every untracked one `add -A` would read. Listing hashes nothing;
    /// the two listings run side by side.
    async fn too_large(&self, index: &Path, max_bytes: u64) -> Vec<Vec<u8>> {
        let (indexed, untracked) = tokio::join!(
            self.git(Some(index), &["ls-files", "-z", "-c"]),
            self.git(Some(index), &["ls-files", "-z", "-o", "--exclude-standard"]),
        );
        let listed = match (indexed, untracked) {
            (Ok(mut indexed), Ok(untracked)) => {
                indexed.push(0);
                indexed.extend(untracked);
                indexed
            }
            (Err(e), _) | (_, Err(e)) => {
                tracing::debug!(error = %e, "turn snapshot: size check skipped");
                return Vec::new();
            }
        };
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            let mut big: Vec<Vec<u8>> = listed
                .split(|b| *b == 0)
                .filter(|rel| !rel.is_empty())
                .filter(|rel| {
                    std::fs::symlink_metadata(root.join(path_from_bytes(rel)))
                        .is_ok_and(|meta| meta.is_file() && meta.len() > max_bytes)
                })
                .map(<[u8]>::to_vec)
                .collect();
            big.sort();
            big.dedup();
            big
        })
        .await
        .unwrap_or_default()
    }

    /// Run git with the session's index, reading `pathspecs` NUL-separated
    /// from stdin when there are any: a list of files can outgrow a command
    /// line.
    async fn run_with_paths(
        &self,
        index: &Path,
        args: &[&str],
        pathspecs: &[Vec<u8>],
    ) -> Result<std::process::Output, String> {
        if pathspecs.is_empty() {
            return self.run_with_stdin(Some(index), args, None).await;
        }
        let mut args = args.to_vec();
        args.extend(["--pathspec-from-file=-", "--pathspec-file-nul"]);
        self.run_with_stdin(Some(index), &args, Some(pathspecs.join(&0u8)))
            .await
    }

    /// Run git, feeding `input` on stdin from its own task so a git that
    /// answers while it reads (`check-ignore --stdin`) cannot fill its
    /// output pipe and block the writer.
    async fn run_with_stdin(
        &self,
        index: Option<&Path>,
        args: &[&str],
        input: Option<Vec<u8>>,
    ) -> Result<std::process::Output, String> {
        let mut cmd = self.command(index);
        cmd.args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("git failed to start: {e}"))?;
        if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
            tokio::spawn(async move {
                let _ = stdin.write_all(&input).await;
            });
        }
        child
            .wait_with_output()
            .await
            .map_err(|e| format!("git failed: {e}"))
    }

    fn command(&self, index: Option<&Path>) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(&self.root)
            .env("GIT_DIR", &self.git_dir)
            .env("GIT_WORK_TREE", &self.root)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0");
        match index {
            Some(index) => cmd.env("GIT_INDEX_FILE", index),
            None => cmd.env_remove("GIT_INDEX_FILE"),
        };
        cmd
    }

    /// Run git against the private repository.
    async fn git(&self, index: Option<&Path>, args: &[&str]) -> Result<Vec<u8>, String> {
        let out = self
            .command(index)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|e| format!("git failed to start: {e}"))?;
        if out.status.success() {
            Ok(out.stdout)
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }

    /// Run git in the root with the root's own repository, if any: what
    /// the alternates and the index seed are read from.
    async fn plain(&self, args: &[&str]) -> Result<String, String> {
        let out = Command::new("git")
            .current_dir(&self.root)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|e| format!("git failed to start: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }
}

/// `path` relative to `root`, for a path recorded under it. A path spelled
/// differently from the canonical root is canonicalized when it still
/// exists.
pub fn rel_under(root: &str, path: &str) -> Option<String> {
    let root = Path::new(root);
    let strip = |p: &Path| {
        p.strip_prefix(root)
            .ok()
            .filter(|rel| !rel.as_os_str().is_empty())
            .map(|rel| rel.to_string_lossy().into_owned())
    };
    strip(Path::new(path)).or_else(|| std::fs::canonicalize(path).ok().and_then(|p| strip(&p)))
}

/// `path`, canonical when it exists.
async fn canonical_or_given(path: &Path) -> PathBuf {
    tokio::fs::canonicalize(path)
        .await
        .unwrap_or_else(|_| path.to_path_buf())
}

/// The private repository's `info/exclude`: [`EXCLUDES`], the turn store
/// when it sits under the root, and the Compose project's runtime state when
/// the project sits under the root. Every worker of a Compose project keeps
/// its state under the project's `data/` (the configuration worker its files
/// under `config/`), the two folders `iii project init` ignores. A root that
/// holds the project, as the stack's launch folder does (the harness's
/// default root for a new session), otherwise re-read the databases, trace
/// store and session files the stack rewrites during every turn, and kept a
/// copy of each: ~40 s and hundreds of MB per turn on a dev stack.
fn exclude_rules(root: &Path, store_dir: &Path, compose_dir: Option<&Path>) -> String {
    let anchored = |dir: &Path| -> Option<String> {
        let rel = dir.strip_prefix(root).ok()?.to_string_lossy().into_owned();
        Some(if rel.is_empty() {
            "/".to_string()
        } else {
            format!("/{rel}/")
        })
    };
    let mut rules: Vec<String> = std::iter::once(RULES_VERSION)
        .chain(EXCLUDES.iter().copied())
        .map(str::to_string)
        .collect();
    if let Some(store) = anchored(store_dir).filter(|rule| rule != "/") {
        rules.push(store);
    }
    if let Some(project) = compose_dir.and_then(anchored) {
        rules.push(format!("{project}data/"));
        rules.push(format!("{project}config/"));
    }
    format!("{}\n", rules.join("\n"))
}

/// Bytes free for this user on the filesystem holding `path`.
#[cfg(unix)]
#[allow(clippy::unnecessary_cast)] // `statvfs` field widths differ between platforms.
fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: `path` is NUL-terminated and `stat` is a valid place for the
    // call to write; it is only read after the call reports success.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    Some((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

#[cfg(not(unix))]
fn free_bytes(_path: &Path) -> Option<u64> {
    None
}

/// A path git printed with `-z`, as the filesystem spells it.
#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_names_stay_within_what_git_accepts() {
        assert_eq!(
            ref_name("s-1", "t:1/x", Side::Before),
            "refs/turns/s-1/t-1-x/before"
        );
    }

    #[test]
    fn rel_under_strips_the_root() {
        assert_eq!(rel_under("/w", "/w/a/b.txt").as_deref(), Some("a/b.txt"));
        assert_eq!(rel_under("/w", "/w"), None);
        assert_eq!(rel_under("/w", "/elsewhere/b.txt"), None);
    }

    /// `.iii/compose/` holds the engine's own runtime state, including sparse
    /// VM disk images: one observed root carried a 16 GiB `upper.ext4` that
    /// the VM rewrote between turns, so every `add -A` re-hashed 16 GiB
    /// (~50s) and stored another ~46MB blob of a disk image no turn will ever
    /// revert to. The rest of `.iii/` is the project's own configuration and
    /// stays in the picture.
    #[tokio::test]
    async fn compose_runtime_state_stays_out_of_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("work");
        let store = dir.path().join("store");
        std::fs::create_dir_all(root.join(".iii/compose/default/vm")).unwrap();
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(root.join(".iii/compose/default/vm/upper.ext4"), "disk").unwrap();
        std::fs::write(root.join(".iii/project.ini"), "name = w\n").unwrap();
        std::fs::write(root.join("notes.md"), "kept\n").unwrap();

        let Some(repo) = SnapshotRepo::open_with(&store.join("repos"), &root, &store, None).await
        else {
            eprintln!("git is not available; skipping");
            return;
        };
        let tree = repo
            .snapshot("s1", DEFAULT_SNAPSHOT_MAX_FILE_BYTES, 0)
            .await
            .unwrap();
        let listed = repo
            .git(None, &["ls-tree", "-r", "--name-only", &tree])
            .await
            .unwrap();
        let listed: Vec<&str> = std::str::from_utf8(&listed).unwrap().lines().collect();

        assert!(
            !listed.iter().any(|p| p.starts_with(".iii/compose/")),
            "{listed:?}"
        );
        assert!(listed.contains(&".iii/project.ini"), "{listed:?}");
        assert!(listed.contains(&"notes.md"), "{listed:?}");
    }

    /// A nested repository with no commit checked out cannot be indexed. The
    /// snapshot still succeeds, keeps every other file, and writes the
    /// session's index so the next snapshot only stats the tree.
    #[tokio::test]
    async fn a_path_git_cannot_index_does_not_fail_the_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("work");
        let store = dir.path().join("store");
        std::fs::create_dir_all(root.join(".tmp/nested")).unwrap();
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(root.join("notes.md"), "kept\n").unwrap();

        let Some(repo) = SnapshotRepo::open_with(&store.join("repos"), &root, &store, None).await
        else {
            eprintln!("git is not available; skipping");
            return;
        };
        let init = Command::new("git")
            .args(["init", "-q"])
            .current_dir(root.join(".tmp/nested"))
            .status()
            .await
            .unwrap();
        assert!(init.success());

        let tree = repo
            .snapshot("s1", DEFAULT_SNAPSHOT_MAX_FILE_BYTES, 0)
            .await
            .unwrap();
        let listed = repo
            .git(None, &["ls-tree", "-r", "--name-only", &tree])
            .await
            .unwrap();
        assert_eq!(std::str::from_utf8(&listed).unwrap().trim(), "notes.md");
        assert!(repo.index_path("s1").is_file());
    }

    async fn listed(repo: &SnapshotRepo, tree: &str) -> Vec<String> {
        let out = repo
            .git(None, &["ls-tree", "-r", "--name-only", tree])
            .await
            .unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn exclude_rules_anchor_the_store_and_the_compose_projects_state() {
        let root = Path::new("/w");
        let rules = exclude_rules(root, Path::new("/w/data/shell/turns"), Some(root));
        let rules: Vec<&str> = rules.lines().collect();
        assert!(rules.contains(&"/data/shell/turns/"), "{rules:?}");
        assert!(rules.contains(&"/data/"), "{rules:?}");
        assert!(rules.contains(&"/config/"), "{rules:?}");

        // A project inside the root is anchored at its own folder.
        let nested = exclude_rules(root, Path::new("/elsewhere"), Some(Path::new("/w/app")));
        assert!(nested.lines().any(|r| r == "/app/data/"), "{nested}");
        assert!(!nested.lines().any(|r| r == "/data/"), "{nested}");

        // A project outside the root, or none, adds nothing; a store that is
        // the root itself never excludes the whole tree.
        let outside = exclude_rules(root, Path::new("/w"), Some(Path::new("/srv/app")));
        assert_eq!(
            outside,
            format!("{RULES_VERSION}\n{}\n", EXCLUDES.join("\n"))
        );
    }

    /// The stack's launch folder is the harness's default root for a new
    /// session. Its `data/` holds every worker's databases and the trace
    /// store, rewritten during each turn: on a dev stack each turn re-hashed
    /// ~13 GB and stored hundreds of MB of copies (~40 s per turn) until the
    /// disk filled. The project's own files stay in the picture.
    #[tokio::test]
    async fn the_compose_projects_runtime_state_stays_out_of_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("stack");
        let store = root.join("data/shell/turns");
        std::fs::create_dir_all(root.join("data/session-manager")).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(root.join("data/iii.db"), "db").unwrap();
        std::fs::write(root.join("data/session-manager/s.json"), "{}").unwrap();
        std::fs::write(root.join("config/harness.yaml"), "x: 1\n").unwrap();
        std::fs::write(root.join("worker-compose.yaml"), "containers: {}\n").unwrap();

        let Some(repo) =
            SnapshotRepo::open_with(&store.join("repos"), &root, &store, Some(&root)).await
        else {
            eprintln!("git is not available; skipping");
            return;
        };
        let tree = repo
            .snapshot("s1", DEFAULT_SNAPSHOT_MAX_FILE_BYTES, 0)
            .await
            .unwrap();
        assert_eq!(listed(&repo, &tree).await, vec!["worker-compose.yaml"]);
    }

    /// A file over the cap is left out of the tree whether it is new or grew
    /// past the cap after an earlier snapshot took it; the rest is kept.
    #[tokio::test]
    async fn files_over_the_size_cap_stay_out_of_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("work");
        let store = dir.path().join("store");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(root.join("notes.md"), "kept\n").unwrap();
        std::fs::write(root.join("big [1].bin"), vec![0u8; 64]).unwrap();
        std::fs::write(root.join("grows.log"), "small\n").unwrap();

        let Some(repo) = SnapshotRepo::open_with(&store.join("repos"), &root, &store, None).await
        else {
            eprintln!("git is not available; skipping");
            return;
        };
        let tree = repo.snapshot("s1", 32, 0).await.unwrap();
        assert_eq!(listed(&repo, &tree).await, vec!["grows.log", "notes.md"]);

        std::fs::write(root.join("grows.log"), vec![b'x'; 64]).unwrap();
        let tree = repo.snapshot("s1", 32, 0).await.unwrap();
        assert_eq!(listed(&repo, &tree).await, vec!["notes.md"]);
    }

    /// A repository created under older rules follows the current ones on
    /// its next open: the exclude file is rewritten, the session indexes
    /// that already hold now-excluded paths are dropped, and the blobs of
    /// dropped turns no longer wait two weeks for gc.
    #[tokio::test]
    async fn a_repository_from_older_rules_follows_the_current_ones() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("stack");
        let store = dir.path().join("store");
        std::fs::create_dir_all(root.join("data")).unwrap();
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(root.join("data/iii.db"), "db").unwrap();
        std::fs::write(root.join("notes.md"), "kept\n").unwrap();

        // An older build: no Compose project rules.
        let Some(old) = SnapshotRepo::open_with(&store.join("repos"), &root, &store, None).await
        else {
            eprintln!("git is not available; skipping");
            return;
        };
        let tree = old
            .snapshot("s1", DEFAULT_SNAPSHOT_MAX_FILE_BYTES, 0)
            .await
            .unwrap();
        assert!(listed(&old, &tree)
            .await
            .contains(&"data/iii.db".to_string()));

        let repo = SnapshotRepo::open_with(&store.join("repos"), &root, &store, Some(&root))
            .await
            .unwrap();
        assert!(!old.index_path("s1").exists());
        let exclude = std::fs::read_to_string(repo.git_dir.join("info/exclude")).unwrap();
        assert!(exclude.lines().any(|r| r == "/data/"), "{exclude}");
        for (key, value) in [
            ("gc.pruneExpire", "1.hour.ago"),
            ("core.untrackedCache", "true"),
        ] {
            let set = repo.git(None, &["config", key]).await.unwrap();
            assert_eq!(String::from_utf8(set).unwrap().trim(), value, "{key}");
        }

        let tree = repo
            .snapshot("s1", DEFAULT_SNAPSHOT_MAX_FILE_BYTES, 0)
            .await
            .unwrap();
        assert_eq!(listed(&repo, &tree).await, vec!["notes.md"]);
    }

    #[test]
    fn free_bytes_reads_the_disk() {
        if cfg!(unix) {
            assert!(free_bytes(Path::new("/")).is_some_and(|free| free > 0));
        }
    }

    /// Below the free-space floor nothing is written: the snapshot is
    /// skipped instead of taking the disk to 0 B.
    #[tokio::test]
    async fn a_nearly_full_disk_skips_the_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("work");
        let store = dir.path().join("store");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(root.join("notes.md"), "kept\n").unwrap();
        let Some(repo) = SnapshotRepo::open_with(&store.join("repos"), &root, &store, None).await
        else {
            eprintln!("git is not available; skipping");
            return;
        };
        let err = repo
            .snapshot("s1", DEFAULT_SNAPSHOT_MAX_FILE_BYTES, u64::MAX)
            .await
            .unwrap_err();
        assert!(err.starts_with("skipped:"), "{err}");
        assert!(!repo.index_path("s1").exists());
    }

    /// A path absent from a tree is "did not exist" only when the snapshot
    /// would have taken it. One the rules or the cap leave out is unknown.
    #[tokio::test]
    async fn bodies_tell_a_left_out_path_from_one_that_did_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("stack");
        let store = dir.path().join("store");
        std::fs::create_dir_all(root.join("data")).unwrap();
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(root.join("data/iii.db"), "db").unwrap();
        std::fs::write(root.join("big.bin"), vec![0u8; 64]).unwrap();
        std::fs::write(root.join("notes.md"), "kept\n").unwrap();
        let Some(repo) =
            SnapshotRepo::open_with(&store.join("repos"), &root, &store, Some(&root)).await
        else {
            eprintln!("git is not available; skipping");
            return;
        };
        let tree = repo.snapshot("s1", 32, 0).await.unwrap();
        let rels: Vec<String> = ["data/iii.db", "big.bin", "gone.txt", "notes.md"]
            .map(str::to_string)
            .to_vec();
        let bodies = repo.bodies(&tree, &rels, u64::MAX, 32).await;
        assert!(
            matches!(bodies["data/iii.db"], Body::Excluded),
            "{bodies:?}"
        );
        assert!(matches!(bodies["big.bin"], Body::Excluded), "{bodies:?}");
        assert!(matches!(bodies["gone.txt"], Body::Missing), "{bodies:?}");
        assert!(
            matches!(&bodies["notes.md"], Body::Bytes(b) if b == b"kept\n"),
            "{bodies:?}"
        );
    }

    /// A file committed at HEAD and over the cap is left out too, even
    /// unchanged: the cap holds for every path in the tree.
    #[tokio::test]
    async fn a_tracked_file_over_the_cap_stays_out_of_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("work");
        let store = dir.path().join("store");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(root.join("big.bin"), vec![0u8; 64]).unwrap();
        std::fs::write(root.join("notes.md"), "kept\n").unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&root)
                .status()
                .is_ok_and(|status| status.success())
        };
        if !git(&["init", "-q"])
            || !git(&["add", "-A"])
            || !git(&[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "-m",
                "init",
            ])
        {
            eprintln!("git is not available; skipping");
            return;
        }
        let repo = SnapshotRepo::open_with(&store.join("repos"), &root, &store, None)
            .await
            .unwrap();
        let tree = repo.snapshot("s1", 32, 0).await.unwrap();
        assert_eq!(listed(&repo, &tree).await, vec!["notes.md"]);
    }
}
