//! Worker runtime config. Mirrors iii-graph / iii-fts.

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct SkillsConfig {
    #[serde(default)]
    pub scopes: ScopesConfig,
    #[serde(default = "default_state_timeout_ms")]
    pub state_timeout_ms: u64,
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
        }
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
        assert_eq!(cfg.scopes.skills, "skills");
        assert_eq!(cfg.scopes.prompts, "prompts");
        assert_eq!(cfg.state_timeout_ms, 10_000);
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let from_empty: SkillsConfig = serde_yaml::from_str("{}").unwrap();
        let from_default = SkillsConfig::default();
        assert_eq!(from_empty.scopes.skills, from_default.scopes.skills);
        assert_eq!(from_empty.scopes.prompts, from_default.scopes.prompts);
        assert_eq!(from_empty.state_timeout_ms, from_default.state_timeout_ms);
    }

    #[test]
    fn custom_yaml_overrides_each_field() {
        let yaml = "\
scopes:
  skills: my-skills
  prompts: my-prompts
state_timeout_ms: 7500
";
        let cfg: SkillsConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.scopes.skills, "my-skills");
        assert_eq!(cfg.scopes.prompts, "my-prompts");
        assert_eq!(cfg.state_timeout_ms, 7_500);
    }

    #[test]
    fn malformed_yaml_errors() {
        let err = load_config("/no/such/path/for/skills.yaml");
        assert!(err.is_err());
    }
}
