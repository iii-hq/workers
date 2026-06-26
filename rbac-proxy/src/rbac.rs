//! The RBAC contract — vendored from the engine so a connection through
//! `rbac-proxy` is gated exactly as the same connection through an engine
//! `iii-worker-manager` RBAC listener would be.
//!
//! The decision logic ([`is_function_allowed`], [`WildcardPattern`],
//! [`FunctionFilter`], [`INFRASTRUCTURE_FUNCTIONS`]) is copied **verbatim**
//! from `iii/engine/src/workers/worker/rbac_config.rs` so the proxy and a
//! `worker-gateway` listener never diverge — only the home of the rules
//! differs. The one adaptation is the access-check input: the engine threads
//! an in-process `Function` and reads `function.metadata`; the proxy has no
//! such object, so it passes the function's registered `metadata` (sourced
//! from the catalog cache) directly. A parity fixture in the tests asserts the
//! vendored matcher agrees with the engine's.
//!
//! The proxy also holds, per downstream connection, a [`ProxySession`] that
//! mirrors the engine's `Session` fields (rbac_session.rs) — the boundaries it
//! re-derives itself because a WebSocket worker receives no in-process
//! `Session` (see the spec's *The proxy has no engine `Session`*).

use std::collections::HashMap;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// RbacConfig — the operator-facing RBAC block (same field set as the engine's
// WorkerManagerConfig.rbac and the devexp `gateway:` block).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct RbacConfig {
    /// Engine function id invoked once per upgrade to authenticate the
    /// connection. Unset ⇒ permissive default session; `expose_functions`
    /// alone gates access.
    #[serde(default)]
    pub auth_function_id: Option<String>,
    /// Filters exposing functions for invocation/discovery; a function is
    /// exposed if **any** filter matches.
    #[serde(default)]
    pub expose_functions: Vec<FunctionFilter>,
    #[serde(default)]
    pub on_function_registration_function_id: Option<String>,
    #[serde(default)]
    pub on_trigger_registration_function_id: Option<String>,
    #[serde(default)]
    pub on_trigger_type_registration_function_id: Option<String>,
}

impl RbacConfig {
    /// `true` when any `expose_functions` filter is a metadata filter. Callers
    /// use this to skip the catalog lookup (and its control-connection round
    /// trip) entirely for wildcard-only configs — a wildcard filter ignores
    /// metadata, so a cold cache must not fail it closed.
    pub fn uses_metadata(&self) -> bool {
        self.expose_functions
            .iter()
            .any(|f| matches!(f, FunctionFilter::Metadata(_)))
    }
}

// ---------------------------------------------------------------------------
// Wildcard matcher — VENDORED VERBATIM from rbac_config.rs:30-82.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct WildcardPattern {
    raw: String,
}

impl WildcardPattern {
    pub fn new(pattern: &str) -> Self {
        Self {
            raw: pattern.to_string(),
        }
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    pub fn matches(&self, value: &str) -> bool {
        wildcard_match(&self.raw, value)
    }
}

/// `*` matches any run of characters; the pattern is anchored at both ends and
/// is case-sensitive. `*` alone matches everything. Copied verbatim from the
/// engine so wildcard semantics are byte-identical.
fn wildcard_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    let parts: Vec<&str> = pattern.split('*').collect();

    if parts.len() == 1 {
        return pattern == value;
    }

    let mut pos = 0;

    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }

        if let Some(found) = value[pos..].find(part) {
            if i == 0 && found != 0 {
                return false;
            }
            pos += found + part.len();
        } else {
            return false;
        }
    }

    if let Some(last) = parts.last() {
        if !last.is_empty() && !value.ends_with(last) {
            return false;
        }
    }

    true
}

// ---------------------------------------------------------------------------
// Metadata matcher — VENDORED from rbac_config.rs:84-103.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum MetadataValue {
    Exact(Value),
    Wildcard(WildcardPattern),
}

impl MetadataValue {
    fn matches(&self, value: &Value) -> bool {
        match self {
            MetadataValue::Exact(expected) => value == expected,
            MetadataValue::Wildcard(pattern) => {
                if let Some(s) = value.as_str() {
                    pattern.matches(s)
                } else {
                    false
                }
            }
        }
    }

