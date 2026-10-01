//! Look at a directory before the page points a container at it: the worker
//! manifest it holds, whether its relative run command is built, the other
//! git checkouts of the same worker, and, for a plain folder, the workers
//! inside it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::stream::{self, StreamExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use tokio::process::Command;

const MAX_CHILDREN: usize = 200;
/// Checkouts described at once; each costs two short git calls.
const DESCRIBE_AT_ONCE: usize = 8;

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct InspectInput {
    /// Absolute directory on the daemon host; `~/` expands to the home directory.
    pub path: String,
    /// The container's run command; a relative executable is checked for existence.
    pub run: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Manifest {
    pub path: String,
    pub name: String,
    pub language: Option<String>,
    pub description: Option<String>,
    pub dependencies: Vec<String>,
    /// `scripts.start`: how compose starts the worker when its entry has no
    /// `run`. Without either, the container cannot start.
    pub start: Option<String>,
    /// `bin`: the binary a Rust worker builds.
    pub bin: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Checkout {
    /// This worker's directory inside the checkout.
    pub path: String,
    pub branch: Option<String>,
    /// Unix seconds of the checkout's HEAD commit.
    pub committed_at: Option<i64>,
    /// Tracked files differ from HEAD.
    pub dirty: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Inspection {
    pub path: String,
    pub exists: bool,
    pub manifest: Option<Manifest>,
    /// `Some(false)` when the run command is a relative executable missing here.
    pub run_found: Option<bool>,
    pub checkouts: Vec<Checkout>,
    /// Workers directly inside a directory that is not itself a worker.
    pub workers: Vec<Manifest>,
}

fn expand(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

pub fn parse_manifest(dir: &Path, source: &str) -> Option<Manifest> {
    let root: Value = serde_yaml::from_str(source).ok()?;
    let text = |key: &str| root.get(key).and_then(Value::as_str).map(str::to_string);
    Some(Manifest {
        path: dir.to_string_lossy().into_owned(),
        name: text("name")?,
        language: text("language"),
        description: text("description"),
        dependencies: root
            .get("dependencies")
            .and_then(Value::as_mapping)
            .into_iter()
            .flat_map(|deps| deps.keys())
            .filter_map(|key| key.as_str().map(str::to_string))
            .collect(),
        start: root
            .get("scripts")
            .and_then(|scripts| scripts.get("start"))
            .and_then(Value::as_str)
            .map(str::to_string),
        bin: text("bin"),
    })
}

async fn read_manifest(dir: &Path) -> Option<Manifest> {
    let source = tokio::fs::read_to_string(dir.join("iii.worker.yaml"))
        .await
        .ok()?;
    parse_manifest(dir, &source)
}

/// `git worktree list --porcelain` → (checkout root, branch).
pub fn parse_worktrees(output: &str) -> Vec<(PathBuf, Option<String>)> {
    output
        .split("\n\n")
        .filter_map(|block| {
            let mut root = None;
            let mut branch = None;
            for line in block.lines() {
                if let Some(path) = line.strip_prefix("worktree ") {
                    root = Some(PathBuf::from(path));
                } else if let Some(name) = line.strip_prefix("branch ") {
                    branch = Some(name.trim_start_matches("refs/heads/").to_string());
                }
            }
            root.map(|root| (root, branch))
        })
        .collect()
}

async fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new("git")
            .arg("-C")
            .arg(dir)
            // Reading worktrees must never run a repository-configured command,
            // nor take the index lock another git process may be holding.
            .args(["--no-optional-locks", "-c", "core.fsmonitor=false"])
            .args(args)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn checkouts(dir: &Path) -> Vec<Checkout> {
    let Some(top) = git(dir, &["rev-parse", "--show-toplevel"]).await else {
        return Vec::new();
    };
    let Ok(subdir) = dir.strip_prefix(top.trim()) else {
        return Vec::new();
    };
    let Some(list) = git(dir, &["worktree", "list", "--porcelain"]).await else {
        return Vec::new();
    };
    let found = parse_worktrees(&list)
        .into_iter()
        // `join("")` would add a trailing slash when the worker is the repository root.
        .map(|(root, branch)| {
            let path = if subdir.as_os_str().is_empty() {
                root
            } else {
                root.join(subdir)
            };
            (path, branch)
        })
        .filter(|(path, _)| path.join("iii.worker.yaml").is_file())
        .collect::<Vec<_>>();
    stream::iter(found)
        .map(|(path, branch)| describe(path, branch))
        .buffered(DESCRIBE_AT_ONCE)
        .collect()
        .await
}

/// When the checkout last moved and whether it holds uncommitted work.
async fn describe(path: PathBuf, branch: Option<String>) -> Checkout {
    let (time, dirty) = tokio::join!(
        // `log.showSignature` would run the repository's `gpg.program`.
        git(&path, &["log", "-1", "--no-show-signature", "--format=%ct"]),
        dirty(&path),
    );
    Checkout {
        path: path.to_string_lossy().into_owned(),
        branch,
        committed_at: time.and_then(|t| t.trim().parse().ok()),
        dirty,
    }
}

/// `git status` runs a filter driver's `clean`/`process` program whenever it
/// must compare a file's content, so every driver the repository configures
/// is switched off for the call. Reading the config runs nothing.
async fn dirty(path: &Path) -> Option<bool> {
    let keys = git(
        path,
        &[
            "config",
            "--name-only",
            "--get-regexp",
            r"^filter\..+\.(clean|smudge|process)$",
        ],
    )
    .await
    .unwrap_or_default();
    let mut args = Vec::new();
    for key in keys.lines() {
        let name = key.strip_prefix("filter.")?.rsplit_once('.')?.0;
        // `-c` splits at the first `=`; a name holding one cannot be overridden.
        if name.contains('=') {
            return None;
        }
        for field in ["clean", "smudge", "process"] {
            args.extend(["-c".to_string(), format!("filter.{name}.{field}=")]);
        }
        args.extend(["-c".to_string(), format!("filter.{name}.required=false")]);
    }
    args.extend(
        [
            "status",
            "--porcelain",
            "--untracked-files=no",
            // A submodule's own config would get the same chance.
            "--ignore-submodules=all",
        ]
        .map(String::from),
    );
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let status = git(path, &args).await?;
    Some(!status.trim().is_empty())
}

async fn child_workers(dir: &Path) -> Vec<Manifest> {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return Vec::new();
    };
    let mut workers = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        if workers.len() >= MAX_CHILDREN {
            break;
        }
        if let Some(manifest) = read_manifest(&entry.path()).await {
            workers.push(manifest);
        }
    }
    workers.sort_by(|a, b| a.name.cmp(&b.name));
    workers
}

fn run_found(dir: &Path, run: Option<&str>) -> Option<bool> {
    let program = run?.split_whitespace().next()?;
    (program.starts_with("./") || program.starts_with("../")).then(|| dir.join(program).is_file())
}

pub async fn inspect(input: &InspectInput) -> Inspection {
    let dir = expand(input.path.trim());
    let exists = dir.is_dir();
    let manifest = if exists {
        read_manifest(&dir).await
    } else {
        None
    };
    let (checkouts, workers) = match (&manifest, exists) {
        (Some(_), _) => (checkouts(&dir).await, Vec::new()),
        (None, true) => (Vec::new(), child_workers(&dir).await),
        (None, false) => (Vec::new(), Vec::new()),
    };
    Inspection {
        path: dir.to_string_lossy().into_owned(),
        exists,
        run_found: manifest.as_ref().and(run_found(&dir, input.run.as_deref())),
        manifest,
        checkouts,
        workers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_worktree_porcelain() {
        let output = "worktree /repo\nHEAD abc\nbranch refs/heads/main\n\nworktree /wt/feature\nHEAD def\nbranch refs/heads/feat/x\n\nworktree /wt/detached\nHEAD 123\ndetached\n";
        assert_eq!(
            parse_worktrees(output),
            [
                (PathBuf::from("/repo"), Some("main".to_string())),
                (PathBuf::from("/wt/feature"), Some("feat/x".to_string())),
                (PathBuf::from("/wt/detached"), None),
            ]
        );
    }

    #[test]
    fn reads_the_manifest_fields_the_page_needs() {
        let manifest = parse_manifest(
            Path::new("/w/memory"),
            "name: memory\nlanguage: rust\ndescription: Agent memory\ndependencies:\n  state: latest\n  queue: \"^0.21\"\n",
        )
        .unwrap();
        assert_eq!(manifest.name, "memory");
        assert_eq!(manifest.dependencies, ["state", "queue"]);
        assert_eq!(manifest.start, None);

        let started = parse_manifest(
            Path::new("/w/judge"),
            "name: judge\nlanguage: rust\nbin: judge-bin\nscripts:\n  start: ./judge\n",
        )
        .unwrap();
        assert_eq!(started.start.as_deref(), Some("./judge"));
        assert_eq!(started.bin.as_deref(), Some("judge-bin"));
        assert!(parse_manifest(Path::new("/x"), "language: rust\n").is_none());
    }

    #[tokio::test]
    async fn describes_each_checkout_with_its_last_commit_and_changes() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        let worker = repo.join("web");
        std::fs::create_dir_all(&worker).unwrap();
        std::fs::write(worker.join("iii.worker.yaml"), "name: web\n").unwrap();
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
        run(&[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/x",
            root.path().join("wt").to_str().unwrap(),
        ]);
        std::fs::write(
            worker.join("iii.worker.yaml"),
            "name: web\nlanguage: rust\n",
        )
        .unwrap();

        let found = inspect(&InspectInput {
            path: worker.to_string_lossy().into_owned(),
            run: None,
        })
        .await;
        assert_eq!(found.checkouts.len(), 2);
        let main = &found.checkouts[0];
        assert_eq!(main.branch.as_deref(), Some("main"));
        assert_eq!(main.dirty, Some(true));
        assert!(main.committed_at.is_some_and(|t| t > 1_600_000_000));
        let feature = &found.checkouts[1];
        assert_eq!(feature.branch.as_deref(), Some("feat/x"));
        assert_eq!(feature.dirty, Some(false));
        assert!(feature.path.ends_with("wt/web"));
    }

    #[tokio::test]
    async fn describing_a_checkout_runs_no_configured_filter() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let marker = root.path().join("ran");
        let filter = root.path().join("filter.sh");
        std::fs::write(
            &filter,
            format!("#!/bin/sh\ntouch '{}'\ncat\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&filter, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        };
        run(&["init", "-q", "-b", "main"]);
        std::fs::write(repo.join(".gitattributes"), "*.txt filter=evil\n").unwrap();
        std::fs::write(repo.join("a.txt"), "hi\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
        let command = format!("{} clean", filter.display());
        run(&["config", "filter.evil.clean", &command]);
        run(&["config", "filter.evil.process", &command]);
        run(&["config", "filter.evil.required", "true"]);

        // Rewritten within the index's second: racily clean, so git must
        // compare the content and would run the filter to do it.
        std::fs::write(repo.join("a.txt"), "hi\n").unwrap();
        assert_eq!(dirty(&repo).await, Some(false));
        std::fs::write(repo.join("a.txt"), "changed\n").unwrap();
        assert_eq!(dirty(&repo).await, Some(true));
        assert!(!marker.exists(), "a repository-configured filter ran");
    }

    #[tokio::test]
    async fn inspects_a_worker_and_a_folder_of_workers() {
        let root = tempfile::tempdir().unwrap();
        let worker = root.path().join("web");
        std::fs::create_dir_all(worker.join("target/debug")).unwrap();
        std::fs::write(
            worker.join("iii.worker.yaml"),
            "name: web\nlanguage: rust\n",
        )
        .unwrap();
        std::fs::write(worker.join("target/debug/web"), "").unwrap();

        let found = inspect(&InspectInput {
            path: worker.to_string_lossy().into_owned(),
            run: Some("./target/debug/web --flag".into()),
        })
        .await;
        assert_eq!(found.manifest.unwrap().name, "web");
        assert_eq!(found.run_found, Some(true));

        let missing_binary = inspect(&InspectInput {
            path: worker.to_string_lossy().into_owned(),
            run: Some("./target/release/web".into()),
        })
        .await;
        assert_eq!(missing_binary.run_found, Some(false));

        let folder = inspect(&InspectInput {
            path: root.path().to_string_lossy().into_owned(),
            run: None,
        })
        .await;
        assert!(folder.manifest.is_none());
        assert_eq!(folder.workers.len(), 1);

        let nowhere = inspect(&InspectInput {
            path: root.path().join("nope").to_string_lossy().into_owned(),
            run: None,
        })
        .await;
        assert!(!nowhere.exists);
    }
}
