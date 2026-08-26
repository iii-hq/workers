//! Worker runtime config.
//!
//! The skills worker is a filesystem-backed reader plus a
//! `directory::skills::download` function that pulls markdown into the
//! configured `skills_folder`. There is no state-backed registry, no
//! glob arrays, no scopes — everything lives on disk under one root.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use arc_swap::ArcSwap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Shared, hot-reloadable config handle. Handlers snapshot the current
/// value per call (`handle.load_full()`, lock-free) so a
/// `configuration:updated` reload that `store`s a new value takes effect
/// on the next invocation without re-registering any function.
pub type SharedConfig = Arc<ArcSwap<SkillsConfig>>;

/// Default base URL for the workers registry. Overrideable via
/// `registry_url:` in the config so self-hosted deployments can repoint.
pub const DEFAULT_REGISTRY_URL: &str = "https://api.workers.iii.dev";

/// Project-relative destination for downloaded skills.
pub const DEFAULT_SKILLS_FOLDER: &str = "skills";

/// Project-relative destination for local skill overrides.
pub const DEFAULT_LOCAL_SKILLS_FOLDER: &str = "skills/iii";

/// Project-relative destination for reusable agent profiles.
pub const DEFAULT_AGENTS_FOLDER: &str = "agents";

/// Project-relative root for agent skills. Read-only; never written by this worker.
pub const DEFAULT_AGENTS_SKILLS_FOLDER: &str = ".agents/skills";

fn default_skills_folder() -> String {
    iii_worker_paths::default_path(DEFAULT_SKILLS_FOLDER)
}

fn default_local_skills_folder() -> String {
    iii_worker_paths::default_path(DEFAULT_LOCAL_SKILLS_FOLDER)
}

fn default_agents_folder() -> String {
    iii_worker_paths::default_path(DEFAULT_AGENTS_FOLDER)
}

fn default_agents_skills_folder() -> String {
    iii_worker_paths::default_path(DEFAULT_AGENTS_SKILLS_FOLDER)
}

fn default_registry_url() -> String {
    DEFAULT_REGISTRY_URL.to_string()
}

fn default_download_timeout_ms() -> u64 {
    60_000
}

fn default_registry_cache_ttl_ms() -> u64 {
    60_000
}

fn default_filter_unregistered() -> bool {
    true
}

fn default_auto_download() -> bool {
    true
}