    /// The operator-facing JSON form (symmetric with deserialization): a
    /// wildcard serializes back to its `match("…")` string, an exact value to
    /// itself. This keeps `to_json`/`from_json` round-tripping through the
    /// `configuration` worker (unlike the engine, whose `RbacConfig` is only
    /// ever deserialized).
    fn to_json(&self) -> Value {
        match self {
            MetadataValue::Exact(v) => v.clone(),
            MetadataValue::Wildcard(p) => Value::String(format!("match(\"{}\")", p.raw)),
        }
    }
}

// ---------------------------------------------------------------------------
// FunctionFilter — VENDORED matcher + custom Deserialize from rbac_config.rs;
// the Serialize/JsonSchema impls are the proxy's own (symmetric round-trip).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum FunctionFilter {
    Match(WildcardPattern),
    Metadata(HashMap<String, MetadataValue>),
}

impl FunctionFilter {
    /// VENDORED from rbac_config.rs:111-128. A function with no metadata never
    /// matches a metadata filter; metadata keys are AND'd within a filter.
    pub fn matches(&self, function_id: &str, metadata: Option<&Value>) -> bool {
        match self {
            FunctionFilter::Match(pattern) => pattern.matches(function_id),
            FunctionFilter::Metadata(expected) => {
                let Some(metadata) = metadata else {
                    return false;
                };
                let Some(obj) = metadata.as_object() else {
                    return false;
                };
                expected
                    .iter()
                    .all(|(key, matcher)| obj.get(key).is_some_and(|v| matcher.matches(v)))
            }
        }
    }
}

/// VENDORED from rbac_config.rs:130-139.
fn parse_match_pattern(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if (trimmed.starts_with("match(\"") && trimmed.ends_with("\")"))
        || (trimmed.starts_with("match('") && trimmed.ends_with("')"))
    {
        Some(trimmed[7..trimmed.len() - 2].to_string())
    } else {
        None
    }
}

/// VENDORED from rbac_config.rs:141-148.
fn parse_metadata_value(value: Value) -> MetadataValue {
    if let Some(s) = value.as_str() {
        if let Some(pattern) = parse_match_pattern(s) {
            return MetadataValue::Wildcard(WildcardPattern::new(&pattern));
        }
    }
    MetadataValue::Exact(value)
}

/// VENDORED from rbac_config.rs:150-207 — accepts a `match("pattern")` string
/// or a `{ metadata: { … } }` map.
impl<'de> Deserialize<'de> for FunctionFilter {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FilterVisitor;

        impl<'de> Visitor<'de> for FilterVisitor {
            type Value = FunctionFilter;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a match(\"pattern\") string or a map with 'metadata' key")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if let Some(pattern) = parse_match_pattern(v) {
                    Ok(FunctionFilter::Match(WildcardPattern::new(&pattern)))
                } else {
                    Err(de::Error::custom(format!(
                        "expected match(\"pattern\"), got: {}",
                        v
                    )))
                }
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut metadata_map: HashMap<String, MetadataValue> = HashMap::new();

                while let Some(key) = map.next_key::<String>()? {
                    if key == "metadata" {
                        let inner: HashMap<String, Value> = map.next_value()?;
                        for (k, v) in inner {
                            metadata_map.insert(k, parse_metadata_value(v));
                        }
                    } else {
                        let _: Value = map.next_value()?;
                    }
                }

                if metadata_map.is_empty() {
                    Err(de::Error::custom(
                        "expected a 'metadata' key with filter conditions",
                    ))
                } else {
                    Ok(FunctionFilter::Metadata(metadata_map))
                }
            }
        }

        deserializer.deserialize_any(FilterVisitor)
    }
}

/// Symmetric with [`FunctionFilter`]'s `Deserialize`: a `Match` serializes to
/// the `match("…")` string form, a `Metadata` filter to `{ "metadata": { … } }`.
/// The engine's `RbacConfig` does not round-trip (it is deserialize-only), but
/// the proxy persists its config through the `configuration` worker and must
/// re-read what it stored, so the canonical serialized form is the
/// operator-facing one.
impl Serialize for FunctionFilter {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            FunctionFilter::Match(p) => {
                serializer.serialize_str(&format!("match(\"{}\")", p.raw))
            }
            FunctionFilter::Metadata(map) => {
                let mut inner = serde_json::Map::new();
                for (k, v) in map {
                    inner.insert(k.clone(), v.to_json());
                }
                let mut m = serializer.serialize_map(Some(1))?;
                m.serialize_entry("metadata", &Value::Object(inner))?;
                m.end()
            }
        }
    }
}

