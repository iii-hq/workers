//! Operator-facing config. Every field has a `#[serde(default = "…")]`
//! attribute so an empty YAML object still produces a fully-populated
//! struct; `impl Default` mirrors those functions so the binary can fall
//! back to defaults when `--config` is missing or unparseable (see
//! [`binary-worker.md`](../../binary-worker.md) §5).

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoderConfig {
    /// Root directory the worker operates inside. Every wire path is
    /// resolved relative to this.
    #[serde(default = "default_base_path")]
    pub base_path: PathBuf,

    /// Glob patterns matched against the *relative* path. Matching files
    /// can be listed but not read/written/deleted/created.
    #[serde(default)]
    pub non_accessible_globs: Vec<String>,

    #[serde(default = "default_max_read_bytes")]
    pub max_read_bytes: u64,

    #[serde(default = "default_max_write_bytes")]
    pub max_write_bytes: u64,

    #[serde(default = "default_tree_default_depth")]
    pub tree_default_depth: u32,

    #[serde(default = "default_tree_per_folder_limit")]
    pub tree_per_folder_limit: u32,

    #[serde(default = "default_list_default_page_size")]
    pub list_default_page_size: u32,

    #[serde(default = "default_list_max_page_size")]
    pub list_max_page_size: u32,

    #[serde(default = "default_search_max_matches")]
    pub search_default_max_matches: u32,

    #[serde(default = "default_search_max_line_bytes")]
    pub search_default_max_line_bytes: u32,
}

fn default_base_path() -> PathBuf {
    PathBuf::from("./")
}
fn default_max_read_bytes() -> u64 {
    10 * 1024 * 1024
}
fn default_max_write_bytes() -> u64 {
    10 * 1024 * 1024
}
fn default_tree_default_depth() -> u32 {
    4
}
fn default_tree_per_folder_limit() -> u32 {
    50
}
fn default_list_default_page_size() -> u32 {
    100
}
fn default_list_max_page_size() -> u32 {
    1_000
}
fn default_search_max_matches() -> u32 {
    1_000
}
fn default_search_max_line_bytes() -> u32 {
    4_096
}

impl Default for CoderConfig {
    fn default() -> Self {
        Self {
            base_path: default_base_path(),
            non_accessible_globs: Vec::new(),
            max_read_bytes: default_max_read_bytes(),
            max_write_bytes: default_max_write_bytes(),
            tree_default_depth: default_tree_default_depth(),
            tree_per_folder_limit: default_tree_per_folder_limit(),
            list_default_page_size: default_list_default_page_size(),
            list_max_page_size: default_list_max_page_size(),
            search_default_max_matches: default_search_max_matches(),
            search_default_max_line_bytes: default_search_max_line_bytes(),
        }
    }
}

pub fn load_config(path: &str) -> Result<CoderConfig> {
    let content = std::fs::read_to_string(path).with_context(|| format!("read {}", path))?;
    let cfg: CoderConfig =
        serde_yaml::from_str(&content).with_context(|| format!("parse {}", path))?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_yaml_parses_to_defaults() {
        let cfg: CoderConfig = serde_yaml::from_str("{}").expect("empty yaml parses");
        assert_eq!(cfg.base_path, PathBuf::from("./"));
        assert!(cfg.non_accessible_globs.is_empty());
        assert_eq!(cfg.max_read_bytes, 10 * 1024 * 1024);
        assert_eq!(cfg.max_write_bytes, 10 * 1024 * 1024);
        assert_eq!(cfg.tree_default_depth, 4);
        assert_eq!(cfg.tree_per_folder_limit, 50);
        assert_eq!(cfg.list_default_page_size, 100);
        assert_eq!(cfg.list_max_page_size, 1_000);
        assert_eq!(cfg.search_default_max_matches, 1_000);
        assert_eq!(cfg.search_default_max_line_bytes, 4_096);
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let from_yaml: CoderConfig = serde_yaml::from_str("{}").unwrap();
        let from_default = CoderConfig::default();
        // Compare via JSON so we don't have to derive PartialEq.
        let a = serde_json::to_value(&from_yaml).unwrap();
        let b = serde_json::to_value(&from_default).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn custom_yaml_overrides_each_field() {
        let yaml = r#"
base_path: /tmp/c
non_accessible_globs:
  - "**/.env"
max_read_bytes: 42
max_write_bytes: 43
tree_default_depth: 7
tree_per_folder_limit: 9
list_default_page_size: 11
list_max_page_size: 13
search_default_max_matches: 17
search_default_max_line_bytes: 19
"#;
        let cfg: CoderConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.base_path, PathBuf::from("/tmp/c"));
        assert_eq!(cfg.non_accessible_globs, vec!["**/.env".to_string()]);
        assert_eq!(cfg.max_read_bytes, 42);
        assert_eq!(cfg.max_write_bytes, 43);
        assert_eq!(cfg.tree_default_depth, 7);
        assert_eq!(cfg.tree_per_folder_limit, 9);
        assert_eq!(cfg.list_default_page_size, 11);
        assert_eq!(cfg.list_max_page_size, 13);
        assert_eq!(cfg.search_default_max_matches, 17);
        assert_eq!(cfg.search_default_max_line_bytes, 19);
    }

    #[test]
    fn shipped_config_yaml_parses() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.yaml");
        let content = std::fs::read_to_string(path).expect("read config.yaml");
        let cfg: CoderConfig = serde_yaml::from_str(&content).expect("config.yaml parses");
        assert_eq!(cfg.base_path, PathBuf::from("./"));
        assert!(cfg.non_accessible_globs.iter().any(|g| g.contains(".env")));
    }
}
