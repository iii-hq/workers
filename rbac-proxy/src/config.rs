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

use crate::rbac::{self, RbacConfig};

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
    /// The RBAC contract (auth function, expose filters, the function and
    /// observer registration hooks).
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
    /// Parse a stored configuration. The `rbac` keys of entries stored before
    /// 0.26 ([`rbac::REMOVED_KEYS`]) are dropped with a `warn` first, so such
    /// an entry loads; every other unknown key is refused.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let mut value = value.clone();
        if let Some(rbac) = value.get_mut("rbac") {
            rbac::drop_removed_keys(rbac);
        }
        serde_json::from_value(value).map_err(|e| format!("json parse: {e}"))
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("WorkerConfig serializes")
    }

    /// The JSON Schema registered with the `configuration` worker, with the
    /// built-in default attached as the top-level `example` (mirrors
    /// approval-gate / context-manager).
    ///
    /// The `rbac` keys of entries stored before 0.26 ([`rbac::REMOVED_KEYS`])
    /// are in the schema as deprecated and ignored, so a stored entry that
    /// sets one validates and [`from_json`](Self::from_json) can drop it.
    pub fn json_schema() -> Value {
        let root = schemars::schema_for!(WorkerConfig);
        let mut schema =
            serde_json::to_value(&root.schema).expect("WorkerConfig JSON Schema serializes");
        let mut definitions =
            serde_json::to_value(&root.definitions).expect("definitions serialize");
        if let Some(properties) = definitions
            .pointer_mut("/RbacConfig/properties")
            .and_then(Value::as_object_mut)
        {
            for (key, replacement) in rbac::REMOVED_KEYS {
                properties.insert(
                    key.to_string(),
                    serde_json::json!({
                        "description": format!("Ignored: {replacement}."),
                        "deprecated": true,
                        "type": ["string", "null"],
                    }),
                );
            }
        }
        // `allow_function_registration: true` requires a non-empty
        // `function_registration_prefix` (`LoopbackAuth::validate`), so
        // `configuration::set` refuses what `from_json` would.
        if let Some(grants) = definitions
            .get_mut("LoopbackAuth")
            .and_then(Value::as_object_mut)
        {
            grants.insert(
                "if".into(),
                serde_json::json!({
                    "properties": { "allow_function_registration": { "const": true } },
                    "required": ["allow_function_registration"]
                }),
            );
            grants.insert(
                "then".into(),
                serde_json::json!({
                    "properties": {
                        "function_registration_prefix": { "type": "string", "pattern": "\\S" }
                    },
                    "required": ["function_registration_prefix"]
                }),
            );
        }
        if let Some(obj) = schema.as_object_mut() {
            if !root.definitions.is_empty() {
                obj.insert("definitions".into(), definitions);
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
        let cfg: WorkerConfig = serde_yaml_ng::from_str(yaml).unwrap();
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
    fn a_stored_entry_with_removed_rbac_keys_loads() {
        let mut stored = json!({ "rbac": { "auth_function_id": "p::auth" } });
        for (key, _) in rbac::REMOVED_KEYS {
            stored["rbac"][key] = json!("p::hook");
        }
        let cfg = WorkerConfig::from_json(&stored).unwrap();
        assert_eq!(cfg.rbac.auth_function_id.as_deref(), Some("p::auth"));
        assert!(
            !cfg.to_json()["rbac"]
                .as_object()
                .unwrap()
                .keys()
                .any(|k| rbac::REMOVED_KEYS.iter().any(|(removed, _)| removed == k))
        );

        // The schema still accepts them, as deprecated.
        let schema = WorkerConfig::json_schema();
        for (key, _) in rbac::REMOVED_KEYS {
            assert_eq!(
                schema["definitions"]["RbacConfig"]["properties"][key]["deprecated"],
                true
            );
        }
    }

    #[test]
    fn expose_endpoints_is_optional_and_in_the_schema() {
        assert!(WorkerConfig::default().rbac.expose_endpoints.is_none());
        let cfg: WorkerConfig = serde_yaml_ng::from_str(
            "rbac:\n  expose_endpoints:\n    - match(\"/\")\n    - match(\"/otel\")\n",
        )
        .unwrap();
        assert_eq!(cfg.rbac.expose_endpoints.as_ref().map(Vec::len), Some(2));
        let schema = WorkerConfig::json_schema();
        let prop = &schema["definitions"]["RbacConfig"]["properties"]["expose_endpoints"];
        assert!(prop.to_string().contains("\"array\""), "{prop}");
        // The default leaves it out, which serves `/`.
        assert!(schema["example"]["rbac"].get("expose_endpoints").is_none());
    }

    #[test]
    fn config_parses_with_and_without_loopback_auth() {
        let without = WorkerConfig::from_json(&json!({ "rbac": {} })).unwrap();
        assert!(without.rbac.loopback_auth.is_none());
        assert_eq!(without, WorkerConfig::default());

        let with: WorkerConfig = serde_yaml_ng::from_str(
            "rbac:\n  loopback_auth:\n    worker_names: [mcp-server]\n    forbidden_functions: [secrets::resolve]\n",
        )
        .unwrap();
        let grants = with.rbac.loopback_auth.as_ref().unwrap();
        assert_eq!(grants.worker_names, vec!["mcp-server"]);
        assert_eq!(grants.forbidden_functions, vec!["secrets::resolve"]);
        // The rest takes the auth result's defaults, except registration.
        assert!(!grants.allow_function_registration);
        assert_eq!(grants.context, json!({}));
        assert!(grants.namespaces.is_empty());
        assert!(grants.allowed_functions.is_empty());
        assert!(grants.function_registration_prefix.is_none());
        assert!(with.rbac.auth_function_id.is_none());
        assert!(
            !with.rbac_enabled(),
            "rbac_enabled keeps meaning an auth function"
        );
        assert_eq!(WorkerConfig::from_json(&with.to_json()).unwrap(), with);

        // `{}` admits loopback peers with the defaults.
        let empty = WorkerConfig::from_json(&json!({ "rbac": { "loopback_auth": {} } })).unwrap();
        assert_eq!(
            empty.rbac.loopback_auth,
            Some(crate::rbac::LoopbackAuth::default())
        );
        assert!(
            WorkerConfig::from_json(&json!({ "rbac": { "loopback_auth": { "nope": 1 } } }))
                .is_err()
        );
        // Registration without a prefix is a configuration error: a stored
        // value fails to load instead of granting unprefixed registration.
        let err = WorkerConfig::from_json(&json!({
            "rbac": { "loopback_auth": { "allow_function_registration": true } }
        }))
        .unwrap_err();
        assert!(err.contains("function_registration_prefix"), "{err}");
    }

    #[test]
    fn loopback_auth_is_in_the_schema_with_docs() {
        let schema = WorkerConfig::json_schema();
        let prop = &schema["definitions"]["RbacConfig"]["properties"]["loopback_auth"];
        assert!(
            prop["description"]
                .as_str()
                .is_some_and(|d| d.contains("loopback") && d.contains("auth_function_id")),
            "{prop}"
        );
        assert!(
            prop.to_string().contains("#/definitions/LoopbackAuth"),
            "{prop}"
        );
        let grants = &schema["definitions"]["LoopbackAuth"];
        assert_eq!(grants["type"], "object");
        // Unknown keys are refused by configuration::set too.
        assert_eq!(grants["additionalProperties"], false, "{grants}");
        for key in [
            "namespaces",
            "allowed_functions",
            "forbidden_functions",
            "allow_function_registration",
            "context",
            "function_registration_prefix",
            "worker_names",
        ] {
            assert!(
                grants["properties"][key]["description"].is_string(),
                "{key} has no description: {grants}"
            );
        }
        assert_eq!(
            grants["properties"]["allow_function_registration"]["default"],
            false
        );
        // configuration::set refuses registration without a prefix.
        assert_eq!(
            grants["if"],
            json!({
                "properties": { "allow_function_registration": { "const": true } },
                "required": ["allow_function_registration"]
            }),
            "{grants}"
        );
        assert_eq!(
            grants["then"],
            json!({
                "properties": {
                    "function_registration_prefix": { "type": "string", "pattern": "\\S" }
                },
                "required": ["function_registration_prefix"]
            }),
            "{grants}"
        );
        for key in [
            "allow_function_registration",
            "function_registration_prefix",
        ] {
            let doc = grants["properties"][key]["description"].as_str().unwrap();
            assert!(doc.contains("required"), "{key}: {doc}");
        }
        // The built-in default leaves it unset: no change unless asked for.
        assert!(schema["example"]["rbac"].get("loopback_auth").is_none());
    }

    #[test]
    fn json_schema_is_typed_object_with_example() {
        let schema = WorkerConfig::json_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].is_object());
        assert!(schema["example"].is_object());
    }
}