#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct SkillsConfig {
    /// Folder that backs every read (`directory::skills::list`,
    /// `directory::skills::get`, `directory::system-prompts::*`) and every
    /// write from `directory::skills::download`. Supports three forms:
    ///
    /// - Absolute path — used as-is.
    /// - `~`-prefixed — expands leading `~` via `dirs::home_dir()`.
    /// - Relative — resolved against `III_COMPOSE_DIR` when available.
    #[serde(default = "default_skills_folder")]
    pub skills_folder: String,

    /// Folder for local (project-scoped) skill overrides. A namespace
    /// directory present under this root shadows the same namespace in
    /// the global `skills_folder` entirely (whole-namespace override).
    /// Supports the same three resolution forms as `skills_folder`.
    #[serde(default = "default_local_skills_folder")]
    pub local_skills_folder: String,

    /// Read-write root for reusable agent profile Markdown files. Supports
    /// the same three resolution forms as `skills_folder`.
    #[serde(default = "default_agents_folder")]
    pub agents_folder: String,

    /// Read-only root for system-installed agent skills (the
    /// `~/.agents/skills` convention: one directory per skill, each
    /// containing a `SKILL.md`). Scanned shallowly — only
    /// `<skill>/SKILL.md`, never the skill's `reference/`/`scripts/`
    /// payload. A missing directory is silently treated as empty; the
    /// worker never creates or writes under this root. Namespaces here
    /// are shadowed by the same namespace under `skills_folder` or
    /// `local_skills_folder`. Supports the same three resolution forms
    /// as `skills_folder`.
    #[serde(default = "default_agents_skills_folder")]
    pub agents_skills_folder: String,

    /// Workers registry base URL — used by `directory::skills::download`
    /// and the `directory::registry::*` proxies when a `worker=` source
    /// is specified. Stored without a trailing slash.
    #[serde(default = "default_registry_url")]
    pub registry_url: String,

    /// Timeout for a single download operation (HTTP request OR `git clone`)
    /// in milliseconds. Also used as the request timeout for
    /// `directory::registry::*` HTTP calls.
    #[serde(default = "default_download_timeout_ms")]
    pub download_timeout_ms: u64,

    /// TTL (in milliseconds) for cached
    /// `directory::registry::workers::list` and
    /// `directory::registry::workers::info` responses. Repeat lookups
    /// within this window skip the HTTP call. Set to 0 to disable
    /// caching.
    #[serde(default = "default_registry_cache_ttl_ms")]
    pub registry_cache_ttl_ms: u64,

    /// When `true` (default), read functions hide skills whose top
    /// namespace segment doesn't match a registered (installed) worker
    /// name. Orphan namespaces are hidden. When `false`, all scanned
    /// skills are returned regardless of installed workers.
    #[serde(default = "default_filter_unregistered")]
    pub filter_unregistered: bool,

    /// When `true` (default), the worker subscribes to `worker` trigger
    /// events and runs a boot-time reconcile to auto-download skills
    /// for installed workers that are missing from the global skills
    /// folder.
    #[serde(default = "default_auto_download")]
    pub auto_download: bool,

    /// Bind the `directory::pre-generate` hook so the conditional search
    /// hint can be injected into agent generations. On by default; turning
    /// it off unbinds the hook entirely (hot, no restart) — the model then
    /// only finds `directory::search_functions` through normal discovery.
    #[serde(default = "default_inject_hint")]
    pub inject_hint: bool,

    /// Only inject the search hint when the session's surface spans at
    /// least this many distinct non-engine workers. Narrower surfaces
    /// resolve faster through normal discovery than through a standing
    /// hint; `0` hints on every surface.
    #[serde(default = "default_hint_min_workers")]
    pub hint_min_workers: usize,

    /// Also search the public worker registry (verified authors only) on
    /// every `directory::search_functions` call and return matching
    /// NOT-installed workers as an `installable` section alongside the
    /// installed results. Fail-open: a registry error just omits the
    /// section.
    #[serde(default = "default_registry_search")]
    pub registry_search: bool,
}

fn default_inject_hint() -> bool {
    true
}

fn default_hint_min_workers() -> usize {
    2
}

fn default_registry_search() -> bool {
    true
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            skills_folder: default_skills_folder(),
            local_skills_folder: default_local_skills_folder(),
            agents_folder: default_agents_folder(),
            agents_skills_folder: default_agents_skills_folder(),
            registry_url: default_registry_url(),
            download_timeout_ms: default_download_timeout_ms(),
            registry_cache_ttl_ms: default_registry_cache_ttl_ms(),
            filter_unregistered: default_filter_unregistered(),
            auto_download: default_auto_download(),
            inject_hint: default_inject_hint(),
            hint_min_workers: default_hint_min_workers(),
            registry_search: default_registry_search(),
        }
    }
}

/// Resolve a path string supporting three forms:
///
/// - `~`-prefixed: expand leading `~` via the process home directory.
///   Leave the path unchanged if no home directory is available.
/// - Absolute: returned as-is.
/// - Relative: resolved against `III_COMPOSE_DIR`, or the process current
///   working directory when the worker runs outside Compose.
impl SkillsConfig {
    /// Absolute path to the configured global skills folder.
    pub fn resolved_skills_folder(&self) -> PathBuf {
        iii_worker_paths::resolve_path(&self.skills_folder)
    }

    /// Absolute path to the configured local skills folder.
    pub fn local_skills_folder(&self) -> PathBuf {
        iii_worker_paths::resolve_path(&self.local_skills_folder)
    }

    /// Absolute path to the configured read-write agent profile folder.
    pub fn resolved_agents_folder(&self) -> PathBuf {
        iii_worker_paths::resolve_path(&self.agents_folder)
    }

