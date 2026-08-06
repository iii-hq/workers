use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::color::ColorMode;
use crate::discover::{
    assign_groups, discover_repo_workers, harness_stack_names, order_worker_names, WorkerSpec,
};

pub const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";
pub const DEFAULT_POLL_INTERVAL_MS: u64 = 2000;
pub const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 120_000;
/// Per-query timeout for `engine::workers::list`. Kept short so a slow or
/// hung engine can't stall the status poll (and, with off-thread polling,
/// can't freeze the TUI).
pub const ENGINE_QUERY_TIMEOUT_MS: u64 = 3_000;
pub const LOG_RING_CAPACITY: usize = 500;
pub const DEFAULT_LOG_TAIL: usize = 100;

#[derive(Debug, Clone)]
pub struct Config {
    pub repo_root: PathBuf,
    pub engine_url: String,
    pub release: bool,
    pub poll_interval_ms: u64,
    pub connect_timeout_ms: u64,
    pub workers: Vec<String>,
    /// Ordered named stacks: `(name, roots)`. Index 0 is always the built-in
    /// `harness` stack (roots overridable via `stacks.harness:`), then file
    /// stacks in YAML order. Roots are filtered to managed workers.
    // Not read outside tests until Task 3 (stack-grouped view) wires it up.
    #[allow(dead_code)]
    pub stacks: Vec<(String, Vec<String>)>,
    /// Name of the stack `up` / bare `start` / Ctrl+u start; always present
    /// in `stacks` with non-empty roots.
    // Not read outside tests until Task 4 (`start_stack`) wires it up.
    #[allow(dead_code)]
    pub default_stack: String,
    pub harness_stack: Vec<String>,
    pub worker_specs: Vec<WorkerSpec>,
    pub stop_on_exit: bool,
    pub color_mode: ColorMode,
    /// Start injectable-UI workers in the SOP's watcher mode by default:
    /// spawn `pnpm watch` in `<worker>/ui` and set `III_<WORKER>_UI_WATCH=1`
    /// so console tabs hot-reload the worker's UI on rebuild. Per-worker
    /// override at runtime via the TUI's `w` key.
    pub ui_watch: bool,
}

#[derive(Debug, Deserialize, Default)]
struct FileConfig {
    repo: Option<PathBuf>,
    engine_url: Option<String>,
    release: Option<bool>,
    poll_interval_ms: Option<u64>,
    connect_timeout_ms: Option<u64>,
    workers: Option<Vec<String>>,
    /// Removed key. Kept in the struct only so its presence can be rejected
    /// with a rename hint — serde would otherwise silently ignore it.
    harness_stack: Option<serde_yaml::Value>,
    stacks: Option<serde_yaml::Mapping>,
    default_stack: Option<String>,
    stop_on_exit: Option<bool>,
    color: Option<String>,
    ui_watch: Option<bool>,
}

