use std::sync::Arc;

use arc_swap::ArcSwap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

pub type SharedConfig = Arc<ArcSwap<WorkerConfig>>;

/// The `console_url` default released before MOT-5356, seeded as `initial_value` on first boot.
const SEEDED_CONSOLE_URL: &str = "http://127.0.0.1:3113";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WorkerConfig {
    /// Tailscale CLI executable name or absolute path.
    pub tailscale_binary: String,
    /// Local iii Console URL. Empty asks this project's ADE (`console::status`) at share time.
    /// Only loopback HTTP(S) targets are accepted.
    pub console_url: String,
    /// HTTPS port used when a share request does not provide one.
    pub default_https_port: u16,
    /// Permit public Tailscale Funnel shares. False keeps this worker tailnet-only.
    pub allow_funnel: bool,
    /// Maximum time for one Tailscale CLI invocation.
    pub command_timeout_ms: u64,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            tailscale_binary: "tailscale".to_string(),
            console_url: String::new(),
            default_https_port: 443,
            allow_funnel: false,
            command_timeout_ms: 20_000,
        }
    }
}

impl WorkerConfig {
    pub fn from_json(value: &serde_json::Value) -> Result<Self, String> {
        let inner = value.get("tailscale").unwrap_or(value);
        let mut config: Self = serde_json::from_value(inner.clone())
            .map_err(|error| format!("invalid tailscale config: {error}"))?;
        // MOT-5356: `ensure` never replaces a stored value, so existing installs still hold the
        // old seeded default. Read it as unset so share asks console::status: on a single-project
        // machine that resolves to the same 3113; it only stops pointing at another project's ADE.
        let stored = config.console_url.strip_suffix('/');
        if stored.unwrap_or(&config.console_url) == SEEDED_CONSOLE_URL {
            config.console_url.clear();
        }
        config.validate()?;
        Ok(config)
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("WorkerConfig serializes")
    }

    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(WorkerConfig))
            .expect("WorkerConfig schema serializes")
    }

    pub fn into_shared(self) -> SharedConfig {
        Arc::new(ArcSwap::from_pointee(self))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.tailscale_binary.trim().is_empty() {
            return Err("tailscale_binary must not be empty".to_string());
        }
        if self.default_https_port == 0 {
            return Err("default_https_port must be between 1 and 65535".to_string());
        }
        if self.command_timeout_ms == 0 {
            return Err("command_timeout_ms must be greater than zero".to_string());
        }
        if !self.console_url.is_empty() {
            validate_console_url(&self.console_url)?;
        }
        Ok(())
    }
}

/// Validates a loopback Console root and returns its canonical form (`http://127.0.0.1:3114/`).
pub fn validate_console_url(raw: &str) -> Result<String, String> {
    let url =
        Url::parse(raw).map_err(|error| format!("console_url is not a valid URL: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("console_url must use http or https".to_string());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "console_url must include a host".to_string())?;
    if !matches!(host, "127.0.0.1" | "localhost" | "[::1]") {
        return Err("console_url must target localhost or a loopback address".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("console_url must not contain credentials".to_string());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("console_url must not contain a query or fragment".to_string());
    }
    if !matches!(url.path(), "" | "/") {
        return Err("console_url must point at the Console root path".to_string());
    }
    // Spelled out with the port: `Url` drops a default one (:80/:443), and
    // `tailscale serve` refuses a localhost target without a port.
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "console_url must include a port".to_string())?;
    Ok(format!("{}://{host}:{port}/", url.scheme()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe_and_valid() {
        let config = WorkerConfig::default();
        assert!(!config.allow_funnel);
        assert!(config.console_url.is_empty());
        config.validate().unwrap();
    }

    #[test]
    fn canonical_console_url_keeps_a_default_port() {
        assert_eq!(
            validate_console_url("http://127.0.0.1:80").unwrap(),
            "http://127.0.0.1:80/"
        );
        assert_eq!(
            validate_console_url("https://localhost").unwrap(),
            "https://localhost:443/"
        );
    }

    #[test]
    fn rejects_non_loopback_targets_and_credentials() {
        assert!(validate_console_url("https://example.com").is_err());
        assert!(validate_console_url("http://user:pass@127.0.0.1:3113").is_err());
        assert!(validate_console_url("file:///tmp/console").is_err());
        assert!(validate_console_url("http://[::1]:3113").is_ok());
    }

    #[test]
    fn console_url_is_canonical() {
        for raw in [
            "http://127.0.0.1:3114",
            "http://127.1:3114",
            "http://0x7f.0.0.1:3114",
        ] {
            assert_eq!(
                validate_console_url(raw).unwrap(),
                "http://127.0.0.1:3114/",
                "{raw}"
            );
        }
    }

    #[test]
    fn old_seeded_console_url_reads_as_unset() {
        for seeded in ["http://127.0.0.1:3113", "http://127.0.0.1:3113/"] {
            let config = WorkerConfig::from_json(&serde_json::json!({"console_url": seeded}));
            assert!(config.unwrap().console_url.is_empty(), "{seeded}");
        }
        let explicit = serde_json::json!({"console_url": "http://127.0.0.1:3114"});
        let config = WorkerConfig::from_json(&explicit).unwrap();
        assert_eq!(config.console_url, "http://127.0.0.1:3114");
    }

    #[test]
    fn wrapped_configuration_round_trips() {
        let value = serde_json::json!({"tailscale": {"allow_funnel": true}});
        let config = WorkerConfig::from_json(&value).unwrap();
        assert!(config.allow_funnel);
        assert_eq!(config.default_https_port, 443);
    }
}
