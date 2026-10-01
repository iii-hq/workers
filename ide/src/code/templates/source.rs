//! Where worker templates come from: a local checkout (`code.templates.dir`,
//! read on every call) or a shallow clone of `url@ref` cached under
//! `cache_dir` and refreshed every `refresh_secs`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::code::config::TemplatesConfig;
use crate::code::error::CoderError;

/// Where the templates were read from.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct TemplateSourceInfo {
    /// `dir` (a local checkout) or `git` (a cached clone).
    pub kind: String,
    /// The templates directory (`dir`) or the clone URL (`git`).
    pub location: String,
    /// Branch or tag of the clone (`git` only).
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    /// Commit the clone is at (`git` only).
    pub revision: Option<String>,
    /// Set when the clone could not be refreshed and a stale copy is served.
    pub warning: Option<String>,
}

/// A resolved template source.
#[derive(Debug, Clone)]
pub struct ResolvedSource {
    pub info: TemplateSourceInfo,
    /// Directory holding the root `template.yaml`.
    pub root: PathBuf,
}

/// When each `(clone dir, ref)` last synced. Holding the lock also
/// serialises every clone and refresh.
// ponytail: one global lock; per-clone locks only if several sources ever
// share one worker.
static SYNCED: tokio::sync::Mutex<BTreeMap<(PathBuf, String), Instant>> =
    tokio::sync::Mutex::const_new(BTreeMap::new());

/// Bound on one git command, so a hung network cannot hold the lock forever.
const GIT_TIMEOUT: Duration = Duration::from_secs(120);

/// Resolve the templates root: `cfg.dir` when set, else the cached clone
/// (cloned on first use; refreshed when older than `refresh_secs` or when
/// `refresh` is true). Does not read the environment: callers pass
/// `cfg.templates.clone().with_env()`.
pub async fn resolve_source(
    cfg: &TemplatesConfig,
    refresh: bool,
) -> Result<ResolvedSource, CoderError> {
    match cfg.dir.as_deref().filter(|d| !d.is_empty()) {
        Some(dir) => from_dir(dir),
        None => from_git(cfg, refresh).await,
    }
}

fn from_dir(dir: &str) -> Result<ResolvedSource, CoderError> {
    let dir = iii_worker_paths::resolve_path(dir);
    let root = [dir.join("iii"), dir.clone()]
        .into_iter()
        .find(|d| d.join("template.yaml").is_file())
        .ok_or_else(|| {
            CoderError::TemplatesUnavailable(format!(
                "templates unavailable: {} has neither iii/template.yaml nor template.yaml. \
                 Set code.templates.dir (or III_TEMPLATE_DIR) to a templates checkout: its \
                 root or its iii/ directory.",
                dir.display()
            ))
        })?;
    Ok(ResolvedSource {
        info: TemplateSourceInfo {
            kind: "dir".into(),
            location: root.display().to_string(),
            git_ref: None,
            revision: None,
            warning: None,
        },
        root,
    })
}

async fn from_git(cfg: &TemplatesConfig, refresh: bool) -> Result<ResolvedSource, CoderError> {
    let hash = format!("{:x}", Sha256::digest(cfg.url.as_bytes()));
    let clone = iii_worker_paths::resolve_path(&cfg.cache_dir).join(&hash[..8]);
    let key = (clone.clone(), cfg.git_ref.clone());
    let mut synced = SYNCED.lock().await;
    let mut warning = None;
    if git(&clone, &["rev-parse", "--verify", "--quiet", "HEAD"])
        .await
        .is_err()
    {
        // Missing, half-cloned or corrupt: start over.
        let _ = std::fs::remove_dir_all(&clone);
        clone_into(cfg, &clone).await.map_err(|e| {
            CoderError::TemplatesUnavailable(format!(
                "templates unavailable: cloning {} @ {} failed: {e}. Set code.templates.dir \
                 (or III_TEMPLATE_DIR) to a local templates checkout, or check the network.",
                cfg.url, cfg.git_ref
            ))
        })?;
        synced.insert(key, Instant::now());
    } else if refresh
        || synced
            .get(&key)
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(cfg.refresh_secs))
    {
        match fetch(cfg, &clone).await {
            Ok(()) => {
                synced.insert(key, Instant::now());
            }
            Err(e) => {
                warning = Some(format!(
                    "could not refresh {} @ {}: {e}; serving the cached copy",
                    cfg.url, cfg.git_ref
                ))
            }
        }
    }
    let revision = git(&clone, &["rev-parse", "HEAD"]).await.ok();
    Ok(ResolvedSource {
        info: TemplateSourceInfo {
            kind: "git".into(),
            location: cfg.url.clone(),
            git_ref: Some(cfg.git_ref.clone()),
            revision,
            warning,
        },
        root: clone.join("iii"),
    })
}

async fn clone_into(cfg: &TemplatesConfig, dir: &Path) -> Result<(), String> {
    let mut cmd = git_command();
    cmd.args([
        "clone",
        "--quiet",
        "--depth",
        "1",
        "--branch",
        cfg.git_ref.as_str(),
        "--",
        cfg.url.as_str(),
    ])
    .arg(dir);
    run(cmd).await.map(drop)
}

async fn fetch(cfg: &TemplatesConfig, dir: &Path) -> Result<(), String> {
    git(
        dir,
        &[
            "fetch",
            "--quiet",
            "--depth",
            "1",
            "--",
            "origin",
            cfg.git_ref.as_str(),
        ],
    )
    .await?;
    git(dir, &["reset", "--quiet", "--hard", "FETCH_HEAD"])
        .await
        .map(drop)
}

/// git on the clone. `--git-dir` pins the repository, so a clone dir that
/// lost its `.git` fails here instead of resolving to an enclosing repo.
async fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = git_command();
    cmd.arg("--git-dir")
        .arg(dir.join(".git"))
        .arg("--work-tree")
        .arg(dir)
        .args(args);
    run(cmd).await
}

fn git_command() -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("git");
    cmd.env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    cmd
}

async fn run(mut cmd: tokio::process::Command) -> Result<String, String> {
    let out = tokio::time::timeout(GIT_TIMEOUT, cmd.output())
        .await
        .map_err(|_| format!("git timed out after {}s", GIT_TIMEOUT.as_secs()))?
        .map_err(|e| format!("git failed to start: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}
