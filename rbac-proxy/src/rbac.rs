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
//! Every decision is keyed by `(namespace, function_id)`, as in the engine
//! since namespaces became a routing dimension: an `expose_functions` rule
//! answers in the namespace it names (`default` when it names none), the auth
//! function's `namespaces` grants answer in their own namespace, and
//! `allowed_functions` answers in `default` only.
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
    /// The engine's observer registration hook (iii 0.25). Accepted so an
    /// engine `rbac` block can be copied here unchanged, but not called yet:
    /// this version of the proxy refuses every `observe` (see the
    /// interceptor), so there is no registration for the hook to rewrite.
    #[serde(default)]
    pub on_observer_registration_function_id: Option<String>,
}

impl RbacConfig {
    /// `true` when any `expose_functions` filter is a metadata filter. Callers
    /// use this to skip the catalog lookup (and its control-connection round
    /// trip) entirely for wildcard-only configs — a wildcard filter ignores
    /// metadata, so a cold cache must not fail it closed.
    pub fn uses_metadata(&self) -> bool {
        self.expose_functions
            .iter()
            .any(FunctionFilter::is_metadata)
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

/// The namespace a request lands in when it names none — the engine's
/// `DEFAULT_NAMESPACE`.
pub const DEFAULT_NAMESPACE: &str = "default";

#[derive(Debug, Clone, PartialEq)]
enum FilterMatcher {
    Match(WildcardPattern),
    Metadata(HashMap<String, MetadataValue>),
}

/// One `expose_functions` rule: a matcher plus an optional namespace scope.
///
/// VENDORED from rbac_config.rs. The namespace is optional and additive. When
/// present, the rule applies only to that namespace; when absent it applies to
/// the `default` namespace only — a rule without a namespace does NOT match a
/// function reached in another namespace.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionFilter {
    matcher: FilterMatcher,
    namespace: Option<String>,
}

impl FunctionFilter {
    /// A wildcard-id rule scoped to the `default` namespace. Chain
    /// [`FunctionFilter::in_namespace`] to scope it elsewhere.
    pub fn match_pattern(pattern: &str) -> Self {
        Self {
            matcher: FilterMatcher::Match(WildcardPattern::new(pattern)),
            namespace: None,
        }
    }

    /// A metadata rule scoped to the `default` namespace. Chain
    /// [`FunctionFilter::in_namespace`] to scope it elsewhere.
    pub fn metadata(map: HashMap<String, MetadataValue>) -> Self {
        Self {
            matcher: FilterMatcher::Metadata(map),
            namespace: None,
        }
    }

    /// Scope this rule to `namespace` instead of the `default` one.
    pub fn in_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }

    /// `true` for a metadata rule (it needs the function's registered
    /// metadata, which the proxy reads from its catalog cache).
    pub fn is_metadata(&self) -> bool {
        matches!(self.matcher, FilterMatcher::Metadata(_))
    }

