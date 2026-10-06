//! Request and response shapes of every `secrets::*` function.
//!
//! Only two types carry a value: [`SetRequest`] (in) and [`ResolveResponse`]
//! (out). Both hold it in a [`SecretString`], so their derived `Debug`
//! prints `[REDACTED]`. Requests ignore unknown fields: the engine stamps
//! `_caller_worker_id` into every invocation payload.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::detect::SourceKind;
use crate::secret::SecretString;

/// Fully qualified ids.
pub mod ids {
    pub const SET: &str = "secrets::set";
    pub const ACCESS: &str = "secrets::access";
    pub const DELETE: &str = "secrets::delete";
    pub const GET: &str = "secrets::get";
    pub const LIST: &str = "secrets::list";
    pub const RESOLVE: &str = "secrets::resolve";
    pub const DETECT: &str = "secrets::detect";
    pub const IMPORT: &str = "secrets::import";
    pub const STATUS: &str = "secrets::status";
    pub const CHANGED_TRIGGER: &str = "secrets::changed";
    pub const ALL_FUNCTIONS: [&str; 9] = [
        SET, ACCESS, DELETE, GET, LIST, RESOLVE, DETECT, IMPORT, STATUS,
    ];
}

/// Everything about a secret except its value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretMeta {
    pub name: String,
    /// `secret://NAME`, the string to put in versioned configuration.
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Worker names allowed to resolve the value. Empty: nobody.
    pub consumers: Vec<String>,
    /// Masked value, e.g. `sk-ant…9f2c`; `••••` under 12 characters.
    pub hint: String,
    /// First 16 hex characters of HMAC-SHA256(master key, value). Changes on rotation.
    pub fingerprint: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_resolved_at: Option<String>,
    /// Worker name of the last successful resolver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_resolved_by: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetRequest {
    /// `^[A-Za-z_][A-Za-z0-9_.-]{0,127}$`; provider keys reuse their env var name.
    pub name: String,
    /// The credential. Stored encrypted; never returned except by `secrets::resolve`.
    pub value: SecretString,
    /// Worker names allowed to resolve. Omit to keep the current list (`[]` for a new secret).
    #[serde(default)]
    pub consumers: Option<Vec<String>>,
    /// Omit to keep the current description; an empty string clears it.
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AccessRequest {
    pub name: String,
    /// The complete new allowlist of worker names; `[]` revokes everyone.
    pub consumers: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NameRequest {
    pub name: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct EmptyRequest {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResolveRequest {
    /// `secret://NAME` or a bare `NAME`.
    #[serde(rename = "ref")]
    pub reference: String,
    /// Engine-stamped id of the calling connection; any client value is overwritten.
    #[serde(rename = "_caller_worker_id", default)]
    #[schemars(skip)]
    pub caller_worker_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DetectRequest {
    /// Env var names to look for (also the secret names they would be stored under).
    pub names: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImportRequest {
    pub name: String,
    /// Where the worker re-reads the value; it never travels through the caller.
    pub source: SourceKind,
    /// Omit to keep the current list (`[]` for a new secret).
    #[serde(default)]
    pub consumers: Option<Vec<String>>,
    /// Omit to keep the current description; an empty string clears it.
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeleteResponse {
    pub deleted: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListResponse {
    pub secrets: Vec<SecretMeta>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ResolveResponse {
    pub name: String,
    pub value: SecretString,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DetectResponse {
    pub results: Vec<DetectResult>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DetectResult {
    pub name: String,
    pub stored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored_hint: Option<String>,
    pub sources: Vec<DetectedSource>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DetectedSource {
    pub kind: SourceKind,
    /// The secrets worker's environment, the `.env` path, or the login shell path.
    pub location: String,
    /// Masked value found there.
    pub hint: String,
    /// The value equals the stored one (same fingerprint).
    pub matches_stored: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum KeySourceKind {
    Env,
    File,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StatusResponse {
    pub vault_path: String,
    /// `env` when III_SECRETS_KEY supplies the master key, else `file`.
    pub key_source: KeySourceKind,
    /// The key file, when `key_source` is `file`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    pub count: usize,
    /// Vault file format version.
    pub version: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn requests_accept_the_engine_stamped_caller() {
        let resolve: ResolveRequest =
            serde_json::from_value(json!({"ref":"secret://A","_caller_worker_id":"w-1"})).unwrap();
        assert_eq!(resolve.caller_worker_id.as_deref(), Some("w-1"));
        for payload in [
            json!({"name":"A","value":"v","_caller_worker_id":"w-1"}),
            json!({"name":"A","value":"v"}),
        ] {
            serde_json::from_value::<SetRequest>(payload).unwrap();
        }
        serde_json::from_value::<EmptyRequest>(json!({"_caller_worker_id":"w"})).unwrap();
        serde_json::from_value::<NameRequest>(json!({"name":"A","_caller_worker_id":"w"})).unwrap();
    }

    #[test]
    fn value_carrying_types_redact_debug() {
        let set: SetRequest =
            serde_json::from_value(json!({"name":"A","value":"sk-very-secret-value-1234"}))
                .unwrap();
        let resolved = ResolveResponse {
            name: "A".into(),
            value: "sk-very-secret-value-1234".into(),
        };
        for printed in [format!("{set:?}"), format!("{resolved:#?}")] {
            assert!(!printed.contains("very-secret"), "{printed}");
        }
        // The wire form still carries it.
        assert_eq!(
            serde_json::to_value(&resolved).unwrap()["value"],
            "sk-very-secret-value-1234"
        );
    }

    #[test]
    fn resolve_schema_hides_the_engine_field() {
        let schema = serde_json::to_value(schemars::schema_for!(ResolveRequest)).unwrap();
        assert!(schema["properties"]["ref"].is_object());
        assert!(schema["properties"].get("_caller_worker_id").is_none());
    }
}
