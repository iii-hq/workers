//! Worker runtime config.
//!
//! The skills worker is a filesystem-backed reader plus a
//! `directory::skills::download` function that pulls markdown into the
//! configured `skills_folder`. There is no state-backed registry, no
//! glob arrays, no scopes — everything lives on disk under one root.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Default base URL for the workers registry. Overrideable via
/// `registry_url:` in the config so self-hosted deployments can repoint.
pub const DEFAULT_REGISTRY_URL: &str = "https://api.workers.iii.dev";

/// Default destination for downloaded skills. Resolved relative to the
/// process current working directory.
pub const DEFAULT_SKILLS_FOLDER: &str = "./skills";

fn default_skills_folder() -> String {
    DEFAULT_SKILLS_FOLDER.to_string()
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

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct SkillsConfig {
    /// Folder that backs every read (`iii://`,
    /// `directory::skills::fetch-skill`, `directory::skills::list`,
    /// `directory::prompts::*`) and every write from
    /// `directory::skills::download`. Relative paths are resolved
    /// against the process current working directory; absolute paths
    /// are used as-is.
    #[serde(default = "default_skills_folder")]
    pub skills_folder: String,

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
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            skills_folder: default_skills_folder(),
            registry_url: default_registry_url(),
            download_timeout_ms: default_download_timeout_ms(),
            registry_cache_ttl_ms: default_registry_cache_ttl_ms(),
        }
    }
}

impl SkillsConfig {
    /// Absolute path to the configured skills folder. Relative paths
    /// are resolved against the process current working directory;
    /// absolute paths are returned as-is.
    pub fn resolved_skills_folder(&self) -> PathBuf {
        let candidate = Path::new(&self.skills_folder);
        if candidate.is_absolute() {
            return candidate.to_path_buf();
        }
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(candidate)
    }

    /// Registry base URL with any trailing slash trimmed so callers can
    /// build URLs as `format!("{base}/w/{worker}/skills")`.
    pub fn registry_base(&self) -> &str {
        self.registry_url.trim_end_matches('/')
    }
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
        assert_eq!(cfg.registry_url, DEFAULT_REGISTRY_URL);
        assert_eq!(cfg.download_timeout_ms, 60_000);
        assert_eq!(cfg.registry_cache_ttl_ms, 60_000);
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let from_empty: SkillsConfig = serde_yaml::from_str("{}").unwrap();
        let from_default = SkillsConfig::default();
        assert_eq!(from_empty.skills_folder, from_default.skills_folder);
        assert_eq!(from_empty.registry_url, from_default.registry_url);
        assert_eq!(
            from_empty.download_timeout_ms,
            from_default.download_timeout_ms
        );
        assert_eq!(
            from_empty.registry_cache_ttl_ms,
            from_default.registry_cache_ttl_ms
        );
    }

    #[test]
    fn custom_yaml_overrides_each_field() {
        let yaml = "\
skills_folder: ./my-skills
registry_url: https://example.com/registry/
download_timeout_ms: 30000
registry_cache_ttl_ms: 5000
";
        let cfg: SkillsConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.skills_folder, "./my-skills");
        assert_eq!(cfg.registry_url, "https://example.com/registry/");
        assert_eq!(cfg.download_timeout_ms, 30_000);
        assert_eq!(cfg.registry_cache_ttl_ms, 5_000);
        assert_eq!(cfg.registry_base(), "https://example.com/registry");
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
    fn registry_base_trims_trailing_slash() {
        let cfg = SkillsConfig {
            registry_url: "https://api.example/".into(),
            ..SkillsConfig::default()
        };
        assert_eq!(cfg.registry_base(), "https://api.example");
    }
}
