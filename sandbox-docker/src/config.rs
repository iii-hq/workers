use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct SandboxWorkerConfig {
    #[serde(default = "default_image")]
    pub default_image: String,
    #[serde(default = "default_timeout")]
    pub default_timeout: u64,
    #[serde(default = "default_memory")]
    pub default_memory: u64,
    #[serde(default = "default_cpu")]
    pub default_cpu: f64,
    #[serde(default = "default_max_sandboxes")]
    pub max_sandboxes: usize,
    #[serde(default = "default_max_cmd_timeout")]
    pub max_cmd_timeout: u64,
    #[serde(default = "default_workspace_dir")]
    pub workspace_dir: String,
    #[serde(default)]
    pub pool_size: usize,
}

fn default_image() -> String {
    "python:3.12-slim".to_string()
}

fn default_timeout() -> u64 {
    3600
}

fn default_memory() -> u64 {
    512
}

fn default_cpu() -> f64 {
    1.0
}

fn default_max_sandboxes() -> usize {
    50
}

fn default_max_cmd_timeout() -> u64 {
    300
}

fn default_workspace_dir() -> String {
    "/workspace".to_string()
}

impl Default for SandboxWorkerConfig {
    fn default() -> Self {
        SandboxWorkerConfig {
            default_image: default_image(),
            default_timeout: default_timeout(),
            default_memory: default_memory(),
            default_cpu: default_cpu(),
            max_sandboxes: default_max_sandboxes(),
            max_cmd_timeout: default_max_cmd_timeout(),
            workspace_dir: default_workspace_dir(),
            pool_size: 0,
        }
    }
}

pub fn load_config(path: &str) -> Result<SandboxWorkerConfig> {
    let contents = std::fs::read_to_string(path)?;
    let config: SandboxWorkerConfig = serde_yaml::from_str(&contents)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config: SandboxWorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(config.default_image, "python:3.12-slim");
        assert_eq!(config.default_timeout, 3600);
        assert_eq!(config.default_memory, 512);
        assert!((config.default_cpu - 1.0).abs() < f64::EPSILON);
        assert_eq!(config.max_sandboxes, 50);
        assert_eq!(config.max_cmd_timeout, 300);
        assert_eq!(config.workspace_dir, "/workspace");
        assert_eq!(config.pool_size, 0);
    }

    #[test]
    fn test_config_custom() {
        let yaml = r#"
default_image: node:20-slim
default_timeout: 1800
default_memory: 1024
default_cpu: 2.0
max_sandboxes: 100
max_cmd_timeout: 600
workspace_dir: /home/user
pool_size: 5
"#;
        let config: SandboxWorkerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.default_image, "node:20-slim");
        assert_eq!(config.default_timeout, 1800);
        assert_eq!(config.default_memory, 1024);
        assert!((config.default_cpu - 2.0).abs() < f64::EPSILON);
        assert_eq!(config.max_sandboxes, 100);
        assert_eq!(config.max_cmd_timeout, 600);
        assert_eq!(config.workspace_dir, "/home/user");
        assert_eq!(config.pool_size, 5);
    }

    #[test]
    fn test_sandbox_worker_config_default() {
        let config = SandboxWorkerConfig::default();
        assert_eq!(config.default_image, "python:3.12-slim");
        assert_eq!(config.default_timeout, 3600);
        assert_eq!(config.default_memory, 512);
        assert!((config.default_cpu - 1.0).abs() < f64::EPSILON);
        assert_eq!(config.max_sandboxes, 50);
        assert_eq!(config.max_cmd_timeout, 300);
        assert_eq!(config.workspace_dir, "/workspace");
        assert_eq!(config.pool_size, 0);
    }
}