    /// Absolute path to the configured (read-only) agents skills folder.
    pub fn resolved_agents_skills_folder(&self) -> PathBuf {
        iii_worker_paths::resolve_path(&self.agents_skills_folder)
    }

    /// Registry base URL with any trailing slash trimmed so callers can
    /// build URLs as `format!("{base}/w/{worker}/skills")`.
    pub fn registry_base(&self) -> &str {
        self.registry_url.trim_end_matches('/')
    }

    /// Restart-requiring fields. A `configuration:updated` reload that
    /// changes any of these is refused (logged "restart required"):
    /// `skills_folder` / `local_skills_folder` / `agents_folder` /
    /// `agents_skills_folder` are the on-disk read/write roots baked into
    /// running tasks (and the fs-watch root set), and `auto_download` wires
    /// the `worker`-trigger subscription + boot reconcile at startup — none
    /// can be re-wired safely in place.
    pub fn topology(&self) -> Topology {
        Topology {
            skills_folder: self.skills_folder.clone(),
            local_skills_folder: self.local_skills_folder.clone(),
            agents_folder: self.agents_folder.clone(),
            agents_skills_folder: self.agents_skills_folder.clone(),
            auto_download: self.auto_download,
        }
    }

    /// JSON Schema registered with the `configuration` worker so the
    /// console can render an editor for the `iii-directory` entry.
    pub fn json_schema() -> Value {
        let root = schemars::schema_for!(SkillsConfig);
        let mut schema =
            serde_json::to_value(&root.schema).expect("SkillsConfig JSON Schema serializes");
        if let Some(obj) = schema.as_object_mut() {
            if !root.definitions.is_empty() {
                obj.insert(
                    "definitions".into(),
                    serde_json::to_value(&root.definitions).expect("definitions serialize"),
                );
            }
            obj.insert("example".into(), SkillsConfig::default().to_json());
        }
        schema
    }

    /// Parse a YAML seed file. Used only to build `initial_value` on first
    /// `configuration::register`; the configuration worker env-expands
    /// `${VAR}` on read, so no local expansion is done here.
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        serde_yaml::from_str(yaml).map_err(|e| format!("yaml parse: {e}"))
    }

    /// Read and parse a YAML seed file from disk.
    pub fn from_file(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::from_yaml(&raw)
    }

    /// Parse the authoritative value returned by `configuration::get`.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|e| format!("json parse: {e}"))
    }

    /// Serialize for `initial_value` on register.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("SkillsConfig serializes")
    }

    /// Wrap into a shared, hot-reloadable handle (see [`SharedConfig`]).
    pub fn into_shared(self) -> SharedConfig {
        Arc::new(ArcSwap::from_pointee(self))
    }
}

/// Restart-requiring subset of [`SkillsConfig`] (see [`SkillsConfig::topology`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Topology {
    pub skills_folder: String,
    pub local_skills_folder: String,
    pub agents_folder: String,
    pub agents_skills_folder: String,
    pub auto_download: bool,
}

