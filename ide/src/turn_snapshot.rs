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

/// Untracked paths a snapshot never takes, on top of the root's own
/// ignore rules: dependency trees and build output. A file tracked despite
/// a rule here is seeded into the index from the root's HEAD and refreshed
/// like any other.
const EXCLUDES: &[&str] = &[
    ".git/",
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

/// One side of a file in a tree.
#[derive(Debug)]
pub enum Body {
    /// The path is not in the tree.
    Missing,
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
}

impl SnapshotRepo {
    /// Open the repository for `root`, creating it on first use. `store_dir`
    /// is the turn store itself, kept out of the picture when it sits under
    /// the root. None when the root is not a directory or git is not here.
    pub async fn open(repos_dir: &Path, root: &Path, store_dir: &Path) -> Option<Self> {
        let root = tokio::fs::canonicalize(root).await.ok()?;
        if !root.is_dir() {
            return None;
        }
        let key = format!("{:x}", Sha256::digest(root.to_string_lossy().as_bytes()));
        let repo = Self {
            git_dir: repos_dir.join(&key[..16]),
            root,
        };
        if !repo.git_dir.join("HEAD").is_file() {
            tokio::fs::create_dir_all(&repo.git_dir).await.ok()?;
            let dir = repo.git_dir.to_string_lossy().into_owned();
            repo.plain(&["init", "-q", "--bare", &dir]).await.ok()?;
            let _ = repo
                .plain(&["--git-dir", &dir, "config", "gc.auto", "256"])
                .await;
            let mut excludes: Vec<String> = EXCLUDES.iter().map(|s| s.to_string()).collect();
            if let Ok(rel) = store_dir.strip_prefix(&repo.root) {
                excludes.push(format!("/{}/", rel.to_string_lossy()));
            }
            let _ = tokio::fs::create_dir_all(repo.git_dir.join("info")).await;
            let _ = tokio::fs::write(
                repo.git_dir.join("info/exclude"),
                format!("{}\n", excludes.join("\n")),
            )
            .await;
        }
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

    /// The tree of the root as it is right now, written through the
    /// session's own index.
    pub async fn snapshot(&self, session_stem: &str) -> Result<String, String> {
        let index = self.git_dir.join(format!("index-{session_stem}"));
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
            }
        }
        self.git(Some(&index), &["add", "-A"]).await?;
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
        for rel in rels {
            if !out.contains_key(rel) && !to_read.iter().any(|(p, _)| p == rel) {
                out.insert(rel.clone(), Body::Missing);
            }
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
}
