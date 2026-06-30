use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn default_port() -> u16 {
    3111
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_timeout() -> u64 {
    30000
}

fn default_concurrency_request_limit() -> usize {
    1024
}

/// HTTP server settings. Doc comments on each field flow into the JSON Schema
/// (via `schemars`) that the `iii-http` configuration entry registers, so an
/// agent introspecting the schema sees the same descriptions and defaults
/// documented here.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RestApiConfig {
    /// TCP port the HTTP server binds to. Defaults to 3111. Use 0 to bind an
    /// OS-assigned ephemeral port (handy in tests).
    #[serde(default = "default_port")]
    pub port: u16,

    /// Host/interface to bind. Defaults to "0.0.0.0" (all interfaces).
    /// Supports `${VAR:default}` env expansion since it is a string field.
    #[serde(default = "default_host")]
    pub host: String,

    /// Per-request timeout in milliseconds; on expiry the server returns
    /// 504 Gateway Timeout. Defaults to 30000 (30s).
    #[serde(default = "default_timeout")]
    pub default_timeout: u64,

    /// CORS policy. When omitted, a permissive layer (any origin/method) is
    /// used. Set this to restrict allowed origins and methods.
    #[serde(default)]
    pub cors: Option<CorsConfig>,

    /// Maximum number of in-flight requests; requests over the limit wait for
    /// a slot. Must be at least 1 — a zero-permit limit would hang every
    /// request, so the schema rejects 0 at `configuration::set` time and any
    /// 0 loaded from a file (bypassing the schema) is clamped to 1.
    /// Defaults to 1024.
    #[serde(default = "default_concurrency_request_limit")]
    #[schemars(range(min = 1))]
    pub concurrency_request_limit: usize,

    /// Global middleware run on every route before the handler, in ascending
    /// `priority` order. Per-route middleware is set on the trigger instead.
    #[serde(default)]
    pub middleware: Vec<MiddlewareConfig>,
}

impl RestApiConfig {
    /// Normalize a freshly-loaded config. Runs on every load path (static
    /// block, seed, or a value read back from the configuration worker):
    /// - Sort global middleware by priority — views iterate in stored order.
    /// - Clamp `concurrency_request_limit` to ≥ 1. The schema's `minimum: 1`
    ///   only guards `configuration::set`; values loaded from a hand-edited
    ///   adapter file or static yaml bypass it, and a zero-permit
    ///   `ConcurrencyLimitLayer` would hang every request.
    pub fn normalized(mut self) -> Self {
        self.middleware.sort_by_key(|m| m.priority);
        if self.concurrency_request_limit == 0 {
            tracing::warn!(
                "iii-http: concurrency_request_limit 0 would block all requests; clamped to 1"
            );
            self.concurrency_request_limit = 1;
        }
        self
    }

    pub fn from_json(value: &serde_json::Value) -> Result<Self, String> {
        serde_json::from_value(value.clone())
            .map(Self::normalized)
            .map_err(|e| format!("json parse: {e}"))
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("RestApiConfig serializes")
    }

    pub fn validate(&self) -> Result<(), String> {
        Ok(()) // normalized already guarantees invariants
    }

    pub fn json_schema() -> serde_json::Value {
        let root = schemars::schema_for!(RestApiConfig);
        let mut schema = serde_json::to_value(&root.schema).expect("schema serializes");
        // Nested types (CorsConfig, MiddlewareConfig) are emitted as `$ref`s into
        // `#/definitions`; without carrying the definitions the schema has
        // dangling refs and `configuration::register` rejects it as invalid.
        if let Some(obj) = schema.as_object_mut() {
            if !root.definitions.is_empty() {
                obj.insert(
                    "definitions".into(),
                    serde_json::to_value(&root.definitions).expect("definitions serialize"),
                );
            }
        }
        schema
    }
}

impl Default for RestApiConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_host(),
            default_timeout: default_timeout(),
            cors: None,
            concurrency_request_limit: default_concurrency_request_limit(),
            middleware: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiddlewareConfig {
    /// ID of the function to invoke as middleware (e.g. `middleware::auth`).
    pub function_id: String,
    /// Lifecycle phase. Currently only "preHandler" (runs before the handler)
    /// is supported. Defaults to "preHandler".
    #[serde(default = "default_phase")]
    pub phase: String,
    /// Execution order among global middleware; lower runs first. Defaults to 0.
    #[serde(default)]
    pub priority: i64,
}