/// A typed-enough JSON Schema for the `configuration` worker and the manifest:
/// either a `match("…")` string or an object with a `metadata` map. (schemars
/// cannot derive this because of the custom (a)symmetric serde.)
impl JsonSchema for FunctionFilter {
    fn schema_name() -> String {
        "FunctionFilter".to_string()
    }

    /// Inline the schema rather than emitting a `$ref` into definitions — the
    /// custom (a)symmetric serde has no derivable named schema.
    fn is_referenceable() -> bool {
        false
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        serde_json::from_value(json!({
            "anyOf": [
                {
                    "type": "string",
                    "description": "Wildcard match, e.g. match(\"api::*\"). `*` matches any run of characters; anchored at both ends."
                },
                {
                    "type": "object",
                    "description": "Metadata filter: all keys must match (AND). String values may be a match(\"…\") wildcard.",
                    "properties": { "metadata": { "type": "object" } },
                    "required": ["metadata"],
                    "additionalProperties": false
                }
            ]
        }))
        .expect("FunctionFilter schema is valid")
    }
}

// ---------------------------------------------------------------------------
// Infrastructure carve-out — VENDORED VERBATIM from rbac_config.rs:240-251.
//
// Part of iii's public contract: additive-only within a major version. These
// ten ids keep connection setup, channel creation, logging, and context
// propagation working regardless of `expose_functions`. The eight *discovery*
// functions are deliberately NOT here — they are gated by `expose_functions`
// and their results are rewritten (see `engine_overrides`).
// ---------------------------------------------------------------------------

pub const INFRASTRUCTURE_FUNCTIONS: &[&str] = &[
    "engine::channels::create",
    "engine::workers::register",
    "engine::log::info",
    "engine::log::warn",
    "engine::log::error",
    "engine::log::debug",
    "engine::log::trace",
    "engine::baggage::get",
    "engine::baggage::set",
    "engine::baggage::get_all",
];

/// VENDORED from rbac_config.rs:253-289, with the engine's `function:
/// Option<&Function>` replaced by `metadata: Option<&Value>` (the proxy reads
/// the function's registered metadata from its catalog cache rather than from
/// an in-process `Function`). The decision flow is unchanged:
///
/// 1. `forbidden` → deny
/// 2. `allowed` → allow
/// 3. infrastructure carve-out → allow
/// 4. any `expose_functions` filter matches → allow
/// 5. otherwise → deny
pub fn is_function_allowed(
    function_id: &str,
    config: Option<&RbacConfig>,
    allowed_functions: &[String],
    forbidden_functions: &[String],
    metadata: Option<&Value>,
) -> bool {
    if forbidden_functions.iter().any(|f| f == function_id) {
        if INFRASTRUCTURE_FUNCTIONS.contains(&function_id) {
            tracing::warn!(
                function_id = %function_id,
                "auth function forbids infrastructure function '{}' — worker may behave unpredictably (connection setup, logging, or context propagation may be blocked)",
                function_id
            );
        }
        return false;
    }

    if allowed_functions.iter().any(|f| f == function_id) {
        return true;
    }

    if INFRASTRUCTURE_FUNCTIONS.contains(&function_id) {
        return true;
    }

    if let Some(config) = config {
        config
            .expose_functions
            .iter()
            .any(|filter| filter.matches(function_id, metadata))
    } else {
        true
    }
}

// ---------------------------------------------------------------------------
// Per-connection session — mirrors the engine's `Session` (rbac_session.rs).
// ---------------------------------------------------------------------------

/// Held by the proxy for the lifetime of each downstream connection. Immutable
/// after the auth function resolves it; shared (behind `Arc`) by both pump
/// directions.
#[derive(Debug, Clone)]
pub struct ProxySession {
    pub session_id: Uuid,
    pub ip_address: String,
    pub allowed_functions: Vec<String>,
    pub forbidden_functions: Vec<String>,
    /// `None` = all trigger types allowed.
    pub allowed_trigger_types: Option<Vec<String>>,
    pub allow_trigger_type_registration: bool,
    pub allow_function_registration: bool,
    /// Forwarded to middleware + registration hooks.
    pub context: Value,
    /// Private namespace prefix for this session's own registrations.
    pub function_registration_prefix: Option<String>,
}

