//! A *line* is a source tree the worker can build: the live working tree,
//! its previous build, a commit reached through a ref or a sha, or a chat
//! turn's before/after tree photographed by the ide worker. This module
//! resolves a line spec to a key and materializes the tree on disk.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::process::Command;

use crate::model::LineKind;
use crate::store::{PREV_KEY, WORKTREE_KEY};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineSpec {
    Worktree,
    Prev,
    Ref(String),
    Sha(String),
    Turn {
        session: String,
        turn: String,
        side: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub key: String,
    pub kind: LineKind,
    pub label: String,
    pub sha: Option<String>,
}

fn is_sha(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|c| c.is_ascii_hexdigit())
}

impl LineSpec {
    /// Accepts `"worktree"`, `"prev"`, a ref name, a full sha, or an object
    /// `{ ref }`, `{ sha }`, `{ turn: { session, turn, side } }`.
    pub fn parse(value: Option<&Value>) -> Result<LineSpec, String> {
        let Some(value) = value else {
            return Ok(LineSpec::Worktree);
        };
        match value {
            Value::Null => Ok(LineSpec::Worktree),
            Value::String(text) => Self::parse_text(text),
            Value::Object(map) => {
                if let Some(text) = map.get("ref").and_then(Value::as_str) {
                    return Self::parse_text(text);
                }
                if let Some(text) = map.get("sha").and_then(Value::as_str) {
                    let sha = text.trim().to_ascii_lowercase();
                    return if is_sha(&sha) {
                        Ok(LineSpec::Sha(sha))
                    } else {
                        Err(format!("not a full sha: {text}"))
                    };
                }
                if let Some(turn) = map.get("turn") {
                    let text = |k: &str| {
                        turn.get(k)
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                    };
                    let side = text("side").unwrap_or_else(|| "after".to_string());
                    if side != "before" && side != "after" {
                        return Err("turn side must be before or after".into());
                    }
                    return Ok(LineSpec::Turn {
                        session: text("session").ok_or("turn.session is required")?,
                        turn: text("turn").ok_or("turn.turn is required")?,
                        side,
                    });
                }
                if let Some(text) = map.get("key").and_then(Value::as_str) {
                    return Self::parse_text(text);
                }
                Err(
                    "a line is a string, { ref }, { sha } or { turn: { session, turn, side } }"
                        .into(),
                )
            }
            _ => Err(
                "a line is a string, { ref }, { sha } or { turn: { session, turn, side } }".into(),
            ),
        }
    }

    fn parse_text(text: &str) -> Result<LineSpec, String> {
        let text = text.trim();
        match text {
            "" | WORKTREE_KEY | "working-tree" | "wt" => Ok(LineSpec::Worktree),
            PREV_KEY | "prev" | "previous" => Ok(LineSpec::Prev),
            _ if is_sha(&text.to_ascii_lowercase()) => Ok(LineSpec::Sha(text.to_ascii_lowercase())),
            _ if text.starts_with("turn:") => {
                let parts: Vec<&str> = text.trim_start_matches("turn:").split('/').collect();
                match parts.as_slice() {
                    [session, turn] => Ok(LineSpec::Turn {
                        session: session.to_string(),
                        turn: turn.to_string(),
                        side: "after".into(),
                    }),
                    [session, turn, side] => Ok(LineSpec::Turn {
                        session: session.to_string(),
                        turn: turn.to_string(),
                        side: side.to_string(),
                    }),
                    _ => Err("turn lines look like turn:<session>/<turn>[/before|after]".into()),
                }
            }
            _ => {
                if text.contains("..")
                    || text.starts_with('-')
                    || text.chars().any(char::is_whitespace)
                {
                    return Err(format!("not a ref name: {text}"));
                }
                Ok(LineSpec::Ref(text.to_string()))
            }
        }
    }
}

pub async fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("spawning git: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub async fn head_sha(root: &Path) -> Option<String> {
    git(root, &["rev-parse", "HEAD"])
        .await
        .ok()
        .filter(|s| is_sha(s))
}

pub async fn is_dirty(root: &Path) -> Option<bool> {
    git(root, &["status", "--porcelain", "--untracked-files=normal"])
        .await
        .ok()
        .map(|out| !out.is_empty())
}

pub async fn current_branch(root: &Path) -> Option<String> {
    git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .await
        .ok()
        .filter(|s| !s.is_empty() && s != "HEAD")
}

