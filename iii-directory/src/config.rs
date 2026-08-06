//! Worker runtime config.
//!
//! The skills worker is a filesystem-backed reader plus a
//! `directory::skills::download` function that pulls markdown into the
//! configured `skills_folder`. There is no state-backed registry, no
//! glob arrays, no scopes — everything lives on disk under one root.

use std::path::{Path, PathBuf};
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

/// Default destination for downloaded (global) skills. Uses the `~`
/// prefix so it expands to the user's home directory at runtime via
/// `dirs::home_dir()`.
pub const DEFAULT_SKILLS_FOLDER: &str = "~/.iii/skills";

/// Default destination for local (project-scoped) skill overrides.
/// Resolved relative to the process current working directory.
pub const DEFAULT_LOCAL_SKILLS_FOLDER: &str = "./.iii/skills";

fn default_skills_folder() -> String {
    DEFAULT_SKILLS_FOLDER.to_string()
}

fn default_local_skills_folder() -> String {
    DEFAULT_LOCAL_SKILLS_FOLDER.to_string()
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

/// Default root for user-authored prompts (the prompt library).
pub const DEFAULT_PROMPTS_FOLDER: &str = "~/.iii/prompts";

/// Default root for project-local prompt overrides.
pub const DEFAULT_LOCAL_PROMPTS_FOLDER: &str = "./.iii/prompts";

fn default_prompts_folder() -> String {
    DEFAULT_PROMPTS_FOLDER.to_string()
}

fn default_local_prompts_folder() -> String {
    DEFAULT_LOCAL_PROMPTS_FOLDER.to_string()
}

#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
pub struct SkillsConfig {
    /// Folder that backs every read (`directory::skills::list`,
    /// `directory::skills::get`, `directory::prompts::*`) and every
    /// write from `directory::skills::download`. Supports three forms:
    ///
    /// - Absolute path — used as-is.
    /// - `~`-prefixed — expands leading `~` via `dirs::home_dir()`.
    /// - Relative — resolved against the process current working directory.
    #[serde(default = "default_skills_folder")]
    pub skills_folder: String,

    /// Folder for local (project-scoped) skill overrides. A namespace
    /// directory present under this root shadows the same namespace in
    /// the global `skills_folder` entirely (whole-namespace override).
    /// Supports the same three resolution forms as `skills_folder`.
    #[serde(default = "default_local_skills_folder")]
    pub local_skills_folder: String,

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

    /// Folder for user-authored prompts (the prompt library): flat
    /// `<name>.md` files with `description` (and optional `kind`)
    /// frontmatter. `directory::prompts::save` writes here; entries
    /// appear in `directory::prompts::list` with `source: user`.
    /// Supports the same three resolution forms as `skills_folder`.
    #[serde(default = "default_prompts_folder")]
    pub prompts_folder: String,

    /// Folder for project-local prompt overrides. An entry here shadows
    /// a same-named entry in `prompts_folder`. Supports the same three
    /// resolution forms as `skills_folder`.
    #[serde(default = "default_local_prompts_folder")]
    pub local_prompts_folder: String,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            skills_folder: default_skills_folder(),
            local_skills_folder: default_local_skills_folder(),
            registry_url: default_registry_url(),
            download_timeout_ms: default_download_timeout_ms(),
            registry_cache_ttl_ms: default_registry_cache_ttl_ms(),
            filter_unregistered: default_filter_unregistered(),
            auto_download: default_auto_download(),
            prompts_folder: default_prompts_folder(),
            local_prompts_folder: default_local_prompts_folder(),
        }
    }
}

/// Resolve a path string supporting three forms:
///
/// - `~`-prefixed: expand leading `~` via `dirs::home_dir()`.
///   Falls back to CWD-relative if `home_dir()` is `None`.
/// - Absolute: returned as-is.
/// - Relative: resolved against the process current working directory.
fn resolve_path(raw: &str) -> PathBuf {
    if let Some(remainder) = raw.strip_prefix('~') {
        let tail = remainder.strip_prefix('/').unwrap_or(remainder);
        match dirs::home_dir() {
            Some(home) => {
                if tail.is_empty() {
                    home
                } else {
                    home.join(tail)
                }
            }
            None => {
                tracing::warn!(
                    path = %raw,
                    "dirs::home_dir() returned None; treating '~' path as CWD-relative"
                );
                let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                cwd.join(raw)
            }
        }
    } else {
        let candidate = Path::new(raw);
        if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(candidate)
        }
    }
}

impl SkillsConfig {
    /// Absolute path to the configured global skills folder.
    pub fn resolved_skills_folder(&self) -> PathBuf {
        resolve_path(&self.skills_folder)
    }

    /// Absolute path to the configured local skills folder.
    pub fn local_skills_folder(&self) -> PathBuf {
        resolve_path(&self.local_skills_folder)
    }

    /// Absolute path to the configured user prompt library folder.
    pub fn resolved_prompts_folder(&self) -> PathBuf {
        resolve_path(&self.prompts_folder)
    }

    /// Absolute path to the configured project-local prompt folder.
    pub fn resolved_local_prompts_folder(&self) -> PathBuf {
        resolve_path(&self.local_prompts_folder)
    }

    /// Registry base URL with any trailing slash trimmed so callers can
    /// build URLs as `format!("{base}/w/{worker}/skills")`.
    pub fn registry_base(&self) -> &str {
        self.registry_url.trim_end_matches('/')
    }

    /// Restart-requiring fields. A `configuration:updated` reload that
    /// changes any of these is refused (logged "restart required"):
    /// `skills_folder` / `local_skills_folder` are the on-disk read/write
    /// roots baked into running tasks, and `auto_download` wires the
    /// `worker`-trigger subscription + boot reconcile at startup — none can
    /// be re-wired safely in place.
    pub fn topology(&self) -> Topology {
        Topology {
            skills_folder: self.skills_folder.clone(),
            local_skills_folder: self.local_skills_folder.clone(),
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
        assert_eq!(cfg.skills_folder, DEFAULT_SKILLS_FOLDER);
        assert_eq!(cfg.local_skills_folder, DEFAULT_LOCAL_SKILLS_FOLDER);
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
registry_url: https://example.com/registry/
download_timeout_ms: 30000
registry_cache_ttl_ms: 5000
filter_unregistered: false
auto_download: false
";
        let cfg: SkillsConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.skills_folder, "./my-skills");
        assert_eq!(cfg.local_skills_folder, "./local-skills");
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
    fn resolved_skills_folder_relative_resolves_against_cwd() {
        let cfg = SkillsConfig {
            skills_folder: "./bar".into(),
            ..SkillsConfig::default()
        };
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(cfg.resolved_skills_folder(), cwd.join("bar"));
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
    fn local_skills_folder_relative_resolves_against_cwd() {
        let cfg = SkillsConfig {
            local_skills_folder: "./.iii/skills".into(),
            ..SkillsConfig::default()
        };
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(cfg.local_skills_folder(), cwd.join(".iii/skills"));
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
        let auto = SkillsConfig {
            auto_download: !base.auto_download,
            ..base.clone()
        };
        assert_ne!(base.topology(), folder.topology());
        assert_ne!(base.topology(), local.topology());
        assert_ne!(base.topology(), auto.topology());
    }
}
