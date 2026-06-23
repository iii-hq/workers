use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::color::ColorMode;
use crate::discover::{discover_repo_workers, harness_stack_names, order_worker_names, WorkerSpec};

pub const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";
pub const DEFAULT_POLL_INTERVAL_MS: u64 = 2000;
pub const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 120_000;
pub const LOG_RING_CAPACITY: usize = 500;
pub const DEFAULT_LOG_TAIL: usize = 100;
pub const DASHBOARD_LOG_LINES: usize = 20;

#[derive(Debug, Clone)]
pub struct Config {
    pub repo_root: PathBuf,
    pub engine_url: String,
    pub engine_host: String,
    pub engine_port: u16,
    pub release: bool,
    pub poll_interval_ms: u64,
    pub connect_timeout_ms: u64,
    pub workers: Vec<String>,
    pub harness_stack: Vec<String>,
    pub worker_specs: Vec<WorkerSpec>,
    pub stop_on_exit: bool,
    pub color_mode: ColorMode,
}

#[derive(Debug, Deserialize, Default)]
struct FileConfig {
    repo: Option<PathBuf>,
    engine_url: Option<String>,
    release: Option<bool>,
    poll_interval_ms: Option<u64>,
    connect_timeout_ms: Option<u64>,
    workers: Option<Vec<String>>,
    harness_stack: Option<Vec<String>>,
    stop_on_exit: Option<bool>,
    color: Option<String>,
}

impl Config {
    pub fn load(
        repo: Option<PathBuf>,
        engine_url: Option<String>,
        port: Option<u16>,
        release: bool,
        config_path: Option<PathBuf>,
        stop_on_exit: bool,
        color: Option<String>,
    ) -> Result<Self> {
        let file_cfg = if let Some(path) = config_path {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("read config file {}", path.display()))?;
            serde_yaml::from_str(&raw)
                .with_context(|| format!("parse config file {}", path.display()))?
        } else {
            FileConfig::default()
        };

        let repo_root = resolve_repo_root(repo.or(file_cfg.repo))?;
        let worker_specs = discover_repo_workers(&repo_root)?;
        if worker_specs.is_empty() {
            bail!("no workers discovered under {}", repo_root.display());
        }

        let engine_url = engine_url
            .or(file_cfg.engine_url)
            .unwrap_or_else(|| DEFAULT_ENGINE_URL.to_string());
        let (engine_host, engine_port) = parse_engine_url(&engine_url, port)?;

        let discovered_names = order_worker_names(&worker_specs);
        let workers = file_cfg.workers.unwrap_or(discovered_names);
        let harness_stack = file_cfg
            .harness_stack
            .unwrap_or_else(|| harness_stack_names(&worker_specs));

        let color_mode = color
            .or(file_cfg.color)
            .as_deref()
            .and_then(|s| {
                ColorMode::parse(s).or_else(|| {
                    eprintln!("warning: invalid color mode {s:?}; using auto");
                    None
                })
            })
            .unwrap_or_default();

        Ok(Self {
            repo_root,
            engine_url,
            engine_host,
            engine_port,
            release: release || file_cfg.release.unwrap_or(false),
            poll_interval_ms: file_cfg
                .poll_interval_ms
                .unwrap_or(DEFAULT_POLL_INTERVAL_MS),
            connect_timeout_ms: file_cfg
                .connect_timeout_ms
                .unwrap_or(DEFAULT_CONNECT_TIMEOUT_MS),
            workers,
            harness_stack,
            worker_specs,
            stop_on_exit: stop_on_exit || file_cfg.stop_on_exit.unwrap_or(false),
            color_mode,
        })
    }

    pub fn worker_spec(&self, name: &str) -> Option<&WorkerSpec> {
        self.worker_specs.iter().find(|s| s.name == name)
    }
}

pub fn resolve_repo_root(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("repo path {}", path.display()))?;
        validate_repo_root(&canonical)?;
        return Ok(canonical);
    }

    if let Ok(env) = std::env::var("WORKERS_DEV_REPO") {
        let path = PathBuf::from(env);
        let canonical = path
            .canonicalize()
            .with_context(|| format!("WORKERS_DEV_REPO={}", path.display()))?;
        validate_repo_root(&canonical)?;
        return Ok(canonical);
    }

    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd);
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."));

    for start in candidates {
        if let Some(root) = find_repo_root(&start) {
            return Ok(root);
        }
    }

    bail!(
        "could not find workers repo root (expected top-level */iii.worker.yaml); \
         pass --repo or set WORKERS_DEV_REPO"
    )
}

fn find_repo_root(start: &Path) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        if validate_repo_root(ancestor).is_ok() {
            return ancestor.canonicalize().ok();
        }
    }
    None
}

fn validate_repo_root(path: &Path) -> Result<()> {
    let count = std::fs::read_dir(path)
        .with_context(|| format!("read {}", path.display()))?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().join("iii.worker.yaml").is_file())
        .count();
    if count > 0 {
        Ok(())
    } else {
        bail!("no top-level iii.worker.yaml workers under {}", path.display())
    }
}

pub fn parse_engine_url(url: &str, port_override: Option<u16>) -> Result<(String, u16)> {
    let stripped = url
        .strip_prefix("ws://")
        .or_else(|| url.strip_prefix("wss://"))
        .unwrap_or(url);

    let authority = stripped.split('/').next().unwrap_or(stripped);

    let (host, port) = if let Some((host, port_str)) = authority.rsplit_once(':') {
        let port: u16 = port_str
            .parse()
            .with_context(|| format!("invalid port in engine url {url}"))?;
        (host.to_string(), port)
    } else {
        (authority.to_string(), 49134)
    };

    Ok((host, port_override.unwrap_or(port)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_default_url() {
        let (host, port) = parse_engine_url(DEFAULT_ENGINE_URL, None).unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 49134);
    }

    #[test]
    fn parse_url_with_trailing_slash() {
        let (host, port) = parse_engine_url("ws://127.0.0.1:49134/", None).unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 49134);
    }

    #[test]
    fn parse_url_with_path() {
        let (host, port) = parse_engine_url("ws://127.0.0.1:49134/ws", None).unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 49134);
    }
}
