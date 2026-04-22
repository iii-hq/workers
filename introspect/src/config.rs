use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct IntrospectConfig {
    #[serde(default = "default_cron")]
    pub cron_topology_refresh: String,
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl_seconds: u64,
}

fn default_cron() -> String {
    "0 */5 * * * *".to_string()
}

fn default_cache_ttl() -> u64 {
    30
}

impl Default for IntrospectConfig {
    fn default() -> Self {
        IntrospectConfig {
            cron_topology_refresh: default_cron(),
            cache_ttl_seconds: default_cache_ttl(),
        }
    }
}

pub fn load_config(path: &str) -> Result<IntrospectConfig> {
    let contents = std::fs::read_to_string(path)?;
    let config: IntrospectConfig = serde_yaml::from_str(&contents)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config: IntrospectConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(config.cron_topology_refresh, "0 */5 * * * *");
        assert_eq!(config.cache_ttl_seconds, 30);
    }

    #[test]
    fn test_config_custom() {
        let yaml = r#"
cron_topology_refresh: "0 */10 * * * *"
cache_ttl_seconds: 60
"#;
        let config: IntrospectConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.cron_topology_refresh, "0 */10 * * * *");
        assert_eq!(config.cache_ttl_seconds, 60);
    }

    #[test]
    fn test_config_default_impl() {
        let config = IntrospectConfig::default();
        assert_eq!(config.cron_topology_refresh, "0 */5 * * * *");
        assert_eq!(config.cache_ttl_seconds, 30);
    }
}