impl ProxySession {
    /// The permissive default used when no `auth_function_id` is configured —
    /// identical to the engine's no-auth-function branch (rbac_session.rs:76-86):
    /// note `allow_trigger_type_registration: true` here, unlike the
    /// `AuthResult` serde default (`false`).
    pub fn permissive(ip_address: String) -> Self {
        Self {
            session_id: Uuid::new_v4(),
            ip_address,
            allowed_functions: vec![],
            forbidden_functions: vec![],
            allowed_trigger_types: None,
            allow_trigger_type_registration: true,
            allow_function_registration: true,
            context: json!({}),
            function_registration_prefix: None,
        }
    }

    /// Does this session's prefix own `candidate` (`{prefix}::…`)?
    pub fn has_prefix(&self) -> bool {
        self.function_registration_prefix.is_some()
    }
}

// ---------------------------------------------------------------------------
// AuthResult — VENDORED defaults from rbac_session.rs:48-64.
// ---------------------------------------------------------------------------

fn default_true() -> bool {
    true
}

fn default_context() -> Value {
    json!({})
}

#[derive(Debug, Deserialize)]
pub struct AuthResult {
    #[serde(default)]
    pub allowed_functions: Vec<String>,
    #[serde(default)]
    pub forbidden_functions: Vec<String>,
    #[serde(default)]
    pub allowed_trigger_types: Option<Vec<String>>,
    #[serde(default)]
    pub allow_trigger_type_registration: bool,
    #[serde(default = "default_true")]
    pub allow_function_registration: bool,
    #[serde(default = "default_context")]
    pub context: Value,
    #[serde(default)]
    pub function_registration_prefix: Option<String>,
}

impl AuthResult {
    pub fn into_session(self, ip_address: String) -> ProxySession {
        ProxySession {
            session_id: Uuid::new_v4(),
            ip_address,
            allowed_functions: self.allowed_functions,
            forbidden_functions: self.forbidden_functions,
            allowed_trigger_types: self.allowed_trigger_types,
            allow_trigger_type_registration: self.allow_trigger_type_registration,
            allow_function_registration: self.allow_function_registration,
            context: self.context,
            function_registration_prefix: self.function_registration_prefix,
        }
    }
}

/// A rejected upgrade: the `{ code, message }` carried in the out-of-band
/// `{"type":"error",...}` frame the proxy sends before closing.
#[derive(Debug, Clone)]
pub struct AuthRejection {
    pub code: String,
    pub message: String,
}

/// Authenticate an upgrade and derive its [`ProxySession`].
///
/// Invokes `rbac.auth_function_id` **once per upgrade** over the control
/// connection (`iii.trigger`), exactly as the engine invokes it via
/// `engine.call` (rbac_session.rs:67-107). Distinguishes "unset" (permissive
/// default) from "set but unresolvable/errored" (**fail closed** — reject the
/// upgrade); a broken control plane never opens the door.
pub async fn resolve_session(
    iii: &IIIClient,
    rbac: Option<&RbacConfig>,
    headers: HashMap<String, String>,
    query_params: HashMap<String, Vec<String>>,
    ip_address: String,
) -> Result<ProxySession, AuthRejection> {
    let Some(auth_fn) = rbac.and_then(|c| c.auth_function_id.as_deref()) else {
        // No auth function: permissive default session (expose_functions alone
        // gates access) — identical to the engine default.
        return Ok(ProxySession::permissive(ip_address));
    };

    let input = json!({
        "headers": headers,
        "query_params": query_params,
        "ip_address": ip_address,
    });

    let result = iii
        .trigger(TriggerRequest {
            function_id: auth_fn.to_string(),
            payload: input,
            action: None,
            timeout_ms: None,
        })
        .await;

    match result {
        Err(e) => Err(AuthRejection {
            code: "AUTH_ERROR".to_string(),
            message: e.to_string(),
        }),
        Ok(v) if v.is_null() => Err(AuthRejection {
            code: "AUTH_ERROR".to_string(),
            message: "auth function returned no result".to_string(),
        }),
        Ok(v) => match serde_json::from_value::<AuthResult>(v) {
            Ok(r) => Ok(r.into_session(ip_address)),
            Err(e) => Err(AuthRejection {
                code: "AUTH_ERROR".to_string(),
                message: e.to_string(),
            }),
        },
    }
}