fn default_phase() -> String {
    "preHandler".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize, Default, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CorsConfig {
    /// Allowed CORS origins (e.g. "https://app.example.com"). Use "*" to allow
    /// any origin. An EMPTY list also allows any origin (permissive fallback)
    /// — to restrict origins, list them explicitly.
    #[serde(default)]
    pub allowed_origins: Vec<String>,

    /// Allowed CORS methods (e.g. "GET", "POST"). An EMPTY list allows any
    /// method (permissive fallback) — to restrict methods, list them
    /// explicitly.
    #[serde(default)]
    pub allowed_methods: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // RestApiConfig defaults
    // =========================================================================

    #[test]
    fn rest_api_config_default_values() {
        let config = RestApiConfig::default();
        assert_eq!(config.port, 3111);
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.default_timeout, 30000);
        assert!(config.cors.is_none());
        assert_eq!(config.concurrency_request_limit, 1024);
    }

    #[test]
    fn rest_api_config_deserialize_empty_json() {
        let config: RestApiConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.port, 3111);
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.default_timeout, 30000);
        assert!(config.cors.is_none());
        assert_eq!(config.concurrency_request_limit, 1024);
    }

    #[test]
    fn rest_api_config_deserialize_custom_values() {
        let json = r#"{
            "port": 8080,
            "host": "127.0.0.1",
            "default_timeout": 5000,
            "concurrency_request_limit": 512
        }"#;
        let config: RestApiConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.port, 8080);
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.default_timeout, 5000);
        assert_eq!(config.concurrency_request_limit, 512);
        assert!(config.cors.is_none());
    }

    #[test]
    fn rest_api_config_deserialize_with_cors() {
        let json = r#"{
            "cors": {
                "allowed_origins": ["http://localhost:3000", "https://example.com"],
                "allowed_methods": ["GET", "POST"]
            }
        }"#;
        let config: RestApiConfig = serde_json::from_str(json).unwrap();
        let cors = config.cors.unwrap();
        assert_eq!(
            cors.allowed_origins,
            vec!["http://localhost:3000", "https://example.com"]
        );
        assert_eq!(cors.allowed_methods, vec!["GET", "POST"]);
    }

    #[test]
    fn rest_api_config_serialize_roundtrip() {
        let config = RestApiConfig {
            port: 9090,
            host: "localhost".to_string(),
            default_timeout: 10000,
            cors: Some(CorsConfig {
                allowed_origins: vec!["*".to_string()],
                allowed_methods: vec!["GET".to_string()],
            }),
            concurrency_request_limit: 256,
            middleware: Vec::new(),
        };
        let json_str = serde_json::to_string(&config).unwrap();
        let deserialized: RestApiConfig = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.port, 9090);
        assert_eq!(deserialized.host, "localhost");
        assert_eq!(deserialized.default_timeout, 10000);
        assert_eq!(deserialized.concurrency_request_limit, 256);
        let cors = deserialized.cors.unwrap();
        assert_eq!(cors.allowed_origins, vec!["*"]);
        assert_eq!(cors.allowed_methods, vec!["GET"]);
    }

    #[test]
    fn middleware_config_deny_unknown_fields() {
        // A typo'd key inside a middleware object (e.g. "priorty") must fail
        // loudly instead of silently running the middleware at priority 0 —
        // global middleware is the auth/rate-limit chain, so ordering typos
        // are security-relevant.
        let json = r#"{
            "middleware": [
                {"function_id": "fn::auth", "priorty": 5}
            ]
        }"#;
        let result: Result<RestApiConfig, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "should reject unknown fields in middleware entries"
        );
    }

    #[test]
    fn normalized_clamps_zero_concurrency_limit() {
        let config: RestApiConfig =
            serde_json::from_str(r#"{"concurrency_request_limit": 0}"#).unwrap();
        assert_eq!(config.normalized().concurrency_request_limit, 1);
    }

    #[test]
    fn rest_api_config_deny_unknown_fields() {
        let json = r#"{"port": 3111, "unknown_field": true}"#;
        let result: Result<RestApiConfig, _> = serde_json::from_str(json);
        assert!(result.is_err(), "should reject unknown fields");
    }

    // =========================================================================
    // CorsConfig
    // =========================================================================

    #[test]
    fn cors_config_default() {
        let cors = CorsConfig::default();
        assert!(cors.allowed_origins.is_empty());
        assert!(cors.allowed_methods.is_empty());
    }

    #[test]
    fn cors_config_deserialize_empty() {
        let cors: CorsConfig = serde_json::from_str("{}").unwrap();
        assert!(cors.allowed_origins.is_empty());
        assert!(cors.allowed_methods.is_empty());
    }

    #[test]
    fn cors_config_deserialize_partial() {
        let json = r#"{"allowed_origins": ["http://example.com"]}"#;
        let cors: CorsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cors.allowed_origins, vec!["http://example.com"]);
        assert!(cors.allowed_methods.is_empty());
    }

    #[test]
    fn cors_config_deny_unknown_fields() {
        let json = r#"{"allowed_origins": [], "fake_key": true}"#;
        let result: Result<CorsConfig, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "should reject unknown fields in CorsConfig"
        );
    }

    #[test]
    fn rest_api_config_deny_unknown_nested_cors_field() {
        let json = r#"{
            "cors": {
                "allowed_origins": ["*"],
                "allow_credentials": true
            }
        }"#;
        let result: Result<RestApiConfig, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "should reject unknown fields in nested CorsConfig"
        );
    }

    // =========================================================================
    // YAML deserialization (via serde_yaml)
    // =========================================================================

    #[test]
    fn rest_api_config_from_yaml() {
        let yaml = r#"
port: 4000
host: "192.168.1.1"
default_timeout: 60000
concurrency_request_limit: 2048
cors:
  allowed_origins:
    - "https://app.example.com"
  allowed_methods:
    - "GET"
    - "POST"
    - "PUT"
"#;
        let config: RestApiConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.port, 4000);
        assert_eq!(config.host, "192.168.1.1");
        assert_eq!(config.default_timeout, 60000);
        assert_eq!(config.concurrency_request_limit, 2048);
        let cors = config.cors.unwrap();
        assert_eq!(cors.allowed_origins, vec!["https://app.example.com"]);
        assert_eq!(cors.allowed_methods, vec!["GET", "POST", "PUT"]);
    }

    #[test]
    fn rest_api_config_from_yaml_defaults() {
        let yaml = "{}";
        let config: RestApiConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.port, 3111);
        assert_eq!(config.host, "0.0.0.0");
    }

    // =========================================================================
    // MiddlewareConfig
    // =========================================================================

    #[test]
    fn rest_api_config_with_middleware() {
        let json = r#"{
            "middleware": [
                {
                    "function_id": "fn::auth_middleware",
                    "phase": "preHandler",
                    "priority": 10
                },
                {
                    "function_id": "fn::logging_middleware",
                    "phase": "preHandler",
                    "priority": 5
                }
            ]
        }"#;
        let config: RestApiConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.middleware.len(), 2);
        assert_eq!(config.middleware[0].function_id, "fn::auth_middleware");
        assert_eq!(config.middleware[0].phase, "preHandler");
        assert_eq!(config.middleware[0].priority, 10);
        assert_eq!(config.middleware[1].function_id, "fn::logging_middleware");
        assert_eq!(config.middleware[1].phase, "preHandler");
        assert_eq!(config.middleware[1].priority, 5);
    }

    #[test]
    fn rest_api_config_without_middleware_defaults_empty() {
        let config: RestApiConfig = serde_json::from_str("{}").unwrap();
        assert!(
            config.middleware.is_empty(),
            "middleware should default to empty vec"
        );
    }

    #[test]
    fn rest_api_config_middleware_from_yaml() {
        let yaml = r#"
middleware:
  - function_id: "fn::rate_limiter"
    phase: "preHandler"
    priority: 1
  - function_id: "fn::jwt_validator"
    phase: "preHandler"
    priority: 2
"#;
        let config: RestApiConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.middleware.len(), 2);
        assert_eq!(config.middleware[0].function_id, "fn::rate_limiter");
        assert_eq!(config.middleware[0].priority, 1);
        assert_eq!(config.middleware[1].function_id, "fn::jwt_validator");
        assert_eq!(config.middleware[1].priority, 2);
    }

    #[test]
    fn rest_api_config_middleware_pre_sorted() {
        let json = r#"{
            "middleware": [
                {"function_id": "fn::c", "priority": 30},
                {"function_id": "fn::a", "priority": 10},
                {"function_id": "fn::b", "priority": 20}
            ]
        }"#;
        let mut config: RestApiConfig = serde_json::from_str(json).unwrap();
        config.middleware.sort_by_key(|m| m.priority);
        assert_eq!(config.middleware[0].function_id, "fn::a");
        assert_eq!(config.middleware[1].function_id, "fn::b");
        assert_eq!(config.middleware[2].function_id, "fn::c");
    }

    #[test]
    fn middleware_config_defaults_phase_and_priority() {
        let json = r#"{"function_id": "fn::my_mw"}"#;
        let mw: MiddlewareConfig = serde_json::from_str(json).unwrap();
        assert_eq!(mw.function_id, "fn::my_mw");
        assert_eq!(mw.phase, "preHandler");
        assert_eq!(mw.priority, 0);
    }

    // =========================================================================
    // from_json / to_json / validate / json_schema
    // =========================================================================

    #[test]
    fn from_json_to_json_roundtrip() {
        let value = serde_json::json!({
            "port": 7000,
            "host": "127.0.0.1",
            "middleware": [
                {"function_id": "fn::b", "priority": 2},
                {"function_id": "fn::a", "priority": 1}
            ]
        });
        let config = RestApiConfig::from_json(&value).unwrap();
        assert_eq!(config.port, 7000);
        assert_eq!(config.host, "127.0.0.1");
        // from_json runs normalized(), so middleware is sorted by priority.
        assert_eq!(config.middleware[0].function_id, "fn::a");
        assert_eq!(config.middleware[1].function_id, "fn::b");

        let round_tripped = config.to_json();
        assert_eq!(round_tripped["port"], 7000);
        assert_eq!(round_tripped["host"], "127.0.0.1");
    }

    #[test]
    fn from_json_rejects_unknown_fields() {
        let value = serde_json::json!({"port": 3111, "unknown_field": true});
        assert!(RestApiConfig::from_json(&value).is_err());
    }

    #[test]
    fn validate_default_config_is_ok() {
        assert!(RestApiConfig::default().validate().is_ok());
    }

    #[test]
    fn json_schema_has_minimum_one_on_concurrency_request_limit() {
        let schema = RestApiConfig::json_schema();
        let prop = &schema["properties"]["concurrency_request_limit"];
        assert_eq!(prop["minimum"].as_f64(), Some(1.0));
    }
}
