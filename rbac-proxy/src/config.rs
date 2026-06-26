//! Runtime configuration for `rbac-proxy`.
//!
//! Config lives in the `configuration` worker under id `rbac-proxy`; no
//! `config.yaml` is committed and [`WorkerConfig::default`] seeds first boot
//! (binary-worker SOP §4d). This is intentionally the same field set as the
//! engine's `WorkerManagerConfig` / the devexp `gateway:` block, so an
//! operator's mental model transfers and a migration to/from `worker-gateway`
//! is a config copy.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::rbac::RbacConfig;

/// Default upstream engine listener (also the control connection's `--url`
/// default).
pub const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

fn default_host() -> String {
    "0.0.0.0".to_string()
}

/// The public RBAC port. Distinct from the engine's `49134` so a co-located
/// engine + proxy do not collide.
fn default_port() -> u16 {
    49200
}

fn default_engine_url() -> String {
    DEFAULT_ENGINE_URL.to_string()
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Host the public RBAC listener binds. Structural (rebind on change).
    #[serde(default = "default_host")]
    pub host: String,
    /// The public RBAC port. Structural (rebind on change).
    #[serde(default = "default_port")]
    pub port: u16,
    /// The trusted internal engine listener the data plane proxies to.
    /// New connections dial this; existing connections finish on their
    /// captured upstream (no forced cutover).
    #[serde(default = "default_engine_url")]
    pub engine_url: String,
    /// When set, every allowed, non-`engine::` invocation is routed to this
    /// function instead of the target; its return value becomes the result.
    #[serde(default)]
    pub middleware_function_id: Option<String>,
    /// Strip operational identity (`pid`, `ip_address`, `isolation`,
    /// `internal`, `latest_metrics`) from `engine::workers::*` results.
    /// `false` (strip) is the multi-tenant-safe default.
    #[serde(default)]
    pub expose_worker_internals: bool,
    /// The RBAC contract (auth function, expose filters, registration hooks).
    #[serde(default)]
    pub rbac: RbacConfig,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            engine_url: default_engine_url(),
            middleware_function_id: None,
            expose_worker_internals: false,
            rbac: RbacConfig::default(),
        }
    }
}

/// The fields that decide a hot-reload's class. Per the spec these are the
/// "structural" fields. The `rbac-proxy::on-config-change` handler compares
/// signatures: a change to `host`/`port` rebinds the public listener; an
/// `engine_url` change needs no rebind because each new connection reads
/// `engine_url` live from the `ConfigCell` snapshot (so it is carried here for
/// spec fidelity but does not by itself trigger a rebind).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootSignature {
    pub host: String,
    pub port: u16,
    pub engine_url: String,
}

impl WorkerConfig {
    pub fn from_json(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|e| format!("json parse: {e}"))
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("WorkerConfig serializes")
    }

    /// The JSON Schema registered with the `configuration` worker, with the
    /// built-in default attached as the top-level `example` (mirrors
    /// approval-gate / context-manager).
    pub fn json_schema() -> Value {
        let root = schemars::schema_for!(WorkerConfig);
        let mut schema =
            serde_json::to_value(&root.schema).expect("WorkerConfig JSON Schema serializes");
        if let Some(obj) = schema.as_object_mut() {
            if !root.definitions.is_empty() {
                obj.insert(
                    "definitions".into(),
                    serde_json::to_value(&root.definitions).expect("definitions serialize"),
                );
            }
            obj.insert("example".into(), WorkerConfig::default().to_json());
        }
        schema
    }

    pub fn boot_signature(&self) -> BootSignature {
        BootSignature {
            host: self.host.clone(),
            port: self.port,
            engine_url: self.engine_url.clone(),
        }
    }

    /// True when the public listener must be rebound (host/port changed). An
    /// `engine_url`-only change returns `false`: new connections pick it up
    /// from the swapped snapshot.
    pub fn requires_rebind(&self, other: &WorkerConfig) -> bool {
        self.host != other.host || self.port != other.port
    }

    /// `true` when an `auth_function_id` is configured.
    pub fn rbac_enabled(&self) -> bool {
        self.rbac.auth_function_id.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_from_empty_object() {
        let cfg: WorkerConfig = serde_json::from_value(json!({})).unwrap();
        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.port, 49200);
        assert_eq!(cfg.engine_url, "ws://127.0.0.1:49134");
        assert!(cfg.middleware_function_id.is_none());
        assert!(!cfg.expose_worker_internals);
        assert!(cfg.rbac.auth_function_id.is_none());
        assert_eq!(cfg, WorkerConfig::default());
    }

    #[test]
    fn round_trips_full_config() {
        let yaml = r#"
            host: 0.0.0.0
            port: 49200
            engine_url: ws://127.0.0.1:49134
            expose_worker_internals: false
            middleware_function_id: my-project::middleware-function
            rbac:
              auth_function_id: my-project::auth-function
              on_function_registration_function_id: my-project::on-function-reg
              expose_functions:
                - match("api::*")
                - metadata:
                    public: true
        "#;
        let cfg: WorkerConfig = serde_yaml::from_str(yaml).unwrap();
        let back = WorkerConfig::from_json(&cfg.to_json()).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn deny_unknown_fields() {
        let err = WorkerConfig::from_json(&json!({ "nope": 1 }));
        assert!(err.is_err());
    }

    #[test]
    fn boot_signature_distinguishes_structural_vs_tuning() {
        let base = WorkerConfig::default();

        // Tuning-only: rbac change keeps host/port → no rebind.
        let mut tuned = base.clone();
        tuned.rbac.auth_function_id = Some("p::auth".into());
        assert!(!base.requires_rebind(&tuned));

        // Structural: port change → rebind.
        let mut moved = base.clone();
        moved.port = 50000;
        assert!(base.requires_rebind(&moved));

        // engine_url change is carried in the signature but does NOT rebind.
        let mut re = base.clone();
        re.engine_url = "ws://10.0.0.1:49134".into();
        assert!(!base.requires_rebind(&re));
        assert_ne!(base.boot_signature(), re.boot_signature());
    }

    #[test]
    fn json_schema_is_typed_object_with_example() {
        let schema = WorkerConfig::json_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].is_object());
        assert!(schema["example"].is_object());
    }
}
