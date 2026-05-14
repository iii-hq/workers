use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StoreBackend {
    #[default]
    IiiState,
    Memory,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct AuthConfig {
    #[serde(default = "default_engine_url")]
    pub engine_url: String,
    #[serde(default = "default_issuer")]
    pub issuer: String,
    #[serde(default = "default_audience")]
    pub audience: String,
    #[serde(default = "default_idp_mode")]
    pub idp_mode: String,
    #[serde(default)]
    pub store: StoreBackend,
    #[serde(default = "default_access_token_ttl_seconds")]
    pub access_token_ttl_seconds: i64,
    #[serde(default = "default_refresh_token_ttl_seconds")]
    pub refresh_token_ttl_seconds: i64,
    #[serde(default = "default_rotation_overlap_seconds")]
    pub rotation_overlap_seconds: i64,
    #[serde(default = "default_rotation_cron")]
    pub rotation_cron: String,
    #[serde(default = "default_default_scopes")]
    pub default_scopes: Vec<String>,
    #[serde(default = "default_supported_scopes")]
    pub supported_scopes: Vec<String>,
    #[serde(default = "default_token_endpoint_auth_methods_supported")]
    pub token_endpoint_auth_methods_supported: Vec<String>,
    #[serde(default = "default_skills_timeout_ms")]
    pub skills_register_timeout_ms: u64,
    #[serde(default = "default_skills_timeout_ms")]
    pub skills_unregister_timeout_ms: u64,
}

fn default_engine_url() -> String {
    "ws://127.0.0.1:49134".to_string()
}

fn default_issuer() -> String {
    "http://127.0.0.1:3111".to_string()
}

fn default_audience() -> String {
    "iii".to_string()
}

fn default_idp_mode() -> String {
    "local".to_string()
}

fn default_access_token_ttl_seconds() -> i64 {
    900
}

fn default_refresh_token_ttl_seconds() -> i64 {
    2_592_000
}

fn default_rotation_overlap_seconds() -> i64 {
    86_400
}

fn default_rotation_cron() -> String {
    "0 0 3 * * * *".to_string()
}

fn default_skills_timeout_ms() -> u64 {
    5_000
}

fn default_default_scopes() -> Vec<String> {
    vec!["mcp:tools".to_string()]
}

fn default_supported_scopes() -> Vec<String> {
    vec![
        "mcp:tools".to_string(),
        "a2a:message".to_string(),
        "function:*".to_string(),
        "iii:function_registration".to_string(),
        "iii:trigger_type_registration".to_string(),
        "iii:trusted_internal".to_string(),
    ]
}

fn default_token_endpoint_auth_methods_supported() -> Vec<String> {
    vec!["client_secret_post".to_string(), "none".to_string()]
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            engine_url: default_engine_url(),
            issuer: default_issuer(),
            audience: default_audience(),
            idp_mode: default_idp_mode(),
            store: StoreBackend::default(),
            access_token_ttl_seconds: default_access_token_ttl_seconds(),
            refresh_token_ttl_seconds: default_refresh_token_ttl_seconds(),
            rotation_overlap_seconds: default_rotation_overlap_seconds(),
            rotation_cron: default_rotation_cron(),
            default_scopes: default_default_scopes(),
            supported_scopes: default_supported_scopes(),
            token_endpoint_auth_methods_supported: default_token_endpoint_auth_methods_supported(),
            skills_register_timeout_ms: default_skills_timeout_ms(),
            skills_unregister_timeout_ms: default_skills_timeout_ms(),
        }
    }
}

pub fn load_config(path: &str) -> Result<AuthConfig> {
    let contents = std::fs::read_to_string(path)?;
    let cfg: AuthConfig = serde_yaml::from_str(&contents)?;
    Ok(cfg)
}

pub fn resolve_store_backend(cfg: &AuthConfig) -> StoreBackend {
    match std::env::var("III_AUTH_STORE").as_deref() {
        Ok("memory") => StoreBackend::Memory,
        Ok("iii_state") => StoreBackend::IiiState,
        Ok(other) if !other.is_empty() => {
            tracing::warn!(%other, "unknown III_AUTH_STORE, using configured store");
            cfg.store
        }
        _ => cfg.store,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: AuthConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.engine_url, "ws://127.0.0.1:49134");
        assert_eq!(cfg.issuer, "http://127.0.0.1:3111");
        assert_eq!(cfg.store, StoreBackend::IiiState);
        assert!(cfg.supported_scopes.contains(&"mcp:tools".to_string()));
    }

    #[test]
    fn custom_yaml_overrides() {
        let cfg: AuthConfig = serde_yaml::from_str(
            r#"engine_url: "ws://example:49134"
issuer: "https://auth.example"
audience: "workers"
store: memory
default_scopes: ["function:demo::read"]
"#,
        )
        .unwrap();
        assert_eq!(cfg.engine_url, "ws://example:49134");
        assert_eq!(cfg.issuer, "https://auth.example");
        assert_eq!(cfg.audience, "workers");
        assert_eq!(cfg.store, StoreBackend::Memory);
        assert_eq!(cfg.default_scopes, vec!["function:demo::read"]);
    }
}
