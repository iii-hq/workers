//! Worker configuration — field parity with the builtin's BridgeClientConfig
//! (engine/src/workers/bridge_client/mod.rs:22-48), plus the same URL
//! fallback chain (config.url -> III_URL env -> ws://0.0.0.0:49134).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_REMOTE_URL: &str = "ws://0.0.0.0:49134";

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BridgeConfig {
    /// Remote engine WebSocket URL. Fallback: `III_URL` env var, then ws://0.0.0.0:49134.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Local functions registered ON the remote engine (remote -> local calls).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expose: Vec<ExposeEntry>,
    /// Local function names that proxy to remote functions (local -> remote calls).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forward: Vec<ForwardEntry>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExposeEntry {
    pub local_function: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_function: Option<String>,
}

impl ExposeEntry {
    /// The name registered on the remote engine — defaults to the local name
    /// (builtin parity, mod.rs:245-248).
    pub fn remote_name(&self) -> &str {
        self.remote_function.as_deref().unwrap_or(&self.local_function)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ForwardEntry {
    pub local_function: String,
    pub remote_function: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

impl BridgeConfig {
    pub fn effective_url(&self) -> String {
        self.effective_url_with(std::env::var("III_URL").ok())
    }

    /// Pure variant for tests — `env_url` stands in for `III_URL`.
    pub fn effective_url_with(&self, env_url: Option<String>) -> String {
        self.url
            .clone()
            .or(env_url)
            .unwrap_or_else(|| DEFAULT_REMOTE_URL.to_string())
    }

    pub fn normalized(&self) -> Self {
        self.clone()
    }

    pub fn json_schema() -> Value {
        serde_json::to_value(schemars::schema_for!(BridgeConfig)).unwrap_or(Value::Null)
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn from_json(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone())
            .map_err(|e| format!("invalid bridge configuration: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_builtin_shaped_config() {
        let c: BridgeConfig = serde_yaml::from_str(
            "{url: 'ws://remote:49134', expose: [{local_function: a.b}], forward: [{local_function: f.local, remote_function: f.remote, timeout_ms: 5000}]}",
        )
        .unwrap();
        assert_eq!(c.url.as_deref(), Some("ws://remote:49134"));
        assert_eq!(c.expose[0].local_function, "a.b");
        assert_eq!(c.expose[0].remote_name(), "a.b", "expose name defaults to local_function");
        assert_eq!(c.forward[0].remote_function, "f.remote");
        assert_eq!(c.forward[0].timeout_ms, Some(5000));
    }

    #[test]
    fn expose_remote_name_prefers_remote_function() {
        let e: ExposeEntry =
            serde_yaml::from_str("{local_function: a.b, remote_function: c.d}").unwrap();
        assert_eq!(e.remote_name(), "c.d");
    }

    #[test]
    fn url_fallback_chain_matches_builtin() {
        let with_url: BridgeConfig = serde_yaml::from_str("{url: 'ws://cfg:1'}").unwrap();
        let without: BridgeConfig = BridgeConfig::default();
        assert_eq!(with_url.effective_url_with(Some("ws://env:2".into())), "ws://cfg:1");
        assert_eq!(without.effective_url_with(Some("ws://env:2".into())), "ws://env:2");
        assert_eq!(without.effective_url_with(None), "ws://0.0.0.0:49134");
    }

    #[test]
    fn rejects_unknown_fields() {
        let r: Result<BridgeConfig, _> = serde_yaml::from_str("{url: x, wat: 1}");
        assert!(r.is_err(), "deny_unknown_fields must reject 'wat'");
    }

    #[test]
    fn json_roundtrip() {
        let c: BridgeConfig =
            serde_yaml::from_str("{forward: [{local_function: a, remote_function: b}]}").unwrap();
        let back = BridgeConfig::from_json(&c.to_json()).unwrap();
        assert_eq!(back, c);
    }
}