impl Config {
    #[allow(clippy::too_many_arguments)]
    pub fn load(
        repo: Option<PathBuf>,
        engine_url: Option<String>,
        port: Option<u16>,
        release: bool,
        config_path: Option<PathBuf>,
        stop_on_exit: bool,
        color: Option<String>,
        ui_watch: bool,
    ) -> Result<Self> {
        let file_cfg = if let Some(path) = config_path {
            load_file_config(&path)?
        } else {
            // No --config: auto-load <repo_root>/workers-dev.yaml when present.
            // The probe resolves the root without a config file (--repo / env /
            // cwd ancestors); if that fails, fall through to defaults and let
            // the final resolve_repo_root below report the real error. An
            // auto-loaded file's `repo:` key is honored as-is — no re-search
            // for another config in the new root.
            match resolve_repo_root(repo.clone()) {
                Ok(root) => {
                    let path = root.join("workers-dev.yaml");
                    if path.is_file() {
                        load_file_config(&path)?
                    } else {
                        FileConfig::default()
                    }
                }
                Err(_) => FileConfig::default(),
            }
        };

        let repo_root = resolve_repo_root(repo.or(file_cfg.repo))?;
        let mut worker_specs = discover_repo_workers(&repo_root)?;
        if worker_specs.is_empty() {
            bail!("no workers discovered under {}", repo_root.display());
        }

        // Resolve stacks before deriving worker order: the default stack's
        // roots drive grouping, and the dashboard's stack group (roots +
        // transitive deps) must be regrouped before the display order is
        // captured below. Root names are validated against the managed
        // `workers` list further down.
        if file_cfg.harness_stack.is_some() {
            bail!(
                "config key `harness_stack:` was replaced by `stacks:` + `default_stack:` — \
                 rename to stacks: {{harness: [...]}}"
            );
        }
        let mut stacks: Vec<(String, Vec<String>)> =
            vec![("harness".to_string(), harness_stack_names(&worker_specs))];
        if let Some(mapping) = file_cfg.stacks {
            for (name, roots) in parse_stacks(mapping)? {
                match stacks.iter_mut().find(|(n, _)| *n == name) {
                    // `stacks.harness:` overrides the builtin roots in place,
                    // keeping harness at index 0.
                    Some(entry) => entry.1 = roots,
                    None => stacks.push((name, roots)),
                }
            }
        }
        let default_stack = file_cfg
            .default_stack
            .unwrap_or_else(|| "harness".to_string());
        let Some(default_roots) = stacks
            .iter()
            .find(|(n, _)| *n == default_stack)
            .map(|(_, r)| r.clone())
        else {
            bail!(
                "default_stack {default_stack:?} is not a defined stack (have: {})",
                stacks
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        };
        assign_groups(&mut worker_specs, &default_roots);

        let raw_engine_url = engine_url
            .or(file_cfg.engine_url)
            .unwrap_or_else(|| DEFAULT_ENGINE_URL.to_string());
        let (engine_host, engine_port) = parse_engine_url(&raw_engine_url, port)?;
        // Canonicalize so a `--port` override is reflected in the URL the SDK
        // actually connects to. The engine WS endpoint is always `/`, so any
        // path in the original URL is intentionally dropped. Preserve a `wss://`
        // scheme so a TLS engine endpoint isn't silently downgraded to plaintext.
        let scheme = if raw_engine_url.starts_with("wss://") {
            "wss"
        } else {
            "ws"
        };
        let engine_url = format!("{scheme}://{engine_host}:{engine_port}");

        let discovered_names = order_worker_names(&worker_specs);
        // An explicit `workers:` list may name folders that auto-discovery
        // skipped (folder/name mismatch or missing iii.worker.yaml). Drop those
        // with a warning instead of letting WorkerGraph::load abort startup —
        // mirroring discover_repo_workers' skip-and-continue behavior.
        let workers = match file_cfg.workers {
            Some(requested) => {
                let valid: std::collections::HashSet<&str> =
                    discovered_names.iter().map(String::as_str).collect();
                requested
                    .into_iter()
                    .filter(|w| {
                        let ok = valid.contains(w.as_str());
                        if !ok {
                            eprintln!(
                                "warning: skipping configured worker {w}: not a discovered worker (folder/name mismatch or missing iii.worker.yaml)"
                            );
                        }
                        ok
                    })
                    .collect()
            }
            None => discovered_names,
        };

        // Keep only stack roots that survived into the managed `workers` set:
        // the WorkerGraph is built over `workers`, so a root outside it makes
        // stack starts bail with "unknown worker". Drop with a warning,
        // mirroring the `workers:` handling.
        let managed: std::collections::HashSet<&str> = workers.iter().map(String::as_str).collect();
        for (stack_name, roots) in &mut stacks {
            roots.retain(|w| {
                let ok = managed.contains(w.as_str());
                if !ok {
                    eprintln!(
                        "warning: skipping stack {stack_name} worker {w}: not in the managed workers list"
                    );
                }
                ok
            });
        }
        let harness_stack = stacks
            .iter()
            .find(|(n, _)| *n == default_stack)
            .map(|(_, r)| r.clone())
            .expect("default stack validated above");
        if harness_stack.is_empty() {
            bail!("default stack {default_stack:?} has no startable workers after validation");
        }

        let color_mode = color
            .or(file_cfg.color)
            .as_deref()
            .and_then(|s| {
                ColorMode::parse(s).or_else(|| {
                    eprintln!(
                        "warning: invalid color mode {s:?} (valid: auto, always, never); using auto"
                    );
                    None
                })
            })
            .unwrap_or_default();

        Ok(Self {
            repo_root,
            engine_url,
            release: release || file_cfg.release.unwrap_or(false),
            poll_interval_ms: file_cfg
                .poll_interval_ms
                .unwrap_or(DEFAULT_POLL_INTERVAL_MS),
            connect_timeout_ms: file_cfg
                .connect_timeout_ms
                .unwrap_or(DEFAULT_CONNECT_TIMEOUT_MS),
            workers,
            stacks,
            default_stack,
            harness_stack,
            worker_specs,
            stop_on_exit: stop_on_exit || file_cfg.stop_on_exit.unwrap_or(false),
            color_mode,
            ui_watch: ui_watch || file_cfg.ui_watch.unwrap_or(false),
        })
    }

    pub fn worker_spec(&self, name: &str) -> Option<&WorkerSpec> {
        self.worker_specs.iter().find(|s| s.name == name)
    }
}

fn load_file_config(path: &Path) -> Result<FileConfig> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read config file {}", path.display()))?;
    serde_yaml::from_str(&raw).with_context(|| format!("parse config file {}", path.display()))
}