/// The ide worker's private snapshot repository for `root`.
fn ide_repo_for(ide_turns_dir: &Path, root: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let key = format!(
        "{:x}",
        Sha256::digest(canonical.to_string_lossy().as_bytes())
    );
    ide_turns_dir.join("repos").join(&key[..16])
}

fn ref_safe(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

pub async fn resolve(
    spec: &LineSpec,
    root: &Path,
    ide_turns_dir: &Path,
) -> Result<Resolved, String> {
    match spec {
        LineSpec::Worktree => Ok(Resolved {
            key: WORKTREE_KEY.into(),
            kind: LineKind::Worktree,
            label: "working tree".into(),
            sha: head_sha(root).await,
        }),
        LineSpec::Prev => Ok(Resolved {
            key: PREV_KEY.into(),
            kind: LineKind::Prev,
            label: "previous build".into(),
            sha: None,
        }),
        LineSpec::Sha(sha) => {
            git(root, &["cat-file", "-e", &format!("{sha}^{{commit}}")])
                .await
                .map_err(|e| format!("unknown commit {sha}: {e}"))?;
            Ok(Resolved {
                key: sha.clone(),
                kind: LineKind::Ref,
                label: sha[..12].to_string(),
                sha: Some(sha.clone()),
            })
        }
        LineSpec::Ref(name) => {
            let sha = git(
                root,
                &["rev-parse", "--verify", &format!("{name}^{{commit}}")],
            )
            .await
            .map_err(|e| format!("unknown ref {name}: {e}"))?;
            if !is_sha(&sha) {
                return Err(format!("{name} did not resolve to a commit"));
            }
            Ok(Resolved {
                key: sha.clone(),
                kind: LineKind::Ref,
                label: name.clone(),
                sha: Some(sha),
            })
        }
        LineSpec::Turn {
            session,
            turn,
            side,
        } => {
            let repo = ide_repo_for(ide_turns_dir, root);
            if !repo.join("HEAD").is_file() {
                return Err(format!(
                    "no ide turn snapshots for this workspace (looked in {})",
                    repo.display()
                ));
            }
            let reference = format!(
                "refs/turns/{}/{}/{}",
                ref_safe(session),
                ref_safe(turn),
                side
            );
            let git_dir = repo.to_string_lossy().into_owned();
            let sha = git(
                root,
                &["--git-dir", &git_dir, "rev-parse", "--verify", &reference],
            )
            .await
            .map_err(|e| format!("unknown turn snapshot {reference}: {e}"))?;
            Ok(Resolved {
                key: sha.clone(),
                kind: LineKind::Turn,
                label: format!("turn {turn} ({side})"),
                sha: Some(sha),
            })
        }
    }
}

/// Put the resolved tree at `dest`: a detached worktree for a commit, an
/// archive extraction for a turn tree. `node_modules` folders are linked
/// from the live checkout so the project's dependencies resolve.
pub async fn materialize(
    resolved: &Resolved,
    root: &Path,
    dest: &Path,
    ide_turns_dir: &Path,
) -> Result<(), String> {
    if dest.exists() {
        let _ = cleanup(resolved, root, dest).await;
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let sha = resolved
        .sha
        .clone()
        .ok_or("line has no object to materialize")?;
    match resolved.kind {
        LineKind::Ref => {
            git(
                root,
                &[
                    "worktree",
                    "add",
                    "--detach",
                    "--force",
                    &dest.to_string_lossy(),
                    &sha,
                ],
            )
            .await?;
        }
        LineKind::Turn => {
            std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
            let git_dir = ide_repo_for(ide_turns_dir, root)
                .to_string_lossy()
                .into_owned();
            let archive = Command::new("git")
                .args(["--git-dir", &git_dir, "archive", "--format=tar", &sha])
                .current_dir(root)
                .stdin(Stdio::null())
                .output()
                .await
                .map_err(|e| format!("spawning git archive: {e}"))?;
            if !archive.status.success() {
                return Err(format!(
                    "git archive failed: {}",
                    String::from_utf8_lossy(&archive.stderr).trim()
                ));
            }
            let mut tar = Command::new("tar")
                .args(["-x", "-C", &dest.to_string_lossy()])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("spawning tar: {e}"))?;
            {
                use tokio::io::AsyncWriteExt;
                let mut stdin = tar.stdin.take().ok_or("tar stdin")?;
                stdin
                    .write_all(&archive.stdout)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            let status = tar.wait_with_output().await.map_err(|e| e.to_string())?;
            if !status.status.success() {
                return Err(format!(
                    "tar failed: {}",
                    String::from_utf8_lossy(&status.stderr).trim()
                ));
            }
        }
        LineKind::Worktree | LineKind::Prev => {
            return Err("the working tree is never materialized".into());
        }
    }
    link_node_modules(root, dest);
    Ok(())
}

pub async fn cleanup(resolved: &Resolved, root: &Path, dest: &Path) -> Result<(), String> {
    if resolved.kind == LineKind::Ref {
        let _ = git(
            root,
            &["worktree", "remove", "--force", &dest.to_string_lossy()],
        )
        .await;
        let _ = git(root, &["worktree", "prune"]).await;
    }
    if dest.exists() {
        std::fs::remove_dir_all(dest).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// For every `package.json` directory under `dest` (skipping node_modules)
/// that lacks a `node_modules`, mirror the live checkout's at the same
/// relative path: a real directory whose entries are symlinks to the live
/// packages. Linking the folder itself would let Vite resolve the generated
/// entries (`node_modules/.stories/*`) to the live tree, outside the build
/// root and next to the wrong sources; `.stories` and `.vite` are left out
/// so each checkout builds into its own cache.
pub fn link_node_modules(src_root: &Path, dest_root: &Path) {
    fn walk(src_root: &Path, dest_root: &Path, dir: &Path, depth: usize) {
        if depth > 6 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        if dir.join("package.json").is_file()
            && !dir.join("node_modules").exists()
            && let Ok(rel) = dir.strip_prefix(dest_root)
        {
            let source = src_root.join(rel).join("node_modules");
            if source.is_dir() {
                mirror_dir(&source, &dir.join("node_modules"));
            }
        }
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir()
                && !path.is_symlink()
                && name != "node_modules"
                && !name.starts_with('.')
                && name != "target"
                && name != "dist"
            {
                walk(src_root, dest_root, &path, depth + 1);
            }
        }
    }
    walk(src_root, dest_root, dest_root, 0);
}

/// `target/` becomes a real directory with one symlink per entry of
/// `source/`, except the per-checkout caches.
pub fn mirror_dir(source: &Path, target: &Path) {
    if std::fs::create_dir_all(target).is_err() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(source) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == ".stories" || name_str == ".vite" {
            continue;
        }
        let link = target.join(&name);
        if link.exists() || link.is_symlink() {
            continue;
        }
        let original = entry.path();
        #[cfg(unix)]
        let _ = std::os::unix::fs::symlink(&original, &link);
        #[cfg(windows)]
        {
            let _ = if original.is_dir() {
                std::os::windows::fs::symlink_dir(&original, &link)
            } else {
                std::os::windows::fs::symlink_file(&original, &link)
            };
        }
    }
}

/// Unified diff between two stored blobs (no git objects needed).
pub async fn diff_blobs(
    a: &Path,
    b: &Path,
    label_a: &str,
    label_b: &str,
) -> Result<String, String> {
    let output = Command::new("git")
        .args([
            "diff",
            "--no-index",
            "--no-color",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--",
            &a.to_string_lossy(),
            &b.to_string_lossy(),
        ])
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("spawning git diff: {e}"))?;
    // Exit 1 means "different", 0 "identical"; anything else is an error.
    match output.status.code() {
        Some(0) | Some(1) => {
            let text = String::from_utf8_lossy(&output.stdout).into_owned();
            Ok(text
                .replace(
                    &format!("a{}", a.to_string_lossy()),
                    &format!("a/{label_a}"),
                )
                .replace(
                    &format!("b{}", b.to_string_lossy()),
                    &format!("b/{label_b}"),
                ))
        }
        _ => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_every_line_shape() {
        assert_eq!(LineSpec::parse(None).unwrap(), LineSpec::Worktree);
        assert_eq!(
            LineSpec::parse(Some(&json!("worktree"))).unwrap(),
            LineSpec::Worktree
        );
        assert_eq!(
            LineSpec::parse(Some(&json!("prev"))).unwrap(),
            LineSpec::Prev
        );
        assert_eq!(
            LineSpec::parse(Some(&json!("main"))).unwrap(),
            LineSpec::Ref("main".into())
        );
        assert_eq!(
            LineSpec::parse(Some(&json!({ "ref": "origin/main" }))).unwrap(),
            LineSpec::Ref("origin/main".into())
        );
        let sha = "a".repeat(40);
        assert_eq!(
            LineSpec::parse(Some(&json!(sha))).unwrap(),
            LineSpec::Sha(sha.clone())
        );
        assert_eq!(
            LineSpec::parse(Some(&json!({ "sha": sha }))).unwrap(),
            LineSpec::Sha(sha)
        );
        assert_eq!(
            LineSpec::parse(Some(&json!({ "turn": { "session": "s1", "turn": "t9" } }))).unwrap(),
            LineSpec::Turn {
                session: "s1".into(),
                turn: "t9".into(),
                side: "after".into()
            }
        );
        assert_eq!(
            LineSpec::parse(Some(&json!("turn:s1/t9/before"))).unwrap(),
            LineSpec::Turn {
                session: "s1".into(),
                turn: "t9".into(),
                side: "before".into()
            }
        );
        assert!(LineSpec::parse(Some(&json!({ "sha": "abc" }))).is_err());
        assert!(LineSpec::parse(Some(&json!("--upload-pack=x"))).is_err());
        assert!(LineSpec::parse(Some(&json!("a..b"))).is_err());
    }

    #[tokio::test]
    async fn resolves_refs_in_a_real_repository() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "t"],
        ] {
            git(root, &args).await.unwrap();
        }
        std::fs::write(root.join("a.txt"), "one").unwrap();
        git(root, &["add", "."]).await.unwrap();
        git(root, &["commit", "-q", "-m", "one"]).await.unwrap();
        let head = head_sha(root).await.unwrap();
        let resolved = resolve(&LineSpec::Ref("main".into()), root, root)
            .await
            .unwrap();
        assert_eq!(resolved.sha.as_deref(), Some(head.as_str()));
        assert_eq!(resolved.kind, LineKind::Ref);
        assert_eq!(is_dirty(root).await, Some(false));
        std::fs::write(root.join("a.txt"), "two").unwrap();
        assert_eq!(is_dirty(root).await, Some(true));
        assert!(
            resolve(&LineSpec::Ref("nope".into()), root, root)
                .await
                .is_err()
        );

        let dest = dir.path().join("wt");
        materialize(&resolved, root, &dest, root).await.unwrap();
        assert_eq!(std::fs::read_to_string(dest.join("a.txt")).unwrap(), "one");
        cleanup(&resolved, root, &dest).await.unwrap();
        assert!(!dest.exists());
    }

    #[test]
    fn node_modules_are_mirrored_as_real_directories_with_linked_entries() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live");
        let checkout = dir.path().join("checkout");
        for root in [&live, &checkout] {
            std::fs::create_dir_all(root.join("app/src")).unwrap();
            std::fs::write(root.join("app/package.json"), "{}").unwrap();
        }
        std::fs::create_dir_all(live.join("app/node_modules/react")).unwrap();
        std::fs::create_dir_all(live.join("app/node_modules/.stories")).unwrap();
        std::fs::create_dir_all(live.join("app/node_modules/.vite")).unwrap();
        std::fs::write(live.join("app/node_modules/react/package.json"), "{}").unwrap();

        link_node_modules(&live, &checkout);
        let mirrored = checkout.join("app/node_modules");
        assert!(
            mirrored.is_dir() && !mirrored.is_symlink(),
            "a real directory, never a symlink"
        );
        assert!(mirrored.join("react").is_symlink());
        assert!(mirrored.join("react/package.json").is_file());
        assert!(
            !mirrored.join(".stories").exists(),
            "per-checkout caches are not shared"
        );
        assert!(!mirrored.join(".vite").exists());
        // Idempotent and never touches an existing node_modules.
        link_node_modules(&live, &checkout);
        assert!(mirrored.join("react").is_symlink());
    }

    #[tokio::test]
    async fn diff_blobs_produces_a_unified_diff() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::write(&a, "x\ny\n").unwrap();
        std::fs::write(&b, "x\nz\n").unwrap();
        let text = diff_blobs(&a, &b, "src/x.ts", "src/x.ts").await.unwrap();
        assert!(text.contains("-y"), "{text}");
        assert!(text.contains("+z"), "{text}");
        assert!(text.contains("b/src/x.ts"), "{text}");
    }
}
