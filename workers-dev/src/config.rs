//! Everything `workers-dev` knows comes from one file: the Compose project at
//! `harness/worker-compose.yaml`. Inventory, dependency edges, the engine URL
//! and the namespace are read from there; nothing is discovered, nothing is
//! written back.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::color::ColorMode;

pub const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";
/// The Compose project, relative to the repo root. Also what identifies the
/// repo root during the ancestor walk.
pub const COMPOSE_FILE_REL: &str = "harness/worker-compose.yaml";
/// Where the on-demand workers are declared: one compose project per worker,
/// gitignored and owned by this tool. One file each, not one shared file —
/// the daemon holds a project as its file was when it loaded it, so adding a
/// container to a loaded project would need a whole-project restart, and every
/// worker already running in it would bounce. A new file is a new project.
pub const LOCAL_DIR_REL: &str = "harness/.workers-dev";
/// Every worker in this repo, and every worker outside it worth offering,
/// carries one.
const MANIFEST_FILE: &str = "iii.worker.yaml";
/// A worker outside this repo often ships the compose container it wants.
const SELF_COMPOSE_FILE: &str = "worker-compose.yaml";
/// Where the spawned `iii compose` daemon's stdout and stderr land. A file,
/// never a pipe: compose prints its banner with bare `println!`, and Rust
/// ignores SIGPIPE, so a closed read end panics the daemon.
pub const DAEMON_LOG_REL: &str = "harness/.workers-dev.log";
/// The env file the compose containers read at every spawn. Gitignored; the
/// tracked `harness/workers-dev.env.example` is the fallback for a plain
/// `iii compose --up`.
pub const UI_WATCH_ENV_REL: &str = "harness/.env.workers-dev";

pub const POLL_INTERVAL_MS: u64 = 1000;
pub const DAEMON_READY_TIMEOUT_MS: u64 = 60_000;
pub const LIFECYCLE_TIMEOUT_MS: u64 = 600_000;
pub const STATUS_TIMEOUT_MS: u64 = 10_000;
pub const STOP_TIMEOUT_MS: u64 = 30_000;
/// How much of a container log to keep on screen. The active segment compose
/// writes grows to 10 MiB, so the pane seeks instead of reading the file.
pub const LOG_TAIL_BYTES: u64 = 64 * 1024;

/// One `containers:` entry, reduced to what the dashboard needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerSpec {
    pub name: String,
    /// `start_after` — the compose DAG, not a guess.
    pub deps: Vec<String>,
    /// `<worker>/ui` when that directory ships a `watch` script. `None`
    /// otherwise, which is what excludes a worker that has a `ui/` but only
    /// builds it.
    pub ui_dir: Option<PathBuf>,
}

/// A worker that lives in this repo but is not declared by the tracked compose
/// file. Discovered from its `iii.worker.yaml`, which every worker here has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoWorker {
    pub name: String,
    /// Where it lives. Absolute, because it is not always under the repo root.
    pub dir: PathBuf,
    /// The container this worker declares for itself, when it ships a
    /// `worker-compose.yaml`. It knows things a synthesized declaration cannot
    /// guess — `harness-e2e` refuses to start without the `config_override`
    /// its own file carries.
    pub declared: Option<serde_yaml::Value>,
    /// The binary to `cargo run --bin`. `None` for the workers that are not
    /// Rust binaries — they install from the registry, not from this tree.
    pub bin: Option<String>,
    pub ui_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub repo_root: PathBuf,
    /// Absolute and canonical: every compose call carries it as the project id.
    pub compose_path: PathBuf,
    pub local_dir: PathBuf,
    pub daemon_log_path: PathBuf,
    pub ui_watch_env_path: PathBuf,
    pub namespace: String,
    pub engine_url: String,
    pub engine_host: String,
    pub engine_port: u16,
    pub workers: Vec<WorkerSpec>,
    /// Everything else the repo ships, in name order — the list the dashboard
    /// offers for an on-demand start.
    pub repo_workers: Vec<RepoWorker>,
    pub color_mode: ColorMode,
    /// Turn every watchable container's UI watcher on at launch.
    pub ui_watch: bool,
}

