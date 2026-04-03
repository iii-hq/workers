use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct SandboxConfig {
    #[serde(default = "default_vmstate_path")]
    pub vmstate_path: String,
    #[serde(default = "default_memfile_path")]
    pub memfile_path: String,
    #[serde(default = "default_mem_size_mb")]
    pub mem_size_mb: usize,
    #[serde(default = "default_timeout")]
    pub default_timeout: u64,
    #[serde(default = "default_max_sandboxes")]
    pub max_sandboxes: usize,
    #[serde(default = "default_max_cmd_timeout")]
    pub max_cmd_timeout: u64,
    #[serde(default = "default_max_output_bytes")]
    pub max_output_bytes: usize,
    #[serde(default = "default_language")]
    pub default_language: String,
}

fn default_vmstate_path() -> String {
    "./template/vmstate".to_string()
}

fn default_memfile_path() -> String {
    "./template/mem".to_string()
}

fn default_mem_size_mb() -> usize {
    256
}

fn default_timeout() -> u64 {
    30
}

fn default_max_sandboxes() -> usize {
    1000
}

fn default_max_cmd_timeout() -> u64 {
    60
}

fn default_max_output_bytes() -> usize {
    1_048_576
}

fn default_language() -> String {
    "python".to_string()
}

impl Default for SandboxConfig {
    fn default() -> Self {
        SandboxConfig {
            vmstate_path: default_vmstate_path(),
            memfile_path: default_memfile_path(),
            mem_size_mb: default_mem_size_mb(),
            default_timeout: default_timeout(),
            max_sandboxes: default_max_sandboxes(),
            max_cmd_timeout: default_max_cmd_timeout(),
            max_output_bytes: default_max_output_bytes(),
            default_language: default_language(),
        }
    }
}

pub fn load_config(path: &str) -> Result<SandboxConfig> {
    let contents = std::fs::read_to_string(path)?;
    let config: SandboxConfig = serde_yaml::from_str(&contents)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config: SandboxConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(config.vmstate_path, "./template/vmstate");
        assert_eq!(config.memfile_path, "./template/mem");
        assert_eq!(config.mem_size_mb, 256);
        assert_eq!(config.default_timeout, 30);
        assert_eq!(config.max_sandboxes, 1000);
        assert_eq!(config.max_cmd_timeout, 60);
        assert_eq!(config.max_output_bytes, 1_048_576);
        assert_eq!(config.default_language, "python");
    }

    #[test]
    fn test_config_custom() {
        let yaml = r#"
vmstate_path: /opt/vm/state
memfile_path: /opt/vm/mem
mem_size_mb: 512
default_timeout: 60
max_sandboxes: 500
max_cmd_timeout: 120
max_output_bytes: 2097152
default_language: node
"#;
        let config: SandboxConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.vmstate_path, "/opt/vm/state");
        assert_eq!(config.memfile_path, "/opt/vm/mem");
        assert_eq!(config.mem_size_mb, 512);
        assert_eq!(config.default_timeout, 60);
        assert_eq!(config.max_sandboxes, 500);
        assert_eq!(config.max_cmd_timeout, 120);
        assert_eq!(config.max_output_bytes, 2_097_152);
        assert_eq!(config.default_language, "node");
    }

    #[test]
    fn test_sandbox_config_default() {
        let config = SandboxConfig::default();
        assert_eq!(config.vmstate_path, "./template/vmstate");
        assert_eq!(config.memfile_path, "./template/mem");
        assert_eq!(config.mem_size_mb, 256);
        assert_eq!(config.default_timeout, 30);
        assert_eq!(config.max_sandboxes, 1000);
        assert_eq!(config.max_cmd_timeout, 60);
        assert_eq!(config.max_output_bytes, 1_048_576);
        assert_eq!(config.default_language, "python");
    }
}