// ---------------------------------------------------------------------------
// Access decision helpers used by the interceptor and the engine overrides.
// ---------------------------------------------------------------------------

/// `is_function_allowed` bound to a session + the live `rbac` config snapshot.
pub fn access_allowed(
    rbac: Option<&RbacConfig>,
    session: &ProxySession,
    function_id: &str,
    metadata: Option<&Value>,
) -> bool {
    is_function_allowed(
        function_id,
        rbac,
        &session.allowed_functions,
        &session.forbidden_functions,
        metadata,
    )
}

/// The remediation branch the engine uses in its `FORBIDDEN` message
/// (engine/mod.rs:912-918): "remove from rbac.forbidden_functions" when the id
/// was explicitly forbidden (rule 1), else "add to rbac.expose_functions".
pub fn remediation(session: &ProxySession, function_id: &str) -> &'static str {
    if session.forbidden_functions.iter().any(|f| f == function_id) {
        "remove from rbac.forbidden_functions"
    } else {
        "add to rbac.expose_functions"
    }
}

/// The engine's `FORBIDDEN` message text (engine/mod.rs:927-930). SDKs key the
/// rejection off the `code`, but the message + remediation are reproduced for
/// parity.
pub fn forbidden_message(function_id: &str, remediation: &str) -> String {
    format!("function '{}' not allowed ({})", function_id, remediation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- Wildcard parity fixtures (mirrors rbac_config.rs unit tests) ------

    #[test]
    fn wildcard_exact_prefix_suffix_contains_star_middle() {
        assert!(WildcardPattern::new("hello::world").matches("hello::world"));
        assert!(!WildcardPattern::new("hello::world").matches("hello::worldx"));

        let p = WildcardPattern::new("engine::*");
        assert!(p.matches("engine::foo"));
        assert!(p.matches("engine::bar::baz"));
        assert!(!p.matches("other::foo"));

        let p = WildcardPattern::new("*::public");
        assert!(p.matches("api::public"));
        assert!(p.matches("x::y::public"));
        assert!(!p.matches("api::private"));

        let p = WildcardPattern::new("*public*");
        assert!(p.matches("public"));
        assert!(p.matches("mypublicfn"));
        assert!(!p.matches("private"));

        assert!(WildcardPattern::new("*").matches("anything"));
        assert!(WildcardPattern::new("*").matches(""));

        let p = WildcardPattern::new("api::*::read");
        assert!(p.matches("api::users::read"));
        assert!(!p.matches("api::users::write"));
    }

    // --- Access-resolution order parity (forbidden > allowed > carve-out >
    //     expose > deny), the exact fixtures from rbac_config.rs ------------

    fn cfg(patterns: &[&str]) -> RbacConfig {
        RbacConfig {
            expose_functions: patterns
                .iter()
                .map(|p| FunctionFilter::Match(WildcardPattern::new(p)))
                .collect(),
            ..RbacConfig::default()
        }
    }

    #[test]
    fn forbidden_takes_precedence_over_allowed_and_expose() {
        let c = cfg(&["*"]);
        let allowed = vec!["test::fn".to_string()];
        let forbidden = vec!["test::fn".to_string()];
        assert!(!is_function_allowed(
            "test::fn",
            Some(&c),
            &allowed,
            &forbidden,
            None
        ));
    }

    #[test]
    fn allowed_overrides_empty_expose() {
        let c = cfg(&[]);
        let allowed = vec!["test::fn".to_string()];
        assert!(is_function_allowed("test::fn", Some(&c), &allowed, &[], None));
    }

    #[test]
    fn carve_out_always_allowed_and_respects_forbidden() {
        for id in INFRASTRUCTURE_FUNCTIONS {
            let c = cfg(&[]);
            assert!(
                is_function_allowed(id, Some(&c), &[], &[], None),
                "carve-out {id} should be allowed with empty expose"
            );
            let c = cfg(&[]);
            let forbidden = vec![id.to_string()];
            assert!(
                !is_function_allowed(id, Some(&c), &[], &forbidden, None),
                "carve-out {id} should be denied when forbidden"
            );
        }
    }

    #[test]
    fn deny_by_default() {
        let c = cfg(&["api::*"]);
        assert!(!is_function_allowed("internal::fn", Some(&c), &[], &[], None));
    }

    #[test]
    fn discovery_functions_are_gated_not_carved_out() {
        for id in [
            "engine::functions::list",
            "engine::functions::info",
            "engine::workers::list",
            "engine::workers::info",
            "engine::triggers::list",
            "engine::triggers::info",
            "engine::registered-triggers::list",
            "engine::registered-triggers::info",
        ] {
            let c = cfg(&[]);
            assert!(
                !is_function_allowed(id, Some(&c), &[], &[], None),
                "discovery id {id} must be gated by expose_functions, not carved out"
            );
        }
        // …but reachable when exposed.
        let c = cfg(&["engine::functions::*"]);
        assert!(is_function_allowed(
            "engine::functions::list",
            Some(&c),
            &[],
            &[],
            None
        ));
    }

    #[test]
    fn no_rbac_config_allows_everything() {
        for id in ["api::anything", "internal::private", "engine::workers::list"] {
            assert!(is_function_allowed(id, None, &[], &[], None));
        }
    }

    #[test]
    fn metadata_filter_matches_registered_metadata() {
        let mut meta = HashMap::new();
        meta.insert("public".to_string(), MetadataValue::Exact(json!(true)));
        let c = RbacConfig {
            expose_functions: vec![FunctionFilter::Metadata(meta)],
            ..RbacConfig::default()
        };
        assert!(is_function_allowed(
            "any::fn",
            Some(&c),
            &[],
            &[],
            Some(&json!({"public": true}))
        ));
        assert!(!is_function_allowed(
            "any::fn",
            Some(&c),
            &[],
            &[],
            Some(&json!({"public": false}))
        ));
        // No metadata → never matches a metadata filter.
        assert!(!is_function_allowed("any::fn", Some(&c), &[], &[], None));
    }

    // --- serde round-trip (the property the engine does NOT have) ----------

    #[test]
    fn rbac_config_round_trips_through_json() {
        let yaml = r#"
            auth_function_id: my-project::auth
            expose_functions:
              - match("api::*")
              - match("*::public")
              - metadata:
                  public: true
                  name: match("*public*")
        "#;
        let parsed: RbacConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.auth_function_id.as_deref(), Some("my-project::auth"));
        assert_eq!(parsed.expose_functions.len(), 3);

        // to_json → from_json must yield an equal config (round-trip).
        let as_json = serde_json::to_value(&parsed).unwrap();
        let back: RbacConfig = serde_json::from_value(as_json).unwrap();
        assert_eq!(parsed, back);
    }

    #[test]
    fn function_filter_serializes_to_operator_form() {
        let f = FunctionFilter::Match(WildcardPattern::new("api::*"));
        assert_eq!(serde_json::to_value(&f).unwrap(), json!("match(\"api::*\")"));
    }

    #[test]
    fn auth_result_defaults_match_engine() {
        let r: AuthResult = serde_json::from_value(json!({})).unwrap();
        assert!(r.allowed_functions.is_empty());
        assert!(r.forbidden_functions.is_empty());
        assert!(r.allowed_trigger_types.is_none());
        assert!(!r.allow_trigger_type_registration); // serde default false
        assert!(r.allow_function_registration); // default true
        assert_eq!(r.context, json!({}));
        assert!(r.function_registration_prefix.is_none());
    }

    #[test]
    fn permissive_session_enables_trigger_type_registration() {
        let s = ProxySession::permissive("1.2.3.4".to_string());
        assert!(s.allow_trigger_type_registration);
        assert!(s.allow_function_registration);
        assert!(s.allowed_trigger_types.is_none());
    }

    #[test]
    fn remediation_branches() {
        let mut s = ProxySession::permissive("ip".into());
        assert_eq!(remediation(&s, "api::x"), "add to rbac.expose_functions");
        s.forbidden_functions = vec!["api::x".into()];
        assert_eq!(
            remediation(&s, "api::x"),
            "remove from rbac.forbidden_functions"
        );
    }
}