#[derive(Debug, Deserialize)]
struct ComposeFile {
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    engine: Option<ComposeEngine>,
    /// A `Mapping` rather than a map type so declaration order survives — the
    /// dashboard lists containers in the order the file does.
    #[serde(default)]
    containers: serde_yaml::Mapping,
}

#[derive(Debug, Deserialize)]
struct ComposeEngine {
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ComposeContainer {
    worker: String,
    #[serde(default)]
    start_after: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct WorkerManifest {
    #[serde(default)]
    name: Option<String>,
    /// `binary` is what `cargo run` can start; anything else comes from the
    /// registry and is not ours to launch from source.
    #[serde(default)]
    deploy: Option<String>,
    #[serde(default)]
    bin: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UiPackage {
    #[serde(default)]
    scripts: HashMap<String, String>,
}

impl Config {
    pub fn load(
        repo: Option<PathBuf>,
        worker_dirs: Vec<PathBuf>,
        namespace: Option<String>,
        color: Option<String>,
        ui_watch: bool,
    ) -> Result<Self> {
        let repo_root = resolve_repo_root(repo)?;
        refuse_legacy_config(&repo_root)?;

        let compose_path = repo_root
            .join(COMPOSE_FILE_REL)
            .canonicalize()
            .with_context(|| format!("{COMPOSE_FILE_REL} under {}", repo_root.display()))?;
        let compose_dir = compose_path.parent().unwrap_or(&repo_root).to_path_buf();

        let text = std::fs::read_to_string(&compose_path)
            .with_context(|| format!("read {}", compose_path.display()))?;
        let file: ComposeFile = serde_yaml::from_str(&text)
            .with_context(|| format!("parse {}", compose_path.display()))?;

        let file_namespace = match file.namespace.as_deref() {
            Some(raw) => Some(expand_env(raw).context("compose namespace")?),
            None => None,
        };
        let namespace = namespace
            .or(file_namespace)
            .unwrap_or_else(|| "default".to_string());

        let engine_url = match file
            .engine
            .as_ref()
            .and_then(|engine| engine.url.as_deref())
        {
            Some(raw) => expand_env(raw).context("compose engine.url")?,
            None => DEFAULT_ENGINE_URL.to_string(),
        };
        let (engine_host, engine_port) = parse_engine_url(&engine_url)?;

        let workers = parse_containers(&file.containers, &compose_dir)?;
        let repo_workers = discover_workers(&repo_root, &worker_dirs, &workers);

        Ok(Self {
            local_dir: repo_root.join(LOCAL_DIR_REL),
            daemon_log_path: repo_root.join(DAEMON_LOG_REL),
            ui_watch_env_path: repo_root.join(UI_WATCH_ENV_REL),
            repo_root,
            compose_path,
            namespace,
            engine_url,
            engine_host,
            engine_port,
            workers,
            repo_workers,
            color_mode: color
                .as_deref()
                .and_then(ColorMode::parse)
                .unwrap_or_default(),
            ui_watch,
        })
    }

    pub fn worker(&self, name: &str) -> Option<&WorkerSpec> {
        self.workers.iter().find(|worker| worker.name == name)
    }

    #[cfg(test)]
    pub fn names(&self) -> Vec<&str> {
        self.workers
            .iter()
            .map(|worker| worker.name.as_str())
            .collect()
    }

    /// Everything that would go down with `name`, transitively — the blast
    /// radius `compose::down` applies, shown before it is applied.
    /// The compose project for one on-demand worker.
    pub fn local_file(&self, worker: &str) -> PathBuf {
        self.local_dir.join(format!("{worker}.yaml"))
    }

    pub fn dependents(&self, name: &str) -> Vec<String> {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut queue: VecDeque<&str> = VecDeque::from([name]);
        let mut out = Vec::new();
        while let Some(current) = queue.pop_front() {
            for worker in &self.workers {
                if worker.deps.iter().any(|dep| dep == current) && seen.insert(&worker.name) {
                    out.push(worker.name.clone());
                    queue.push_back(&worker.name);
                }
            }
        }
        out
    }
}

/// Every worker the compose file does not already declare: the repo's own, and
/// whatever `--worker-dir` adds.
///
/// A root that carries an `iii.worker.yaml` *is* one worker (`harness-e2e`);
/// otherwise its children are scanned (a monorepo like this one). Read loosely:
/// an unparsable manifest is a worker we cannot offer, not a reason to refuse
/// to start. The repo comes first, so a name it already uses wins.
fn discover_workers(
    repo_root: &Path,
    extra: &[PathBuf],
    declared: &[WorkerSpec],
) -> Vec<RepoWorker> {
    let mut workers: Vec<RepoWorker> = Vec::new();
    let mut seen: HashSet<String> = declared.iter().map(|worker| worker.name.clone()).collect();

    for root in std::iter::once(repo_root.to_path_buf()).chain(extra.iter().cloned()) {
        let dirs: Vec<PathBuf> = if root.join(MANIFEST_FILE).is_file() {
            vec![root]
        } else {
            let Ok(entries) = std::fs::read_dir(&root) else {
                continue;
            };
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect()
        };
        for dir in dirs {
            let Some(worker) = read_worker(&dir) else {
                continue;
            };
            if seen.insert(worker.name.clone()) {
                workers.push(worker);
            }
        }
    }
    workers.sort_by(|a, b| a.name.cmp(&b.name));
    workers
}

fn read_worker(dir: &Path) -> Option<RepoWorker> {
    let text = std::fs::read_to_string(dir.join(MANIFEST_FILE)).ok()?;
    let manifest: WorkerManifest = serde_yaml::from_str(&text).ok()?;
    let name = manifest
        .name
        .or_else(|| dir.file_name()?.to_str().map(str::to_string))?;
    let declared = declared_container(dir, &name);
    // A worker that declares its own container can be started whatever its
    // deploy kind says: the declaration carries the command.
    let bin = (declared.is_some() || manifest.deploy.as_deref() == Some("binary"))
        .then(|| manifest.bin.clone().unwrap_or_else(|| name.clone()));
    Some(RepoWorker {
        ui_dir: watchable_ui_dir(dir),
        dir: dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()),
        declared,
        name,
        bin,
    })
}

/// The container a worker declares for itself. Its own key when there is one,
/// otherwise the sole container — a single-worker project names it whatever
/// it likes.
fn declared_container(dir: &Path, name: &str) -> Option<serde_yaml::Value> {
    let text = std::fs::read_to_string(dir.join(SELF_COMPOSE_FILE)).ok()?;
    let file: serde_yaml::Value = serde_yaml::from_str(&text).ok()?;
    let containers = file.get("containers")?.as_mapping()?;
    containers
        .get(serde_yaml::Value::from(name))
        .cloned()
        .or_else(|| {
            (containers.len() == 1)
                .then(|| containers.values().next().cloned())
                .flatten()
        })
}

/// Colon-separated, like `PATH`: export it once rather than passing
/// `--worker-dir` on every launch.
pub fn worker_dirs(explicit: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let from_env = std::env::var("WORKERS_DEV_WORKER_DIRS")
        .unwrap_or_default()
        .split(':')
        .filter(|entry| !entry.trim().is_empty())
        .map(PathBuf::from)
        .collect::<Vec<_>>();

    explicit
        .into_iter()
        .chain(from_env)
        .map(|dir| {
            dir.canonicalize()
                .with_context(|| format!("worker directory {}", dir.display()))
        })
        .collect()
}

fn parse_containers(
    containers: &serde_yaml::Mapping,
    compose_dir: &Path,
) -> Result<Vec<WorkerSpec>> {
    let mut workers = Vec::new();
    for (key, value) in containers {
        let name = key
            .as_str()
            .with_context(|| format!("containers: key {key:?} is not a string"))?
            .to_string();
        let container: ComposeContainer =
            serde_yaml::from_value(value.clone()).with_context(|| format!("containers.{name}"))?;
        let ui_dir = container
            .worker
            .strip_prefix("path://")
            .map(|relative| compose_dir.join(relative))
            .and_then(|dir| watchable_ui_dir(&dir));
        workers.push(WorkerSpec {
            name,
            deps: container.start_after,
            ui_dir,
        });
    }
    if workers.is_empty() {
        bail!("the compose file declares no containers");
    }
    Ok(workers)
}

/// A worker is watchable only when its UI project declares a `watch` script.
/// That one rule is what keeps a build-only `ui/` out of the watcher column
/// without naming any worker.
fn watchable_ui_dir(worker_dir: &Path) -> Option<PathBuf> {
    let ui_dir = worker_dir.join("ui");
    let package = std::fs::read_to_string(ui_dir.join("package.json")).ok()?;
    let parsed: UiPackage = serde_json::from_str(&package).ok()?;
    parsed
        .scripts
        .contains_key("watch")
        .then(|| ui_dir.canonicalize().unwrap_or(ui_dir))
}

/// The compose file's own `${NAME}` / `${NAME:-default}` syntax, applied to
/// the two strings this tool reads out of it. Compose expands the whole
/// document itself; we only need the same answer for `namespace` and
/// `engine.url`, so this is a string pass, not a document walker.
pub fn expand_env(raw: &str) -> Result<String> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let end = tail
            .find('}')
            .with_context(|| format!("unterminated ${{ in {raw:?}"))?;
        let expression = &tail[..end];
        let (name, default) = match expression.split_once(":-") {
            Some((name, default)) => (name, Some(default)),
            None => (expression, None),
        };
        match (std::env::var(name).ok(), default) {
            (Some(value), _) => out.push_str(&value),
            (None, Some(default)) => out.push_str(default),
            (None, None) => bail!("{raw:?} needs ${{{name}}}, which is not set"),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// `workers-dev.yaml` is gitignored, so every checkout on this machine still
/// has one. Silently ignoring it would drop someone's `engine_url` without a
/// word, so it is a hard error that says where the settings went.
fn refuse_legacy_config(repo_root: &Path) -> Result<()> {
    let path = repo_root.join("workers-dev.yaml");
    if path.is_file() {
        bail!(
            "{} is no longer read — workers, dependencies, env and the engine URL now come \
             from {COMPOSE_FILE_REL}. Delete it; for a second worktree use \
             `III_ENGINE_PORT=<port> workers-dev --namespace <ns>`.",
            path.display()
        );
    }
    Ok(())
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
        for ancestor in start.ancestors() {
            if validate_repo_root(ancestor).is_ok() {
                if let Ok(root) = ancestor.canonicalize() {
                    return Ok(root);
                }
            }
        }
    }

    bail!(
        "could not find the workers repo root (expected {COMPOSE_FILE_REL}); \
         pass --repo or set WORKERS_DEV_REPO"
    )
}

fn validate_repo_root(path: &Path) -> Result<()> {
    if path.join(COMPOSE_FILE_REL).is_file() {
        Ok(())
    } else {
        bail!("no {COMPOSE_FILE_REL} under {}", path.display())
    }
}

/// Host and port out of a `ws://host:port` engine URL. compose's managed
/// engine is probed over plain TCP before any WebSocket is attempted.
pub fn parse_engine_url(url: &str) -> Result<(String, u16)> {
    let stripped = url
        .strip_prefix("ws://")
        .or_else(|| url.strip_prefix("wss://"))
        .unwrap_or(url);
    let authority = stripped.split('/').next().unwrap_or(stripped);

    Ok(match authority.rsplit_once(':') {
        Some((host, port)) => (
            host.to_string(),
            port.parse().with_context(|| {
                format!("invalid port in engine url {url} (port must be a number, e.g. 49134)")
            })?,
        ),
        None => (authority.to_string(), 49134),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_fixture() -> tempfile::TempDir {
        let repo = tempfile::tempdir().unwrap();
        let harness = repo.path().join("harness");
        std::fs::create_dir_all(&harness).unwrap();
        std::fs::write(
            harness.join("worker-compose.yaml"),
            "namespace: fixture-ns\n\
             engine:\n  url: ws://127.0.0.1:${III_TEST_PORT:-49134}\n\
             containers:\n\
             \x20 state:\n    worker: path://../state\n\
             \x20 llm-router:\n    worker: path://../llm-router\n    start_after: [state]\n\
             \x20 harness:\n    worker: path://.\n    start_after: [llm-router]\n",
        )
        .unwrap();
        for (worker, script) in [("state", Some("watch")), ("llm-router", Some("build"))] {
            let ui = repo.path().join(worker).join("ui");
            std::fs::create_dir_all(&ui).unwrap();
            std::fs::write(
                ui.join("package.json"),
                format!("{{\"scripts\": {{\"{}\": \"x\"}}}}", script.unwrap()),
            )
            .unwrap();
        }
        repo
    }

    fn load_with(repo: &tempfile::TempDir, extra: Vec<PathBuf>) -> Config {
        Config::load(
            Some(repo.path().to_path_buf()),
            extra,
            None,
            Some("never".into()),
            false,
        )
        .unwrap()
    }

    fn load(repo: &tempfile::TempDir) -> Config {
        load_with(repo, Vec::new())
    }

    // Pins `start_after` (not `depends_on`, whose `#[serde(default)]` would
    // silently yield an empty graph) and the watch-script rule in one go.
    #[test]
    fn compose_file_is_the_worker_inventory() {
        let repo = repo_fixture();
        let config = load(&repo);
        assert_eq!(config.names(), ["state", "llm-router", "harness"]);
        assert_eq!(config.worker("llm-router").unwrap().deps, ["state"]);
        assert!(config.worker("state").unwrap().ui_dir.is_some());
        // ships a ui/ but only builds it
        assert!(config.worker("llm-router").unwrap().ui_dir.is_none());
        assert_eq!(config.namespace, "fixture-ns");
    }

    #[test]
    fn dependents_are_transitive() {
        let repo = repo_fixture();
        let config = load(&repo);
        assert_eq!(config.dependents("state"), ["llm-router", "harness"]);
        assert!(config.dependents("harness").is_empty());
    }

    // `expand_env` is the one hand-written parser here and the whole
    // multi-worktree story rests on it.
    #[test]
    fn engine_url_expands_the_port_default() {
        let repo = repo_fixture();
        assert_eq!(load(&repo).engine_port, 49134);
        assert_eq!(expand_env("p=${III_TEST_UNSET:-7}").unwrap(), "p=7");
        assert!(expand_env("${III_TEST_UNSET}").is_err());
        assert!(expand_env("${III_TEST_UNTERMINATED").is_err());
    }

    #[test]
    fn a_workers_dev_yaml_is_refused_with_a_pointer() {
        let repo = repo_fixture();
        std::fs::write(repo.path().join("workers-dev.yaml"), "release: true\n").unwrap();
        let error = Config::load(
            Some(repo.path().to_path_buf()),
            Vec::new(),
            None,
            None,
            false,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("workers-dev.yaml"), "{error}");
        assert!(error.contains("worker-compose.yaml"), "{error}");
    }

    // The list the dashboard offers for an on-demand start, and the rule that
    // decides which of them this tool can actually launch from source.
    #[test]
    fn repo_workers_are_everything_the_compose_file_does_not_declare() {
        let repo = repo_fixture();
        for (worker, deploy) in [("browser", "binary"), ("scrapling", "bundle")] {
            std::fs::create_dir_all(repo.path().join(worker)).unwrap();
            std::fs::write(
                repo.path().join(worker).join("iii.worker.yaml"),
                format!("iii: v1\nname: {worker}\ndeploy: {deploy}\nbin: {worker}\n"),
            )
            .unwrap();
        }
        // Declared by the compose file, so it belongs to the stack, not the list.
        std::fs::write(
            repo.path().join("state").join("iii.worker.yaml"),
            "iii: v1\nname: state\ndeploy: binary\n",
        )
        .unwrap();

        let config = load(&repo);
        let names: Vec<&str> = config
            .repo_workers
            .iter()
            .map(|worker| worker.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["browser", "scrapling"],
            "name order, stack excluded"
        );
        assert_eq!(config.repo_workers[0].bin.as_deref(), Some("browser"));
        // Not a Rust binary: offered, but never started with `cargo run`.
        assert_eq!(config.repo_workers[1].bin, None);
    }

    /// A sibling project that is itself one worker — the `harness-e2e` shape.
    #[test]
    fn a_worker_dir_outside_the_repo_joins_the_list() {
        let repo = repo_fixture();
        let outside = tempfile::tempdir().unwrap();
        let worker = outside.path().join("harness-e2e");
        std::fs::create_dir_all(&worker).unwrap();
        std::fs::write(
            worker.join("iii.worker.yaml"),
            "iii: v1\nname: harness-e2e\ndeploy: binary\nbin: harness-e2e\n",
        )
        .unwrap();

        // Pointed at the worker itself, not at a directory of workers.
        let config = load_with(&repo, vec![worker.clone()]);
        let found = config
            .repo_workers
            .iter()
            .find(|candidate| candidate.name == "harness-e2e")
            .expect("the outside worker is offered");
        assert_eq!(found.bin.as_deref(), Some("harness-e2e"));
        assert_eq!(found.dir, worker.canonicalize().unwrap());

        // Pointed at its parent, which is a directory of workers instead.
        let config = load_with(&repo, vec![outside.path().to_path_buf()]);
        assert!(config
            .repo_workers
            .iter()
            .any(|candidate| candidate.name == "harness-e2e"));
    }

    /// The repo is scanned first, so a name it already uses is not replaced by
    /// a stranger — and a worker the stack declares never appears at all.
    #[test]
    fn the_repo_wins_a_name_collision() {
        let repo = repo_fixture();
        std::fs::create_dir_all(repo.path().join("browser")).unwrap();
        std::fs::write(
            repo.path().join("browser/iii.worker.yaml"),
            "iii: v1\nname: browser\ndeploy: binary\n",
        )
        .unwrap();

        let outside = tempfile::tempdir().unwrap();
        for name in ["browser", "state"] {
            let dir = outside.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("iii.worker.yaml"),
                format!("iii: v1\nname: {name}\ndeploy: binary\n"),
            )
            .unwrap();
        }

        let config = load_with(&repo, vec![outside.path().to_path_buf()]);
        let browser: Vec<&RepoWorker> = config
            .repo_workers
            .iter()
            .filter(|worker| worker.name == "browser")
            .collect();
        assert_eq!(browser.len(), 1, "one entry per name");
        assert_eq!(
            browser[0].dir,
            repo.path().join("browser").canonicalize().unwrap()
        );
        // `state` is a container in the compose file; it is the stack's, not a
        // candidate to start on demand.
        assert!(!config
            .repo_workers
            .iter()
            .any(|worker| worker.name == "state"));
    }

    // The only check that fails when the tracked compose file drifts — the
    // next ade/ide-style rename lands here first.
    #[test]
    fn harness_compose_declares_the_real_inventory() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let config =
            Config::load(Some(repo), Vec::new(), None, Some("never".into()), false).unwrap();
        assert_eq!(
            config.names(),
            [
                "queue",
                "state",
                "session-manager",
                "llm-router",
                "provider-openai-codex",
                "provider-openai",
                "provider-anthropic",
                "context-manager",
                "iii-directory",
                "cron",
                "ade",
                "ide",
                "harness",
            ]
        );
        let watchable: Vec<&str> = config
            .workers
            .iter()
            .filter(|worker| worker.ui_dir.is_some())
            .map(|worker| worker.name.as_str())
            .collect();
        assert_eq!(
            watchable,
            [
                "state",
                "llm-router",
                "context-manager",
                "iii-directory",
                "cron",
                "ade",
                "ide",
                "harness"
            ]
        );
        assert_eq!(config.worker("harness").unwrap().deps.len(), 12);
        // The rest of the repo is what the dashboard can start on demand.
        assert!(
            config.repo_workers.len() > 40,
            "{}",
            config.repo_workers.len()
        );
        assert!(config
            .repo_workers
            .iter()
            .all(|worker| config.worker(&worker.name).is_none()));
        assert!(config
            .repo_workers
            .iter()
            .any(|worker| worker.name == "database"));
    }

    #[test]
    fn parse_default_url() {
        assert_eq!(
            parse_engine_url(DEFAULT_ENGINE_URL).unwrap(),
            ("127.0.0.1".to_string(), 49134)
        );
        assert!(parse_engine_url("ws://127.0.0.1:nope").is_err());
    }
}