/// Parse the `stacks:` mapping preserving YAML order (serde_yaml::Mapping is
/// insertion-ordered). Every value must be a list of worker-name strings.
fn parse_stacks(mapping: serde_yaml::Mapping) -> Result<Vec<(String, Vec<String>)>> {
    let mut stacks = Vec::new();
    for (key, value) in mapping {
        let name = key
            .as_str()
            .with_context(|| format!("stacks: key {key:?} is not a string"))?
            .to_string();
        let roots: Vec<String> = serde_yaml::from_value(value)
            .with_context(|| format!("stacks.{name}: expected a list of worker names"))?;
        stacks.push((name, roots));
    }
    Ok(stacks)
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
        bail!(
            "no top-level iii.worker.yaml workers under {}",
            path.display()
        )
    }
}

pub fn parse_engine_url(url: &str, port_override: Option<u16>) -> Result<(String, u16)> {
    let stripped = url
        .strip_prefix("ws://")
        .or_else(|| url.strip_prefix("wss://"))
        .unwrap_or(url);

    let authority = stripped.split('/').next().unwrap_or(stripped);

    let (host, port) = if let Some((host, port_str)) = authority.rsplit_once(':') {
        let port: u16 = port_str.parse().with_context(|| {
            format!("invalid port in engine url {url} (port must be a number, e.g. 49134)")
        })?;
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

    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Minimal discoverable repo: one rust/binary worker.
    fn write_repo(tmp: &TempDir) {
        let dir = tmp.path().join("harness");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("iii.worker.yaml"),
            "iii: v1\nname: harness\nlanguage: rust\ndeploy: binary\ndescription: test\n",
        )
        .unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[workspace]\n").unwrap();
    }

    fn load(tmp: &TempDir, config_path: Option<PathBuf>) -> Result<Config> {
        Config::load(
            Some(tmp.path().to_path_buf()),
            None,
            None,
            false,
            config_path,
            false,
            None,
            false,
        )
    }

    #[test]
    fn auto_loads_workers_dev_yaml_from_repo_root() {
        let tmp = TempDir::new().unwrap();
        write_repo(&tmp);
        std::fs::write(
            tmp.path().join("workers-dev.yaml"),
            "engine_url: ws://127.0.0.1:55555\n",
        )
        .unwrap();
        let cfg = load(&tmp, None).unwrap();
        assert_eq!(cfg.engine_url, "ws://127.0.0.1:55555");
    }

    #[test]
    fn absent_config_file_means_defaults() {
        let tmp = TempDir::new().unwrap();
        write_repo(&tmp);
        let cfg = load(&tmp, None).unwrap();
        assert_eq!(cfg.engine_url, DEFAULT_ENGINE_URL);
    }

    #[test]
    fn explicit_config_beats_auto_load() {
        let tmp = TempDir::new().unwrap();
        write_repo(&tmp);
        std::fs::write(
            tmp.path().join("workers-dev.yaml"),
            "engine_url: ws://127.0.0.1:55555\n",
        )
        .unwrap();
        let other = tmp.path().join("elsewhere.yaml");
        std::fs::write(&other, "engine_url: ws://127.0.0.1:44444\n").unwrap();
        let cfg = load(&tmp, Some(other)).unwrap();
        assert_eq!(cfg.engine_url, "ws://127.0.0.1:44444");
    }

    /// Repo with enough workers to define non-trivial stacks.
    fn write_repo_multi(tmp: &TempDir) {
        for name in ["harness", "session-manager", "console"] {
            let dir = tmp.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("iii.worker.yaml"),
                format!(
                    "iii: v1\nname: {name}\nlanguage: rust\ndeploy: binary\ndescription: test\n"
                ),
            )
            .unwrap();
            std::fs::write(dir.join("Cargo.toml"), "[workspace]\n").unwrap();
        }
    }

    fn load_with_yaml(tmp: &TempDir, yaml: &str) -> Result<Config> {
        std::fs::write(tmp.path().join("workers-dev.yaml"), yaml).unwrap();
        load(tmp, None)
    }

    #[test]
    fn stacks_parse_with_builtin_harness_first() {
        let tmp = TempDir::new().unwrap();
        write_repo_multi(&tmp);
        let cfg = load_with_yaml(
            &tmp,
            "stacks:\n  console: [console, session-manager]\ndefault_stack: console\n",
        )
        .unwrap();
        assert_eq!(cfg.default_stack, "console");
        let names: Vec<&str> = cfg.stacks.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["harness", "console"]);
        // Built-in harness roots = HARNESS_STACK const filtered to discovered.
        assert_eq!(cfg.stacks[0].1, vec!["session-manager", "harness"]);
        assert_eq!(cfg.stacks[1].1, vec!["console", "session-manager"]);
        // Bridge until Task 4: harness_stack carries the DEFAULT stack's roots.
        assert_eq!(cfg.harness_stack, vec!["console", "session-manager"]);
    }

    #[test]
    fn stacks_keep_yaml_definition_order() {
        let tmp = TempDir::new().unwrap();
        write_repo_multi(&tmp);
        let cfg = load_with_yaml(
            &tmp,
            "stacks:\n  zebra: [console]\n  alpha: [session-manager]\n",
        )
        .unwrap();
        let names: Vec<&str> = cfg.stacks.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["harness", "zebra", "alpha"]);
        assert_eq!(cfg.default_stack, "harness");
    }

    #[test]
    fn stacks_harness_entry_overrides_builtin_roots() {
        let tmp = TempDir::new().unwrap();
        write_repo_multi(&tmp);
        let cfg = load_with_yaml(&tmp, "stacks:\n  harness: [console]\n").unwrap();
        assert_eq!(cfg.stacks.len(), 1);
        assert_eq!(cfg.stacks[0].0, "harness");
        assert_eq!(cfg.stacks[0].1, vec!["console"]);
    }

    #[test]
    fn unknown_default_stack_fails() {
        let tmp = TempDir::new().unwrap();
        write_repo_multi(&tmp);
        let err = load_with_yaml(&tmp, "default_stack: nope\n").unwrap_err();
        assert!(err.to_string().contains("not a defined stack"), "{err:#}");
    }

    #[test]
    fn removed_harness_stack_key_fails_with_rename_hint() {
        let tmp = TempDir::new().unwrap();
        write_repo_multi(&tmp);
        let err = load_with_yaml(&tmp, "harness_stack:\n  - harness\n").unwrap_err();
        assert!(err.to_string().contains("replaced by `stacks:`"), "{err:#}");
    }

    #[test]
    fn unknown_stack_roots_warn_and_drop() {
        let tmp = TempDir::new().unwrap();
        write_repo_multi(&tmp);
        let cfg = load_with_yaml(&tmp, "stacks:\n  console: [console, bogus]\n").unwrap();
        let console = cfg.stacks.iter().find(|(n, _)| n == "console").unwrap();
        assert_eq!(console.1, vec!["console"]);
    }

    #[test]
    fn empty_default_stack_after_filtering_fails() {
        let tmp = TempDir::new().unwrap();
        write_repo_multi(&tmp);
        let err =
            load_with_yaml(&tmp, "stacks:\n  ghost: [bogus]\ndefault_stack: ghost\n").unwrap_err();
        assert!(err.to_string().contains("no startable workers"), "{err:#}");
    }

    #[test]
    fn non_list_stack_value_fails() {
        let tmp = TempDir::new().unwrap();
        write_repo_multi(&tmp);
        let err = load_with_yaml(&tmp, "stacks:\n  console: 5\n").unwrap_err();
        assert!(
            err.to_string().contains("expected a list of worker names"),
            "{err:#}"
        );
    }
}
