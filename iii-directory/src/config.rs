//! Worker runtime config.
//!
//! The skills worker is a filesystem-backed reader plus a `skills::download`
//! function that pulls markdown into the configured `skills_folder`. There
//! is no state-backed registry, no glob arrays, no scopes — everything
//! lives on disk under one root.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Default base URL for the workers registry. Overrideable via
/// `registry_url:` in the config so self-hosted deployments can repoint.
pub const DEFAULT_REGISTRY_URL: &str = "https://api.workers.iii.dev";

/// Default destination for downloaded skills. Resolved relative to the
/// directory of the loaded config file (or CWD if the config has no
/// parent).
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
    /// Folder that backs every read (`iii://`, `skill::fetch`, `skills::list`,
    /// `prompts::*`) and every write from `skills::download`. Resolved
    /// relative to the directory of the loaded config file.
    #[serde(default = "default_skills_folder")]
    pub skills_folder: String,

    /// Workers registry base URL — used by `skills::download` and the
    /// `registry::*` proxies when a `worker=` source is specified.
    /// Stored without a trailing slash.
    #[serde(default = "default_registry_url")]
    pub registry_url: String,

    /// Timeout for a single download operation (HTTP request OR `git clone`)
    /// in milliseconds. Also used as the request timeout for `registry::*`
    /// HTTP calls.
    #[serde(default = "default_download_timeout_ms")]
    pub download_timeout_ms: u64,

    /// TTL (in milliseconds) for cached `registry::worker-list` and
    /// `registry::worker-info` responses. Repeat lookups within this
    /// window skip the HTTP call. Set to 0 to disable caching.
    #[serde(default = "default_registry_cache_ttl_ms")]
    pub registry_cache_ttl_ms: u64,

    /// Directory the `skills_folder` is resolved against. Set to the
    /// parent of the loaded config path; falls back to CWD when no
    /// config file is read or the path has no parent. Skipped from
    /// (de)serialization so config files don't have to declare it.
    #[serde(skip)]
    pub config_dir: Option<PathBuf>,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            skills_folder: default_skills_folder(),
            registry_url: default_registry_url(),
            download_timeout_ms: default_download_timeout_ms(),
            registry_cache_ttl_ms: default_registry_cache_ttl_ms(),
            config_dir: None,
        }
    }
}

impl SkillsConfig {
    /// Absolute path to the configured skills folder. Relative paths are
    /// resolved against the config file's directory; absolute paths are
    /// returned as-is.
    pub fn resolved_skills_folder(&self) -> PathBuf {
        let candidate = Path::new(&self.skills_folder);
        if candidate.is_absolute() {
            return candidate.to_path_buf();
        }
        if let Some(dir) = &self.config_dir {
            return dir.join(candidate);
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
    let mut cfg: SkillsConfig = serde_yaml::from_str(&contents)?;
    cfg.config_dir = parent_dir(Path::new(path));
    Ok(cfg)
}

fn parent_dir(path: &Path) -> Option<PathBuf> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    abs.parent().map(|p| p.to_path_buf())
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
        assert!(cfg.config_dir.is_none());
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
    fn load_config_records_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "skills_folder: ./my-skills\n").unwrap();
        let cfg = load_config(path.to_str().unwrap()).unwrap();
        assert_eq!(cfg.config_dir.as_deref(), Some(dir.path()));
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
    fn resolved_skills_folder_relative_resolves_against_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = SkillsConfig {
            skills_folder: "./bar".into(),
            config_dir: Some(dir.path().to_path_buf()),
            ..SkillsConfig::default()
        };
        assert_eq!(cfg.resolved_skills_folder(), dir.path().join("bar"));
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