    /// VENDORED from rbac_config.rs. Whether this rule exposes `function_id`
    /// reached in `namespace`. A function with no metadata never matches a
    /// metadata rule; metadata keys are AND'd within a rule.
    pub fn matches(&self, function_id: &str, namespace: &str, metadata: Option<&Value>) -> bool {
        let rule_ns = self.namespace.as_deref().unwrap_or(DEFAULT_NAMESPACE);
        if rule_ns != namespace {
            return false;
        }
        match &self.matcher {
            FilterMatcher::Match(pattern) => pattern.matches(function_id),
            FilterMatcher::Metadata(expected) => {
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

/// VENDORED from rbac_config.rs.
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

/// VENDORED from rbac_config.rs.
fn parse_metadata_value(value: Value) -> MetadataValue {
    if let Some(s) = value.as_str() {
        if let Some(pattern) = parse_match_pattern(s) {
            return MetadataValue::Wildcard(WildcardPattern::new(&pattern));
        }
    }
    MetadataValue::Exact(value)
}

/// VENDORED from rbac_config.rs — accepts a `match("pattern")` string, or a map
/// with a `match` (raw pattern) or a `metadata` key and an optional
/// `namespace`. Unknown keys are refused: a typo like `namesapce` would
/// otherwise leave the rule scoped to `default` without a word.
impl<'de> Deserialize<'de> for FunctionFilter {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FilterVisitor;

        impl<'de> Visitor<'de> for FilterVisitor {
            type Value = FunctionFilter;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str(
                    "a match(\"pattern\") string or a map with a 'match' or 'metadata' key",
                )
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if let Some(pattern) = parse_match_pattern(v) {
                    Ok(FunctionFilter::match_pattern(&pattern))
                } else {
                    Err(de::Error::custom(format!(
                        "expected match(\"pattern\"), got: {v} — exact ids need it too: match(\"{v}\")"
                    )))
                }
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut metadata_map: HashMap<String, MetadataValue> = HashMap::new();
                let mut match_pattern: Option<WildcardPattern> = None;
                let mut namespace: Option<String> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "metadata" => {
                            let inner: HashMap<String, Value> = map.next_value()?;
                            for (k, v) in inner {
                                metadata_map.insert(k, parse_metadata_value(v));
                            }
                        }
                        // Map form of a wildcard rule, e.g. `{ match: "svc::*" }`.
                        // The raw pattern is taken as-is (no `match("...")` wrapper).
                        "match" => {
                            let raw: String = map.next_value()?;
                            match_pattern = Some(WildcardPattern::new(&raw));
                        }
                        "namespace" => {
                            namespace = Some(map.next_value()?);
                        }
                        other => {
                            let _: Value = map.next_value()?;
                            return Err(de::Error::custom(format!(
                                "unknown key '{other}' in expose_functions filter; \
                                 expected 'match', 'metadata', or 'namespace'"
                            )));
                        }
                    }
                }

                let matcher = match (match_pattern, metadata_map.is_empty()) {
                    (Some(pattern), true) => FilterMatcher::Match(pattern),
                    (None, false) => FilterMatcher::Metadata(metadata_map),
                    (Some(_), false) => {
                        return Err(de::Error::custom(
                            "a filter has either a 'match' or a 'metadata' key, not both",
                        ));
                    }
                    (None, true) => {
                        return Err(de::Error::custom(
                            "expected a 'match' or 'metadata' key with filter conditions",
                        ));
                    }
                };
                Ok(FunctionFilter { matcher, namespace })
            }
        }

        deserializer.deserialize_any(FilterVisitor)
    }
}

/// Symmetric with [`FunctionFilter`]'s `Deserialize`: a namespace-less `match`
/// rule serializes to the `match("…")` string form; a metadata rule, or any
/// rule with a namespace, to the map form (`{ "match": "…" }` /
/// `{ "metadata": { … } }`, plus `"namespace"`).
/// The engine's `RbacConfig` does not round-trip (it is deserialize-only), but
/// the proxy persists its config through the `configuration` worker and must
/// re-read what it stored, so the canonical serialized form is the
/// operator-facing one.
impl Serialize for FunctionFilter {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match (&self.matcher, &self.namespace) {
            (FilterMatcher::Match(p), None) => {
                serializer.serialize_str(&format!("match(\"{}\")", p.raw))
            }
            (matcher, namespace) => {
                let mut m = serializer.serialize_map(None)?;
                match matcher {
                    FilterMatcher::Match(p) => m.serialize_entry("match", p.raw())?,
                    FilterMatcher::Metadata(map) => {
                        let mut inner = serde_json::Map::new();
                        for (k, v) in map {
                            inner.insert(k.clone(), v.to_json());
                        }
                        m.serialize_entry("metadata", &Value::Object(inner))?;
                    }
                }
                if let Some(namespace) = namespace {
                    m.serialize_entry("namespace", namespace)?;
                }
                m.end()
            }
        }
    }
}

