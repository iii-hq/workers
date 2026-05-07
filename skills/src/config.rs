//! Worker runtime config. Mirrors iii-graph / iii-fts.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct SkillsConfig {
    #[serde(default)]
    pub scopes: ScopesConfig,
    #[serde(default = "default_state_timeout_ms")]
    pub state_timeout_ms: u64,
    /// Glob patterns for filesystem-backed skills. Resolved relative to
    /// the directory of the config file. Each match becomes a skill
    /// keyed by its path relative to the glob's static prefix, with
    /// `.md` stripped.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Glob patterns for filesystem-backed prompts. Each match must
    /// have YAML frontmatter declaring at least `description`.
    #[serde(default)]
    pub prompts: Vec<String>,
    /// Directory that filesystem globs are resolved against. Set to the
    /// parent of the loaded config path; falls back to CWD when no
    /// config file is read or the path has no parent. Skipped from
    /// (de)serialization so config files don't have to declare it.
    #[serde(skip)]
    pub config_dir: Option<PathBuf>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ScopesConfig {
    #[serde(default = "default_skills_scope")]
    pub skills: String,
    #[serde(default = "default_prompts_scope")]
    pub prompts: String,
}

fn default_skills_scope() -> String {
    "skills".to_string()
}

fn default_prompts_scope() -> String {
    "prompts".to_string()
}

fn default_state_timeout_ms() -> u64 {
    10_000
}

impl Default for ScopesConfig {
    fn default() -> Self {
        Self {
            skills: default_skills_scope(),
            prompts: default_prompts_scope(),
        }
    }
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            scopes: ScopesConfig::default(),
            state_timeout_ms: default_state_timeout_ms(),
            skills: Vec::new(),
            prompts: Vec::new(),
            config_dir: None,
        }
    }
}

impl SkillsConfig {
    /// Directory used to resolve relative globs. Falls back to CWD
    /// (or `.` if even CWD is unavailable) so `expand_*` callers can
    /// always assume a base path.
    pub fn glob_base_dir(&self) -> PathBuf {
        if let Some(dir) = &self.config_dir {
            return dir.clone();
        }
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
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
        assert_eq!(cfg.scopes.skills, "skills");
        assert_eq!(cfg.scopes.prompts, "prompts");
        assert_eq!(cfg.state_timeout_ms, 10_000);
        assert!(cfg.skills.is_empty());
        assert!(cfg.prompts.is_empty());
        assert!(cfg.config_dir.is_none());
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let from_empty: SkillsConfig = serde_yaml::from_str("{}").unwrap();
        let from_default = SkillsConfig::default();
        assert_eq!(from_empty.scopes.skills, from_default.scopes.skills);
        assert_eq!(from_empty.scopes.prompts, from_default.scopes.prompts);
        assert_eq!(from_empty.state_timeout_ms, from_default.state_timeout_ms);
        assert_eq!(from_empty.skills, from_default.skills);
        assert_eq!(from_empty.prompts, from_default.prompts);
    }

    #[test]
    fn custom_yaml_overrides_each_field() {
        let yaml = "\
scopes:
  skills: my-skills
  prompts: my-prompts
state_timeout_ms: 7500
skills:
  - my-folder/**/*.md
prompts:
  - prompts-folder/*.md
";
        let cfg: SkillsConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.scopes.skills, "my-skills");
        assert_eq!(cfg.scopes.prompts, "my-prompts");
        assert_eq!(cfg.state_timeout_ms, 7_500);
        assert_eq!(cfg.skills, vec!["my-folder/**/*.md".to_string()]);
        assert_eq!(cfg.prompts, vec!["prompts-folder/*.md".to_string()]);
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
        std::fs::write(&path, "scopes:\n  skills: x\n  prompts: y\n").unwrap();
        let cfg = load_config(path.to_str().unwrap()).unwrap();
        assert_eq!(cfg.config_dir.as_deref(), Some(dir.path()));
    }

    #[test]
    fn glob_base_dir_falls_back_to_cwd() {
        let cfg = SkillsConfig::default();
        let base = cfg.glob_base_dir();
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(base, cwd);
    }
}
