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
    #[serde(default = "default_environment")]
    pub environment: String,
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
    #[serde(default = "default_registration_admin_token_env")]
    pub registration_admin_token_env: String,
    #[serde(default = "default_state_timeout_ms")]
    pub state_timeout_ms: u64,
    #[serde(default = "default_skills_timeout_ms")]
    pub skills_register_timeout_ms: u64,
    #[serde(default = "default_skills_timeout_ms")]
    pub skills_unregister_timeout_ms: u64,
}

fn default_engine_url() -> String {
    "ws://127.0.0.1:49134".to_string()
}

fn default_environment() -> String {
    "local".to_string()
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
    "0 0 3 * * *".to_string()
}

fn default_skills_timeout_ms() -> u64 {
    5_000
}

fn default_default_scopes() -> Vec<String> {
    vec!["mcp:tools".to_string()]
}

fn default_supported_scopes() -> Vec<String> {
    vec!["mcp:tools".to_string(), "a2a:message".to_string()]
}

fn default_token_endpoint_auth_methods_supported() -> Vec<String> {
    vec![
        "client_secret_post".to_string(),
        "client_secret_basic".to_string(),
    ]
}

fn default_registration_admin_token_env() -> String {
    "III_AUTH_REGISTRATION_TOKEN".to_string()
}

fn default_state_timeout_ms() -> u64 {
    5_000
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            environment: default_environment(),
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
            registration_admin_token_env: default_registration_admin_token_env(),
            state_timeout_ms: default_state_timeout_ms(),
            skills_register_timeout_ms: default_skills_timeout_ms(),
            skills_unregister_timeout_ms: default_skills_timeout_ms(),
        }
    }
}

pub fn load_config(path: &str) -> Result<AuthConfig> {
    let contents = std::fs::read_to_string(path)?;
    let mut cfg: AuthConfig = serde_yaml::from_str(&contents)?;
    if let Ok(environment) = std::env::var("III_AUTH_ENV") {
        if !environment.is_empty() {
            cfg.environment = environment;
        }
    }
    validate_config(&cfg)?;
    Ok(cfg)
}

pub fn validate_config(cfg: &AuthConfig) -> Result<()> {
    if cfg.access_token_ttl_seconds <= 0 {
        anyhow::bail!("access_token_ttl_seconds must be positive");
    }
    if cfg.refresh_token_ttl_seconds <= 0 {
        anyhow::bail!("refresh_token_ttl_seconds must be positive");
    }
    if cfg.rotation_overlap_seconds <= 0 {
        anyhow::bail!("rotation_overlap_seconds must be positive");
    }
    if cfg.state_timeout_ms == 0 {
        anyhow::bail!("state_timeout_ms must be positive");
    }
    if cfg.skills_register_timeout_ms == 0 {
        anyhow::bail!("skills_register_timeout_ms must be positive");
    }
    if cfg.skills_unregister_timeout_ms == 0 {
        anyhow::bail!("skills_unregister_timeout_ms must be positive");
    }
    if cfg.supported_scopes.is_empty() {
        anyhow::bail!("supported_scopes must not be empty");
    }
    for scope in &cfg.default_scopes {
        if !scope_supported_by(scope, &cfg.supported_scopes) {
            anyhow::bail!("default scope {scope} must be listed in supported_scopes");
        }
    }
    if cfg.environment.eq_ignore_ascii_case("production") {
        if cfg.engine_url.starts_with("ws://") {
            anyhow::bail!("production auth config requires wss:// engine_url");
        }
        if cfg.issuer.starts_with("http://") {
            anyhow::bail!("production auth config requires https:// issuer");
        }
    }
    let cron_fields = cfg.rotation_cron.split_whitespace().count();
    if cron_fields != 6 {
        anyhow::bail!("rotation_cron must use iii's 6-field cron format");
    }
    Ok(())
}

fn scope_supported_by(scope: &str, supported_scopes: &[String]) -> bool {
    supported_scopes.iter().any(|supported| {
        supported == scope
            || (supported == "function:*" && scope.starts_with("function:"))
            || (supported == "trigger:*" && scope.starts_with("trigger:"))
    })
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
        assert_eq!(cfg.environment, "local");
        assert_eq!(cfg.store, StoreBackend::IiiState);
        assert!(cfg.supported_scopes.contains(&"mcp:tools".to_string()));
        assert!(!cfg.supported_scopes.contains(&"function:*".to_string()));
        assert_eq!(
            cfg.registration_admin_token_env,
            "III_AUTH_REGISTRATION_TOKEN"
        );
        assert_eq!(cfg.state_timeout_ms, 5_000);
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

    #[test]
    fn production_rejects_insecure_urls() {
        let cfg = AuthConfig {
            environment: "production".to_string(),
            ..AuthConfig::default()
        };
        let err = validate_config(&cfg).unwrap_err();
        assert!(err.to_string().contains("wss:// engine_url"));
    }

    #[test]
    fn cron_must_be_six_fields() {
        let cfg = AuthConfig {
            rotation_cron: "0 0 3 * * * *".to_string(),
            ..AuthConfig::default()
        };
        let err = validate_config(&cfg).unwrap_err();
        assert!(err.to_string().contains("6-field cron"));
    }

    #[test]
    fn ttls_must_be_positive() {
        let cfg = AuthConfig {
            access_token_ttl_seconds: 0,
            ..AuthConfig::default()
        };
        let err = validate_config(&cfg).unwrap_err();
        assert!(err.to_string().contains("access_token_ttl_seconds"));
    }

    #[test]
    fn default_scopes_must_be_supported() {
        let cfg = AuthConfig {
            default_scopes: vec!["function:demo::read".to_string()],
            supported_scopes: vec!["mcp:tools".to_string()],
            ..AuthConfig::default()
        };
        let err = validate_config(&cfg).unwrap_err();
        assert!(err.to_string().contains("default scope"));
    }

    #[test]
    fn wildcard_supported_scopes_cover_default_scopes() {
        let cfg = AuthConfig {
            default_scopes: vec!["function:demo::read".to_string()],
            supported_scopes: vec!["function:*".to_string()],
            ..AuthConfig::default()
        };
        validate_config(&cfg).unwrap();
    }
}