/// A typed-enough JSON Schema for the `configuration` worker and the manifest:
/// a `match("…")` string, or an object with a `match` pattern or a `metadata`
/// map and an optional `namespace`. (schemars
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
                    "description": "Wildcard match scoped to a namespace (`default` when omitted), e.g. { match: \"api::*\", namespace: \"orders\" }.",
                    "properties": {
                        "match": { "type": "string" },
                        "namespace": { "type": "string" }
                    },
                    "required": ["match"],
                    "additionalProperties": false
                },
                {
                    "type": "object",
                    "description": "Metadata filter: all keys must match (AND). String values may be a match(\"…\") wildcard. Scoped to `namespace` (`default` when omitted).",
                    "properties": {
                        "metadata": { "type": "object" },
                        "namespace": { "type": "string" }
                    },
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

/// Whether one scoped grant covers `function_id`. VENDORED from rbac_config.rs.
///
/// A grant is an exact id or a wildcard. Both spellings of the wildcard are
/// accepted: bare (`svc::*`) and wrapped (`match("svc::*")`).
fn grant_matches(grant: &str, function_id: &str) -> bool {
    let raw = parse_match_pattern(grant).unwrap_or_else(|| grant.to_string());
    raw == function_id || (raw.contains('*') && WildcardPattern::new(&raw).matches(function_id))
}

/// VENDORED from rbac_config.rs (`is_function_allowed_ref`), with the engine's
/// `function: Option<&Function>` replaced by `metadata: Option<&Value>` (the
/// proxy reads the function's registered metadata from its catalog cache
/// rather than from an in-process `Function`). The decision flow is unchanged:
///
/// 1. `forbidden` → deny (global: a denial holds in every namespace)
/// 2. a `namespaces` grant naming `namespace` → allow
/// 3. `allowed` (no namespace) → allow, in `default` only
/// 4. infrastructure carve-out → allow
/// 5. an `expose_functions` rule scoped to `namespace` matches → allow
/// 6. otherwise → deny
pub fn is_function_allowed(
    function_id: &str,
    namespace: &str,
    config: Option<&RbacConfig>,
    session_namespaces: &HashMap<String, Vec<String>>,
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

    // A grant that names this namespace, by exact id or by pattern.
    if session_namespaces.get(namespace).is_some_and(|granted| {
        granted
            .iter()
            .any(|grant| grant_matches(grant, function_id))
    }) {
        return true;
    }

    // A grant that names no namespace answers in `default` and nowhere else.
    if namespace == DEFAULT_NAMESPACE && allowed_functions.iter().any(|f| f == function_id) {
        return true;
    }

    if INFRASTRUCTURE_FUNCTIONS.contains(&function_id) {
        return true;
    }

    if let Some(config) = config {
        config
            .expose_functions
            .iter()
            .any(|filter| filter.matches(function_id, namespace, metadata))
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
    /// Grants scoped to one namespace each: `{ "orders": ["svc::*"] }`. The
    /// keys are also the namespaces the connection may declare on
    /// `engine::workers::register`; an empty map leaves it free to declare
    /// any (the engine's pre-namespace behaviour).
    pub namespaces: HashMap<String, Vec<String>>,
    /// Grants that name no namespace. They apply in `default` only.
    pub allowed_functions: Vec<String>,
    /// Denials, global on purpose (deny wins in every namespace).
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
            namespaces: HashMap::new(),
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
    pub namespaces: HashMap<String, Vec<String>>,
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
            namespaces: self.namespaces,
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

/// `is_function_allowed` bound to a session + the live `rbac` config snapshot,
/// for `function_id` reached in `namespace`.
pub fn access_allowed(
    rbac: Option<&RbacConfig>,
    session: &ProxySession,
    namespace: &str,
    function_id: &str,
    metadata: Option<&Value>,
) -> bool {
    is_function_allowed(
        function_id,
        namespace,
        rbac,
        &session.namespaces,
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
                .map(|p| FunctionFilter::match_pattern(p))
                .collect(),
            ..RbacConfig::default()
        }
    }

    /// The pre-namespace call shape: `default`, no scoped grants.
    fn allowed_in_default(
        function_id: &str,
        config: Option<&RbacConfig>,
        allowed: &[String],
        forbidden: &[String],
        metadata: Option<&Value>,
    ) -> bool {
        is_function_allowed(
            function_id,
            DEFAULT_NAMESPACE,
            config,
            &HashMap::new(),
            allowed,
            forbidden,
            metadata,
        )
    }

    #[test]
    fn forbidden_takes_precedence_over_allowed_and_expose() {
        let c = cfg(&["*"]);
        let allowed = vec!["test::fn".to_string()];
        let forbidden = vec!["test::fn".to_string()];
        assert!(!allowed_in_default(
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
        assert!(allowed_in_default(
            "test::fn",
            Some(&c),
            &allowed,
            &[],
            None
        ));
    }

    #[test]
    fn carve_out_always_allowed_and_respects_forbidden() {
        for id in INFRASTRUCTURE_FUNCTIONS {
            let c = cfg(&[]);
            assert!(
                allowed_in_default(id, Some(&c), &[], &[], None),
                "carve-out {id} should be allowed with empty expose"
            );
            let c = cfg(&[]);
            let forbidden = vec![id.to_string()];
            assert!(
                !allowed_in_default(id, Some(&c), &[], &forbidden, None),
                "carve-out {id} should be denied when forbidden"
            );
        }
    }

    #[test]
    fn deny_by_default() {
        let c = cfg(&["api::*"]);
        assert!(!allowed_in_default(
            "internal::fn",
            Some(&c),
            &[],
            &[],
            None
        ));
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
                !allowed_in_default(id, Some(&c), &[], &[], None),
                "discovery id {id} must be gated by expose_functions, not carved out"
            );
        }
        // …but reachable when exposed.
        let c = cfg(&["engine::functions::*"]);
        assert!(allowed_in_default(
            "engine::functions::list",
            Some(&c),
            &[],
            &[],
            None
        ));
    }

    #[test]
    fn no_rbac_config_allows_everything() {
        for id in [
            "api::anything",
            "internal::private",
            "engine::workers::list",
        ] {
            assert!(allowed_in_default(id, None, &[], &[], None));
        }
    }

    #[test]
    fn metadata_filter_matches_registered_metadata() {
        let mut meta = HashMap::new();
        meta.insert("public".to_string(), MetadataValue::Exact(json!(true)));
        let c = RbacConfig {
            expose_functions: vec![FunctionFilter::metadata(meta)],
            ..RbacConfig::default()
        };
        assert!(allowed_in_default(
            "any::fn",
            Some(&c),
            &[],
            &[],
            Some(&json!({"public": true}))
        ));
        assert!(!allowed_in_default(
            "any::fn",
            Some(&c),
            &[],
            &[],
            Some(&json!({"public": false}))
        ));
        // No metadata → never matches a metadata filter.
        assert!(!allowed_in_default("any::fn", Some(&c), &[], &[], None));
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
        let f = FunctionFilter::match_pattern("api::*");
        assert_eq!(
            serde_json::to_value(&f).unwrap(),
            json!("match(\"api::*\")")
        );
    }

    #[test]
    fn auth_result_defaults_match_engine() {
        let r: AuthResult = serde_json::from_value(json!({})).unwrap();
        assert!(r.namespaces.is_empty());
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

    // --- Namespace parity (the fixtures from rbac_config.rs) ---------------

    fn grants(pairs: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
        pairs
            .iter()
            .map(|(ns, ids)| {
                (
                    ns.to_string(),
                    ids.iter().map(|id| id.to_string()).collect(),
                )
            })
            .collect()
    }

    fn denying_everything() -> RbacConfig {
        // Nothing gets through on the operator's config alone; whatever passes
        // below passed on the session's grants.
        cfg(&["nothing::matches"])
    }

    fn allowed_in(
        namespace: &str,
        function_id: &str,
        config: &RbacConfig,
        scoped: &HashMap<String, Vec<String>>,
        allowed: &[String],
        forbidden: &[String],
    ) -> bool {
        is_function_allowed(
            function_id,
            namespace,
            Some(config),
            scoped,
            allowed,
            forbidden,
            None,
        )
    }

    #[test]
    fn scoped_grant_answers_in_its_own_namespace_only() {
        let scoped = grants(&[("orders", &["svc::get"])]);
        let c = denying_everything();
        assert!(allowed_in("orders", "svc::get", &c, &scoped, &[], &[]));
        assert!(!allowed_in("analytics", "svc::get", &c, &scoped, &[], &[]));
    }

    #[test]
    fn scoped_grant_accepts_wildcards_and_exact_ids_in_either_spelling() {
        let c = denying_everything();
        for grant in ["svc::*", "match(\"svc::*\")"] {
            let scoped = grants(&[("orders", &[grant])]);
            assert!(allowed_in("orders", "svc::get", &c, &scoped, &[], &[]));
            assert!(!allowed_in("orders", "other::get", &c, &scoped, &[], &[]));
        }
        for grant in ["svc::get", "match(\"svc::get\")"] {
            let scoped = grants(&[("orders", &[grant])]);
            assert!(allowed_in("orders", "svc::get", &c, &scoped, &[], &[]));
        }
    }

    #[test]
    fn a_namespace_key_alone_grants_nothing() {
        let scoped = grants(&[("orders", &[])]);
        assert!(!allowed_in(
            "orders",
            "svc::get",
            &denying_everything(),
            &scoped,
            &[],
            &[]
        ));
    }

    #[test]
    fn unscoped_allowed_functions_apply_in_default_only() {
        let allowed = vec!["svc::get".to_string()];
        let c = denying_everything();
        let none = HashMap::new();
        assert!(allowed_in(
            DEFAULT_NAMESPACE,
            "svc::get",
            &c,
            &none,
            &allowed,
            &[]
        ));
        assert!(!allowed_in("orders", "svc::get", &c, &none, &allowed, &[]));
    }

    #[test]
    fn forbidden_stays_global_across_namespaces() {
        let scoped = grants(&[("orders", &["svc::get"])]);
        let forbidden = vec!["svc::get".to_string()];
        for namespace in [DEFAULT_NAMESPACE, "orders", "analytics"] {
            assert!(
                !allowed_in(
                    namespace,
                    "svc::get",
                    &denying_everything(),
                    &scoped,
                    &[],
                    &forbidden
                ),
                "a denial must hold in {namespace} too"
            );
        }
    }

    #[test]
    fn infrastructure_stays_reachable_from_a_scoped_namespace() {
        let scoped = grants(&[("orders", &[])]);
        for id in INFRASTRUCTURE_FUNCTIONS {
            assert!(
                allowed_in("orders", id, &denying_everything(), &scoped, &[], &[]),
                "expected {id} to stay reachable in a scoped namespace"
            );
        }
    }

    #[test]
    fn expose_filter_without_namespace_matches_only_default() {
        let c = cfg(&["svc::*"]);
        let none = HashMap::new();
        assert!(allowed_in(
            DEFAULT_NAMESPACE,
            "svc::get",
            &c,
            &none,
            &[],
            &[]
        ));
        assert!(!allowed_in("orders", "svc::get", &c, &none, &[], &[]));
    }

    #[test]
    fn expose_filter_with_namespace_matches_only_that_namespace() {
        let c = RbacConfig {
            expose_functions: vec![FunctionFilter::match_pattern("svc::*").in_namespace("orders")],
            ..RbacConfig::default()
        };
        let none = HashMap::new();
        assert!(allowed_in("orders", "svc::get", &c, &none, &[], &[]));
        assert!(!allowed_in(
            DEFAULT_NAMESPACE,
            "svc::get",
            &c,
            &none,
            &[],
            &[]
        ));
        assert!(!allowed_in("analytics", "svc::get", &c, &none, &[], &[]));
    }

    #[test]
    fn function_filter_round_trips_through_serde() {
        let mut map = HashMap::new();
        map.insert(
            "public".to_string(),
            MetadataValue::Exact(Value::Bool(true)),
        );
        map.insert(
            "scope".to_string(),
            MetadataValue::Wildcard(WildcardPattern::new("team-*")),
        );
        let filter = FunctionFilter::metadata(map).in_namespace("team");
        let back: FunctionFilter =
            serde_json::from_value(serde_json::to_value(&filter).unwrap()).unwrap();
        assert_eq!(back, filter);
        let meta = json!({ "public": true, "scope": "team-alpha" });
        assert!(back.matches("svc::get", "team", Some(&meta)));
        assert!(!back.matches("svc::get", DEFAULT_NAMESPACE, Some(&meta)));

        // A namespaced `match` rule uses the map form and keeps its scope.
        let m = FunctionFilter::match_pattern("api::*").in_namespace("orders");
        let as_json = serde_json::to_value(&m).unwrap();
        assert_eq!(as_json, json!({ "match": "api::*", "namespace": "orders" }));
        let mb: FunctionFilter = serde_json::from_value(as_json).unwrap();
        assert!(mb.matches("api::x", "orders", None));
        assert!(!mb.matches("api::x", DEFAULT_NAMESPACE, None));
    }

    #[test]
    fn function_filter_parses_the_engine_map_forms() {
        let yaml = r#"
            expose_functions:
              - match: "api::*"
                namespace: orders
              - metadata:
                  public: true
                namespace: billing
        "#;
        let c: RbacConfig = serde_yaml::from_str(yaml).unwrap();
        let none = HashMap::new();
        assert!(allowed_in("orders", "api::x", &c, &none, &[], &[]));
        assert!(!allowed_in(
            DEFAULT_NAMESPACE,
            "api::x",
            &c,
            &none,
            &[],
            &[]
        ));
        assert!(is_function_allowed(
            "any::fn",
            "billing",
            Some(&c),
            &none,
            &[],
            &[],
            Some(&json!({ "public": true }))
        ));
        assert!(c.uses_metadata());
    }

    #[test]
    fn function_filter_rejects_unknown_keys_and_ambiguous_maps() {
        // A typo must not silently widen or narrow the rule.
        assert!(serde_json::from_value::<FunctionFilter>(
            json!({ "match": "api::*", "namesapce": "orders" })
        )
        .is_err());
        assert!(serde_json::from_value::<FunctionFilter>(
            json!({ "match": "api::*", "metadata": { "public": true } })
        )
        .is_err());
        assert!(serde_json::from_value::<FunctionFilter>(json!({})).is_err());
    }

    #[test]
    fn auth_result_reads_scoped_namespaces() {
        let r: AuthResult =
            serde_json::from_value(json!({ "namespaces": { "orders": ["svc::*"] } })).unwrap();
        let s = r.into_session("ip".into());
        assert_eq!(
            s.namespaces.get("orders"),
            Some(&vec!["svc::*".to_string()])
        );
    }

    #[test]
    fn observer_registration_hook_is_accepted() {
        let c: RbacConfig = serde_json::from_value(
            json!({ "on_observer_registration_function_id": "p::on-observe" }),
        )
        .unwrap();
        assert_eq!(
            c.on_observer_registration_function_id.as_deref(),
            Some("p::on-observe")
        );
    }
}
