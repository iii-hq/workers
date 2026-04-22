use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct CodingConfig {
    #[serde(default = "default_workspace_dir")]
    pub workspace_dir: String,
    #[serde(default = "default_supported_languages")]
    pub supported_languages: Vec<String>,
    #[serde(default = "default_execute_timeout_ms")]
    pub execute_timeout_ms: u64,
    #[serde(default = "default_max_file_size_kb")]
    pub max_file_size_kb: u64,
}

fn default_workspace_dir() -> String {
    "/tmp/iii-coding-workspace".to_string()
}

fn default_supported_languages() -> Vec<String> {
    vec![
        "rust".to_string(),
        "typescript".to_string(),
        "python".to_string(),
    ]
}

fn default_execute_timeout_ms() -> u64 {
    30000
}

fn default_max_file_size_kb() -> u64 {
    256
}

impl Default for CodingConfig {
    fn default() -> Self {
        CodingConfig {
            workspace_dir: default_workspace_dir(),
            supported_languages: default_supported_languages(),
            execute_timeout_ms: default_execute_timeout_ms(),
            max_file_size_kb: default_max_file_size_kb(),
        }
    }
}

pub fn load_config(path: &str) -> Result<CodingConfig> {
    let contents = std::fs::read_to_string(path)?;
    let config: CodingConfig = serde_yaml::from_str(&contents)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config: CodingConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(config.workspace_dir, "/tmp/iii-coding-workspace");
        assert_eq!(config.supported_languages.len(), 3);
        assert_eq!(config.execute_timeout_ms, 30000);
        assert_eq!(config.max_file_size_kb, 256);
    }

    #[test]
    fn test_config_custom() {
        let yaml = r#"
workspace_dir: "/home/user/workspace"
supported_languages: ["rust"]
execute_timeout_ms: 10000
max_file_size_kb: 512
"#;
        let config: CodingConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.workspace_dir, "/home/user/workspace");
        assert_eq!(config.supported_languages, vec!["rust"]);
        assert_eq!(config.execute_timeout_ms, 10000);
        assert_eq!(config.max_file_size_kb, 512);
    }

    #[test]
    fn test_config_default_impl() {
        let config = CodingConfig::default();
        assert_eq!(config.workspace_dir, "/tmp/iii-coding-workspace");
        assert!(config.supported_languages.contains(&"rust".to_string()));
        assert!(config.supported_languages.contains(&"typescript".to_string()));
        assert!(config.supported_languages.contains(&"python".to_string()));
        assert_eq!(config.execute_timeout_ms, 30000);
        assert_eq!(config.max_file_size_kb, 256);
    }
}