pub fn load_config(path: &str) -> Result<SkillsConfig> {
    let contents = std::fs::read_to_string(path)?;
    let cfg: SkillsConfig = serde_yaml::from_str(&contents)?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: SkillsConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.skills_folder, default_skills_folder());
        assert_eq!(cfg.local_skills_folder, default_local_skills_folder());
        assert_eq!(cfg.agents_folder, default_agents_folder());
        assert_eq!(cfg.agents_skills_folder, default_agents_skills_folder());
        assert_eq!(cfg.registry_url, DEFAULT_REGISTRY_URL);
        assert_eq!(cfg.download_timeout_ms, 60_000);
        assert_eq!(cfg.registry_cache_ttl_ms, 60_000);
        assert!(cfg.filter_unregistered);
        assert!(cfg.auto_download);
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let from_empty: SkillsConfig = serde_yaml::from_str("{}").unwrap();
        let from_default = SkillsConfig::default();
        assert_eq!(from_empty.skills_folder, from_default.skills_folder);
        assert_eq!(
            from_empty.local_skills_folder,
            from_default.local_skills_folder
        );
        assert_eq!(from_empty.agents_folder, from_default.agents_folder);
        assert_eq!(
            from_empty.agents_skills_folder,
            from_default.agents_skills_folder
        );
        assert_eq!(from_empty.registry_url, from_default.registry_url);
        assert_eq!(
            from_empty.download_timeout_ms,
            from_default.download_timeout_ms
        );
        assert_eq!(
            from_empty.registry_cache_ttl_ms,
            from_default.registry_cache_ttl_ms
        );
        assert_eq!(
            from_empty.filter_unregistered,
            from_default.filter_unregistered
        );
        assert_eq!(from_empty.auto_download, from_default.auto_download);
    }

    #[test]
    fn custom_yaml_overrides_each_field() {
        let yaml = "\
skills_folder: ./my-skills
local_skills_folder: ./local-skills
agents_folder: ./profiles
agents_skills_folder: ./agents-skills
registry_url: https://example.com/registry/
download_timeout_ms: 30000
registry_cache_ttl_ms: 5000
filter_unregistered: false
auto_download: false
";
        let cfg: SkillsConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.skills_folder, "./my-skills");
        assert_eq!(cfg.local_skills_folder, "./local-skills");
        assert_eq!(cfg.agents_folder, "./profiles");
        assert_eq!(cfg.agents_skills_folder, "./agents-skills");
        assert_eq!(cfg.registry_url, "https://example.com/registry/");
        assert_eq!(cfg.download_timeout_ms, 30_000);
        assert_eq!(cfg.registry_cache_ttl_ms, 5_000);
        assert_eq!(cfg.registry_base(), "https://example.com/registry");
        assert!(!cfg.filter_unregistered);
        assert!(!cfg.auto_download);
    }

    #[test]
    fn malformed_yaml_errors() {
        let err = load_config("/no/such/path/for/skills.yaml");
        assert!(err.is_err());
    }

    #[test]
    fn resolved_skills_folder_absolute_passes_through() {
        let cfg = SkillsConfig {
            skills_folder: "/tmp/foo".into(),
            ..SkillsConfig::default()
        };
        assert_eq!(cfg.resolved_skills_folder(), PathBuf::from("/tmp/foo"));
    }

    #[test]
    fn resolved_skills_folder_relative_resolves_against_project() {
        let cfg = SkillsConfig {
            skills_folder: "./bar".into(),
            ..SkillsConfig::default()
        };
        assert_eq!(
            cfg.resolved_skills_folder(),
            iii_worker_paths::project_path("bar")
        );
    }

    #[test]
    fn resolved_skills_folder_tilde_expands_home() {
        let cfg = SkillsConfig {
            skills_folder: "~/.iii/skills".into(),
            ..SkillsConfig::default()
        };
        // dirs::home_dir() must return Some on CI and dev machines.
        // If it doesn't, the warning fallback is exercised instead.
        if let Some(home) = dirs::home_dir() {
            assert_eq!(cfg.resolved_skills_folder(), home.join(".iii/skills"),);
        }
    }

    #[test]
    fn resolved_agents_skills_folder_tilde_expands_home() {
        let cfg = SkillsConfig {
            agents_skills_folder: "~/.agents/skills".into(),
            ..SkillsConfig::default()
        };
        if let Some(home) = dirs::home_dir() {
            assert_eq!(
                cfg.resolved_agents_skills_folder(),
                home.join(".agents/skills")
            );
        }
    }

    #[test]
    fn resolved_agents_folder_relative_resolves_against_project() {
        let cfg = SkillsConfig {
            agents_folder: "./profiles".into(),
            ..SkillsConfig::default()
        };
        assert_eq!(
            cfg.resolved_agents_folder(),
            iii_worker_paths::project_path("profiles")
        );
    }

    #[test]
    fn local_skills_folder_relative_resolves_against_project() {
        let cfg = SkillsConfig {
            local_skills_folder: "./.iii/skills".into(),
            ..SkillsConfig::default()
        };
        assert_eq!(
            cfg.local_skills_folder(),
            iii_worker_paths::project_path(".iii/skills")
        );
    }

    #[test]
    fn registry_base_trims_trailing_slash() {
        let cfg = SkillsConfig {
            registry_url: "https://api.example/".into(),
            ..SkillsConfig::default()
        };
        assert_eq!(cfg.registry_base(), "https://api.example");
    }

    #[test]
    fn json_schema_is_object_with_known_properties() {
        let schema = SkillsConfig::json_schema();
        let props = schema
            .get("properties")
            .and_then(|p| p.as_object())
            .unwrap();
        for field in [
            "skills_folder",
            "local_skills_folder",
            "agents_folder",
            "agents_skills_folder",
            "registry_url",
            "download_timeout_ms",
            "registry_cache_ttl_ms",
            "filter_unregistered",
            "auto_download",
        ] {
            assert!(props.contains_key(field), "schema missing {field}");
        }
        assert!(schema.get("example").is_some());
    }

    #[test]
    fn to_json_from_json_roundtrip() {
        let cfg = SkillsConfig {
            skills_folder: "./my-skills".into(),
            registry_url: "https://example.com/registry".into(),
            download_timeout_ms: 1234,
            registry_cache_ttl_ms: 5678,
            filter_unregistered: false,
            auto_download: false,
            ..SkillsConfig::default()
        };
        let back = SkillsConfig::from_json(&cfg.to_json()).unwrap();
        assert_eq!(back.skills_folder, cfg.skills_folder);
        assert_eq!(back.registry_url, cfg.registry_url);
        assert_eq!(back.download_timeout_ms, cfg.download_timeout_ms);
        assert_eq!(back.registry_cache_ttl_ms, cfg.registry_cache_ttl_ms);
        assert_eq!(back.filter_unregistered, cfg.filter_unregistered);
        assert_eq!(back.auto_download, cfg.auto_download);
    }

    #[test]
    fn from_yaml_matches_from_json_for_seed_shape() {
        let yaml = "skills_folder: ./s\nregistry_url: https://r\ndownload_timeout_ms: 10\n";
        let from_yaml = SkillsConfig::from_yaml(yaml).unwrap();
        let from_json = SkillsConfig::from_json(&from_yaml.to_json()).unwrap();
        assert_eq!(from_yaml.skills_folder, from_json.skills_folder);
        assert_eq!(from_yaml.registry_url, from_json.registry_url);
        assert_eq!(from_yaml.download_timeout_ms, from_json.download_timeout_ms);
    }

    #[test]
    fn topology_equal_when_only_tunables_differ() {
        let base = SkillsConfig::default();
        let tuned = SkillsConfig {
            registry_url: "https://other".into(),
            download_timeout_ms: 1,
            registry_cache_ttl_ms: 2,
            filter_unregistered: !base.filter_unregistered,
            ..base.clone()
        };
        assert_eq!(base.topology(), tuned.topology());
    }

    #[test]
    fn topology_differs_when_a_topology_field_changes() {
        let base = SkillsConfig::default();
        let folder = SkillsConfig {
            skills_folder: "/other".into(),
            ..base.clone()
        };
        let local = SkillsConfig {
            local_skills_folder: "/other-local".into(),
            ..base.clone()
        };
        let agents = SkillsConfig {
            agents_skills_folder: "/other-agents".into(),
            ..base.clone()
        };
        let profiles = SkillsConfig {
            agents_folder: "/other-profiles".into(),
            ..base.clone()
        };
        let auto = SkillsConfig {
            auto_download: !base.auto_download,
            ..base.clone()
        };
        assert_ne!(base.topology(), folder.topology());
        assert_ne!(base.topology(), local.topology());
        assert_ne!(base.topology(), agents.topology());
        assert_ne!(base.topology(), profiles.topology());
        assert_ne!(base.topology(), auto.topology());
    }
}
