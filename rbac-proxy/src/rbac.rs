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

use iii_sdk::{CallRequest, IIIClient};
use schemars::JsonSchema;
use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// RbacConfig — the operator-facing RBAC block (same field set as the engine's
// WorkerManagerConfig.rbac and the devexp `gateway:` block).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct RbacConfig {
    /// Engine function id invoked once per upgrade to authenticate the
    /// connection. Unset ⇒ every upgrade is refused, except a loopback
    /// peer's when `loopback_auth` is set.
    #[serde(default)]
    pub auth_function_id: Option<String>,
    /// Built-in authentication for local projects, used only when
    /// `auth_function_id` is not set: a connection whose peer address is
    /// loopback (exactly 127.0.0.1, ::1 or ::ffff:127.0.0.1) gets these grants, as if an
    /// auth function had returned them; any other peer is refused. Only safe
    /// where every local process is trusted: a reverse proxy on the same
    /// machine makes every client look local. `auth_function_id` wins when
    /// both are set.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_loopback_auth"
    )]
    pub loopback_auth: Option<LoopbackAuth>,
    /// Filters exposing functions for invocation/discovery; a function is
    /// exposed if **any** filter matches.
    #[serde(default)]
    pub expose_functions: Vec<FunctionFilter>,
    /// Endpoints of the engine's worker port the proxy serves (`/`, `/otel`,
    /// `/ws/channels/*`); one is served if **any** rule matches it. Unset
    /// serves `/` only, as earlier versions did (minus channels), so an
    /// upgrade keeps workers connected; an empty list serves nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expose_endpoints: Option<Vec<crate::endpoints::EndpointFilter>>,
    /// Called before a function is registered, with its `function_id`,
    /// `description`, `metadata`, `namespace`, `private`, `observable`,
    /// `args_format` (when set) and the session's `context`. An object answer
    /// may rewrite `function_id`, `description`, `metadata`, `private`,
    /// `observable` and `args_format`; anything else denies (the registration
    /// is dropped).
    #[serde(default)]
    pub on_function_registration_function_id: Option<String>,
    /// Called before an observer is created, with `observer_id`,
    /// `function_id`, `namespace`, `args`, `filters`, `target_function_id`,
    /// `target_namespace`, `metadata` and the session's `context`. An object
    /// answer may rewrite `function_id`, `target_function_id`, `args`,
    /// `filters` and `metadata`; anything else denies (`FORBIDDEN`).
    #[serde(default)]
    pub on_observer_registration_function_id: Option<String>,
}

impl RbacConfig {
    /// The authentication in force for new connections.
    pub fn auth_mode(&self) -> AuthMode {
        match (&self.auth_function_id, &self.loopback_auth) {
            (Some(_), _) => AuthMode::Function,
            (None, Some(_)) => AuthMode::Loopback,
            (None, None) => AuthMode::None,
        }
    }

    /// The endpoint rules in force ([`crate::endpoints::effective_endpoints`]).
    pub fn endpoints(&self) -> Vec<crate::endpoints::EndpointFilter> {
        crate::endpoints::effective_endpoints(self.expose_endpoints.as_deref())
    }
}

/// Which authentication admits new connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    /// `rbac.auth_function_id` authenticates every connection.
    Function,
    /// No auth function; `rbac.loopback_auth` admits loopback peers only.
    Loopback,
    /// Neither: every connection is refused.
    None,
}

/// `rbac.loopback_auth`: the grants a loopback peer gets when no
/// `auth_function_id` is set. The fields and defaults of an auth function's
/// result ([`AuthResult`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoopbackAuth {
    /// Grants scoped to one namespace each (`{ orders: ["svc::*"] }`); the
    /// keys are then the only namespaces the connection may declare.
    #[serde(default)]
    pub namespaces: HashMap<String, Vec<String>>,
    /// Function ids the connection may call in the `default` namespace
    /// (exact ids; a pattern goes in `namespaces`).
    #[serde(default)]
    pub allowed_functions: Vec<String>,
    /// Function ids the connection may never call, in any namespace; wins
    /// over every other rule.
    #[serde(default)]
    pub forbidden_functions: Vec<String>,
    /// Whether the connection may register functions (event sources
    /// included). Default `false`, unlike an auth function's result; when
    /// `true`, a non-empty `function_registration_prefix` is required.
    #[serde(default)]
    pub allow_function_registration: bool,
    /// Passed to the middleware and the registration hooks.
    #[serde(default = "default_context")]
    pub context: Value,
    /// Private prefix for the connection's own registrations and worker
    /// names. Non-empty and required when `allow_function_registration` is
    /// `true`.
    #[serde(default)]
    pub function_registration_prefix: Option<String>,
    /// Worker names the connection may declare unchanged (exact, never an
    /// `rbac-proxy:` name; one live session at a time holds each).
    #[serde(default)]
    pub worker_names: Vec<String>,
    /// `Origin` values whose upgrades still get the loopback grant (exact,
    /// scheme and host compared case-insensitively, e.g.
    /// `http://localhost:3000`). An upgrade carrying any other `Origin`, a
    /// browser page's, is refused as a non-loopback peer; one with no
    /// `Origin` (SDK workers) is unaffected. Default `[]`: every upgrade
    /// that carries an `Origin` is refused.
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    /// Whether an upgrade carrying a proxy's forwarding headers (`Forwarded`,
    /// `X-Forwarded-For`, `X-Forwarded-Host`, `X-Forwarded-Proto`,
    /// `X-Real-IP` or `Via`) still gets the loopback grant. Default `false`:
    /// such an upgrade came through a proxy, so its client is not known to be
    /// local, and it is refused as a non-loopback peer.
    #[serde(default)]
    pub allow_forwarded: bool,
}

impl Default for LoopbackAuth {
    fn default() -> Self {
        Self {
            namespaces: HashMap::new(),
            allowed_functions: vec![],
            forbidden_functions: vec![],
            allow_function_registration: false,
            context: default_context(),
            function_registration_prefix: None,
            worker_names: vec![],
            allowed_origins: vec![],
            allow_forwarded: false,
        }
    }
}

impl LoopbackAuth {
    /// The rules serde cannot express: registration needs a non-empty
    /// `function_registration_prefix`, so every local process's registrations
    /// stay apart from the engine's own ids.
    pub fn validate(&self) -> Result<(), String> {
        let has_prefix = self
            .function_registration_prefix
            .as_deref()
            .is_some_and(|prefix| !prefix.trim().is_empty());
        if self.allow_function_registration && !has_prefix {
            return Err(
                "rbac.loopback_auth: allow_function_registration: true requires a \
                        non-empty function_registration_prefix"
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// `rbac.loopback_auth`, refused when [`LoopbackAuth::validate`] fails: an
/// invalid grant is a configuration error, like an unknown key.
fn deserialize_loopback_auth<'de, D>(deserializer: D) -> Result<Option<LoopbackAuth>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let grants = Option::<LoopbackAuth>::deserialize(deserializer)?;
    if let Some(grants) = &grants {
        grants.validate().map_err(serde::de::Error::custom)?;
    }
    Ok(grants)
}

impl From<LoopbackAuth> for AuthResult {
    fn from(grants: LoopbackAuth) -> Self {
        Self {
            namespaces: grants.namespaces,
            allowed_functions: grants.allowed_functions,
            forbidden_functions: grants.forbidden_functions,
            allow_function_registration: grants.allow_function_registration,
            context: grants.context,
            function_registration_prefix: grants.function_registration_prefix,
            worker_names: grants.worker_names,
        }
    }
}

/// `true` when `ip_address`, the peer address passed to auth, is loopback:
/// exactly `127.0.0.1`, `::1` or the IPv4-mapped `::ffff:127.0.0.1`. The rest
/// of 127.0.0.0/8 is not: a service-mesh sidecar (Istio connects from
/// `127.0.0.6`) forwards remote clients from such an address. Anything that
/// does not parse as an address is not loopback either.
pub fn is_loopback_peer(ip_address: &str) -> bool {
    ip_address
        .parse::<std::net::IpAddr>()
        .is_ok_and(|ip| match ip.to_canonical() {
            std::net::IpAddr::V4(v4) => v4 == std::net::Ipv4Addr::LOCALHOST,
            std::net::IpAddr::V6(v6) => v6 == std::net::Ipv6Addr::LOCALHOST,
        })
}

/// The forwarding headers a proxy adds: an upgrade carrying one came through
/// a proxy, so its client is not known to be local.
pub const FORWARDING_HEADERS: [&str; 6] = [
    "forwarded",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-proto",
    "x-real-ip",
    "via",
];

/// The value of header `name` in an upgrade's `headers` (any case).
fn header<'a>(headers: &'a HashMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

/// `true` when an upgrade with `headers` shows no sign of a non-local client
/// that `grants` does not allow: an `Origin` outside `allowed_origins`
/// (compared case-insensitively), or a forwarding header without
/// `allow_forwarded`.
fn loopback_headers_allowed(grants: &LoopbackAuth, headers: &HashMap<String, String>) -> bool {
    if let Some(origin) = header(headers, "origin")
        && !grants
            .allowed_origins
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(origin))
    {
        return false;
    }
    grants.allow_forwarded
        || FORWARDING_HEADERS
            .iter()
            .all(|name| header(headers, name).is_none())
}

/// The session `rbac.loopback_auth` grants the peer `ip_address` whose
/// upgrade carried `headers`, built as from an auth function's result.
/// `None` when an `auth_function_id` is set (it wins), when `loopback_auth`
/// is not, when the peer is not loopback, or when the headers show a
/// non-local client (a foreign `Origin`, a forwarding header) the grant does
/// not allow.
pub fn loopback_session(
    rbac: &RbacConfig,
    ip_address: &str,
    headers: &HashMap<String, String>,
) -> Option<ProxySession> {
    if rbac.auth_function_id.is_some() || !is_loopback_peer(ip_address) {
        return None;
    }
    let grants = rbac.loopback_auth.clone()?;
    if !loopback_headers_allowed(&grants, headers) {
        return None;
    }
    Some(AuthResult::from(grants).into_session(ip_address.to_string()))
}

/// Logs one `warn` when both `auth_function_id` and `loopback_auth` are set
/// (the auth function wins); `true` when it did. Called once per applied
/// configuration.
pub fn warn_auth_conflict(rbac: &RbacConfig) -> bool {
    let conflict = rbac.auth_function_id.is_some() && rbac.loopback_auth.is_some();
    if conflict {
        tracing::warn!(
            "rbac.auth_function_id and rbac.loopback_auth are both set; \
             rbac.auth_function_id authenticates every connection and rbac.loopback_auth is ignored"
        );
    }
    conflict
}

/// `rbac` keys that configuration entries stored before 0.26 may set, with
/// the setting that covers each (the engine's own list, `rbac_config.rs`
/// `REMOVED_KEYS`). The proxy ignores them: an entry that sets one loads, with
/// one `warn` per key.
pub const REMOVED_KEYS: [(&str, &str); 2] = [
    (
        "on_trigger_registration_function_id",
        "use on_observer_registration_function_id, which gates every observer",
    ),
    (
        "on_trigger_type_registration_function_id",
        "use on_function_registration_function_id: an event source is a function \
         registered with args_format and private",
    ),
];

/// Remove the [`REMOVED_KEYS`] from a raw `rbac` object before it is parsed
/// (the struct denies unknown fields), logging one `warn` for each key set.
pub fn drop_removed_keys(rbac: &mut Value) {
    let Some(obj) = rbac.as_object_mut() else {
        return;
    };
    for (key, replacement) in REMOVED_KEYS {
        if obj.remove(key).is_some() {
            tracing::warn!(key = %key, "rbac.{key} is ignored: {replacement}");
        }
    }
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

    if let Some(last) = parts.last()
        && !last.is_empty()
        && !value.ends_with(last)
    {
        return false;
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
pub(crate) fn parse_match_pattern(s: &str) -> Option<String> {
    // strip_prefix then strip_suffix: the two cannot share a quote, so
    // `match(")` is no pattern rather than a panicking slice.
    let trimmed = s.trim();
    [("match(\"", "\")"), ("match('", "')")]
        .iter()
        .find_map(|(open, close)| trimmed.strip_prefix(open)?.strip_suffix(close))
        .map(str::to_string)
}

/// VENDORED from rbac_config.rs.
fn parse_metadata_value(value: Value) -> MetadataValue {
    if let Some(s) = value.as_str()
        && let Some(pattern) = parse_match_pattern(s)
    {
        return MetadataValue::Wildcard(WildcardPattern::new(&pattern));
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
// propagation working regardless of `expose_functions`. The six *discovery*
// functions are deliberately NOT here — they are gated by `expose_functions`
// and their results are rewritten (see `engine_overrides`). The carve-out is
// for calling: the interceptor refuses observers of these ids, whose calls
// carry other workers' channel keys, logs and registrations.
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
    /// Whether the connection may register functions, event sources
    /// included (an event source is a function with `args_format`).
    pub allow_function_registration: bool,
    /// Forwarded to middleware + registration hooks.
    pub context: Value,
    /// Private namespace prefix for this session's own registrations.
    pub function_registration_prefix: Option<String>,
    /// Worker names this session may declare unchanged, matched exactly
    /// ([`ProxySession::proxied_worker_name`]).
    pub worker_names: Vec<String>,
}

impl ProxySession {
    /// The permissive default used when no `auth_function_id` is configured —
    /// identical to the engine's no-auth-function branch
    /// (`Session::authenticate` in rbac_session.rs).
    pub fn permissive(ip_address: String) -> Self {
        Self {
            session_id: Uuid::new_v4(),
            ip_address,
            namespaces: HashMap::new(),
            allowed_functions: vec![],
            forbidden_functions: vec![],
            allow_function_registration: true,
            context: json!({}),
            function_registration_prefix: None,
            worker_names: vec![],
        }
    }

    /// Does this session's prefix own `candidate` (`{prefix}::…`)?
    pub fn has_prefix(&self) -> bool {
        self.function_registration_prefix.is_some()
    }

    /// The worker name the engine knows this session's connection by, for
    /// the name it declared on `engine::workers::register`:
    /// `rbac-proxy:<prefix>::<declared>` under a `function_registration_prefix`,
    /// `rbac-proxy:<session id>:<declared>` without one. The engine admits one
    /// live worker per name and namespace, and an `observe` from a connection
    /// under the same name and namespace replaces that worker's suspended
    /// observer (decision 51); the proxy's connections all reach it as one
    /// principal. The prefix scopes a worker's name as it scopes the worker's
    /// function ids: sessions under one prefix share their workers'
    /// identities, and a session under another prefix declaring the same
    /// name is another worker. Nothing else tells two unprefixed sessions
    /// apart, so each one's connection is a worker of its own: another
    /// session declaring the same name neither shuts it out nor takes its
    /// suspended observers. The `rbac-proxy:` start keeps proxied workers off
    /// the names of workers connected to the engine directly.
    ///
    /// A name the auth function listed in `worker_names` is the exception:
    /// it reaches the engine as declared, so a supervisor that waits for the
    /// worker by name (compose) sees it. One live session at a time holds a
    /// granted name: another session declaring it meanwhile gets its own
    /// prefixed name, so two sessions never share one worker identity (or
    /// its suspended observers) through the proxy. A worker connected to the
    /// engine directly under the same name is the operator's to avoid.
    pub fn proxied_worker_name(&self, declared: &str) -> String {
        if self.worker_names.iter().any(|granted| granted == declared)
            && claim_worker_name(declared, self.session_id)
        {
            return declared.to_string();
        }
        match &self.function_registration_prefix {
            Some(prefix) => format!("{WORKER_NAME_PREFIX}{prefix}::{declared}"),
            None => format!("{WORKER_NAME_PREFIX}{}:{declared}", self.session_id),
        }
    }
}

/// Start of every worker name a proxied connection declares
/// ([`ProxySession::proxied_worker_name`]).
pub const WORKER_NAME_PREFIX: &str = "rbac-proxy:";

/// Granted worker names in use, by the session holding each.
static GRANTED_NAMES: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Uuid>>> =
    std::sync::LazyLock::new(Default::default);

/// Take `name` for `session`; `false` when another live session holds it.
fn claim_worker_name(name: &str, session: Uuid) -> bool {
    let mut held = GRANTED_NAMES.lock().unwrap();
    match held.get(name) {
        Some(holder) => *holder == session,
        None => {
            held.insert(name.to_string(), session);
            true
        }
    }
}

/// Frees the session's granted names when dropped (its connection ended).
pub struct WorkerNamesGuard(pub Uuid);

impl Drop for WorkerNamesGuard {
    fn drop(&mut self) {
        release_worker_names(self.0);
    }
}

/// Free every granted name `session` holds; called when its connection ends.
pub fn release_worker_names(session: Uuid) {
    GRANTED_NAMES
        .lock()
        .unwrap()
        .retain(|_, holder| *holder != session);
}

// ---------------------------------------------------------------------------
// AuthResult — VENDORED defaults from rbac_session.rs (`AuthResult`).
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
    #[serde(default = "default_true")]
    pub allow_function_registration: bool,
    #[serde(default = "default_context")]
    pub context: Value,
    #[serde(default)]
    pub function_registration_prefix: Option<String>,
    /// Worker names the session may declare unchanged. Not in the engine's
    /// `AuthResult`: the engine never renames a worker.
    #[serde(default)]
    pub worker_names: Vec<String>,
}

impl AuthResult {
    pub fn into_session(self, ip_address: String) -> ProxySession {
        ProxySession {
            session_id: Uuid::new_v4(),
            ip_address,
            namespaces: self.namespaces,
            allowed_functions: self.allowed_functions,
            forbidden_functions: self.forbidden_functions,
            allow_function_registration: self.allow_function_registration,
            context: self.context,
            function_registration_prefix: self.function_registration_prefix,
            worker_names: granted_worker_names(self.worker_names),
        }
    }
}

/// The granted names a session may use: never one under `rbac-proxy:`, the
/// names the proxy gives sessions (another tenant's worker among them).
fn granted_worker_names(names: Vec<String>) -> Vec<String> {
    names
        .into_iter()
        .filter(|name| {
            let reserved = name.starts_with(WORKER_NAME_PREFIX);
            if reserved {
                tracing::warn!(name = %name, "a worker_names grant names a reserved worker name; ignored");
            }
            !reserved
        })
        .collect()
}

/// Auth result fields that auth functions written before 0.26 may return;
/// the proxy ignores them (the engine's `REMOVED_FIELDS` in rbac_session.rs).
pub const REMOVED_AUTH_FIELDS: [&str; 2] =
    ["allowed_trigger_types", "allow_trigger_type_registration"];

/// Logs one `warn` per process when an auth function's result sets a removed
/// field: registering an event source is registering a function, gated by
/// `allow_function_registration` and the registration hook.
fn warn_removed_auth_fields(result: &Value) {
    static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if REMOVED_AUTH_FIELDS
        .iter()
        .any(|field| result.get(field).is_some())
        && !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed)
    {
        tracing::warn!(
            fields = ?REMOVED_AUTH_FIELDS,
            "the auth function's result sets fields rbac-proxy ignores; function registration \
             (allow_function_registration, on_function_registration_function_id) gates event \
             sources"
        );
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
/// connection (`iii.call`), exactly as the engine invokes it via
/// `engine.call` (`Session::authenticate` in rbac_session.rs). Unset,
/// unresolvable or errored all **fail closed** (reject the upgrade): neither
/// a missing setting nor a broken control plane opens the door. The one
/// exception is `rbac.loopback_auth`: with no auth function set, a loopback
/// peer gets its grants ([`loopback_session`]).
///
/// The auth function receives the upgrade's headers (bearer tokens
/// included), so operators register it with `observable: false`.
pub async fn resolve_session(
    iii: &IIIClient,
    rbac: Option<&RbacConfig>,
    headers: HashMap<String, String>,
    query_params: HashMap<String, Vec<String>>,
    ip_address: String,
    endpoint: &str,
) -> Result<ProxySession, AuthRejection> {
    let Some(auth_fn) = rbac.and_then(|c| c.auth_function_id.as_deref()) else {
        // No auth function: a loopback peer gets `rbac.loopback_auth`'s
        // grants when it is set and its upgrade shows no sign of a non-local
        // client; otherwise it is refused exactly like a remote peer.
        if let Some(session) = rbac.and_then(|c| loopback_session(c, &ip_address, &headers)) {
            return Ok(session);
        }
        // Otherwise refuse (unlike the engine's permissive default).
        // The proxy's stored default entry has none, so a lost override must
        // close the door, not open it.
        return Err(AuthRejection {
            code: "AUTH_ERROR".to_string(),
            message: "rbac.auth_function_id is not set".to_string(),
        });
    };

    // `endpoint` ("/" or "/otel") lets the auth function grant an endpoint
    // per credential, for example /otel to trusted tokens only.
    let input = json!({
        "headers": headers,
        "query_params": query_params,
        "ip_address": ip_address,
        "endpoint": endpoint,
    });

    let result = iii.call(CallRequest::new(auth_fn, input)).await;

    match result {
        Err(e) => Err(AuthRejection {
            code: "AUTH_ERROR".to_string(),
            message: e.to_string(),
        }),
        Ok(v) if v.is_null() => Err(AuthRejection {
            code: "AUTH_ERROR".to_string(),
            message: "auth function returned no result".to_string(),
        }),
        Ok(v) => {
            warn_removed_auth_fields(&v);
            match serde_json::from_value::<AuthResult>(v) {
                Ok(r) => Ok(r.into_session(ip_address)),
                Err(e) => Err(AuthRejection {
                    code: "AUTH_ERROR".to_string(),
                    message: e.to_string(),
                }),
            }
        }
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

    #[test]
    fn a_malformed_match_rule_is_a_config_error_not_a_panic() {
        for bad in ["match(\")", "match(')"] {
            let endpoints =
                serde_json::from_value::<RbacConfig>(json!({ "expose_endpoints": [bad] }));
            assert!(endpoints.is_err(), "expose_endpoints {bad}");
            let functions =
                serde_json::from_value::<RbacConfig>(json!({ "expose_functions": [bad] }));
            assert!(functions.is_err(), "expose_functions {bad}");
        }
    }

    #[test]
    fn a_match_rule_whose_quotes_overlap_is_not_a_pattern() {
        // `match(")` passes both the prefix and the suffix check on one
        // shared quote; slicing it used to panic during deserialization.
        assert_eq!(parse_match_pattern("match(\")"), None);
        assert_eq!(parse_match_pattern("match(')"), None);
        assert_eq!(parse_match_pattern("match(\"\")"), Some(String::new()));
        assert_eq!(parse_match_pattern("match('/otel')"), Some("/otel".into()));
        assert_eq!(
            parse_match_pattern(" match(\"/otel\") "),
            Some("/otel".into())
        );
    }

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
            "engine::observers::list",
            "engine::observers::get",
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
        let parsed: RbacConfig = serde_yaml_ng::from_str(yaml).unwrap();
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
        assert!(r.allow_function_registration); // default true
        assert_eq!(r.context, json!({}));
        assert!(r.function_registration_prefix.is_none());
        assert!(r.worker_names.is_empty());
    }

    #[test]
    fn auth_result_reads_granted_worker_names_into_the_session() {
        let r: AuthResult =
            serde_json::from_value(json!({ "worker_names": ["mcp", "mcp-auth"] })).unwrap();
        assert_eq!(r.worker_names, vec!["mcp", "mcp-auth"]);
        let s = r.into_session("1.2.3.4".to_string());
        assert_eq!(s.worker_names, vec!["mcp", "mcp-auth"]);
    }

    fn granted(prefix: Option<&str>, names: &[&str]) -> ProxySession {
        ProxySession {
            function_registration_prefix: prefix.map(str::to_string),
            worker_names: names.iter().map(|n| n.to_string()).collect(),
            ..ProxySession::permissive("1.2.3.4".to_string())
        }
    }

    #[test]
    fn a_granted_worker_name_reaches_the_engine_as_declared() {
        for prefix in [None, Some("tenant1")] {
            let s = granted(prefix, &["mcp-declared"]);
            assert_eq!(s.proxied_worker_name("mcp-declared"), "mcp-declared");
            release_worker_names(s.session_id);
        }
    }

    #[test]
    fn a_name_the_auth_function_did_not_grant_is_still_pinned() {
        let s = granted(None, &["mcp"]);
        assert_eq!(
            s.proxied_worker_name("queue"),
            format!("rbac-proxy:{}:queue", s.session_id)
        );
        assert_eq!(
            granted(Some("tenant1"), &["mcp"]).proxied_worker_name("queue"),
            "rbac-proxy:tenant1::queue"
        );
    }

    #[test]
    fn granted_names_match_exactly() {
        // No patterns, no case folding: a grant names one worker.
        let s = granted(None, &["mcp"]);
        for declared in ["MCP", "mcp ", "mcp-auth", "m", ""] {
            assert!(
                s.proxied_worker_name(declared)
                    .starts_with(WORKER_NAME_PREFIX)
            );
        }
        assert!(
            granted(None, &["*"])
                .proxied_worker_name("mcp")
                .starts_with(WORKER_NAME_PREFIX)
        );
    }

    #[test]
    fn no_grant_is_the_permissive_default() {
        assert!(
            ProxySession::permissive("1.2.3.4".to_string())
                .worker_names
                .is_empty()
        );
    }

    #[tokio::test]
    async fn no_auth_function_refuses_the_connection() {
        let iii = iii_sdk::register_worker("ws://127.0.0.1:1", iii_sdk::InitOptions::default());
        for rbac in [None, Some(RbacConfig::default())] {
            let refused = resolve_session(
                &iii,
                rbac.as_ref(),
                HashMap::new(),
                HashMap::new(),
                "ip".into(),
                "/",
            )
            .await
            .unwrap_err();
            assert_eq!(refused.code, "AUTH_ERROR");
        }
    }

    #[test]
    fn permissive_session_allows_function_registration() {
        let s = ProxySession::permissive("1.2.3.4".to_string());
        assert!(s.allow_function_registration);
        assert!(s.namespaces.is_empty());
        assert!(s.function_registration_prefix.is_none());
    }

    /// An auth result written before 0.26 parses, and its removed fields are
    /// ignored.
    #[test]
    fn removed_auth_fields_are_ignored() {
        let mut raw = json!({ "allowed_functions": ["api::a"] });
        for field in REMOVED_AUTH_FIELDS {
            raw[field] = json!(true);
        }
        let r: AuthResult = serde_json::from_value(raw).unwrap();
        assert_eq!(r.allowed_functions, vec!["api::a".to_string()]);
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
        let c: RbacConfig = serde_yaml_ng::from_str(yaml).unwrap();
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
        assert!(
            serde_json::from_value::<FunctionFilter>(
                json!({ "match": "api::*", "namesapce": "orders" })
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<FunctionFilter>(
                json!({ "match": "api::*", "metadata": { "public": true } })
            )
            .is_err()
        );
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

    /// A stored `rbac` block that sets a removed key loads once the key is
    /// dropped; every other unknown key is still refused.
    #[test]
    fn removed_keys_are_dropped_before_parsing() {
        let mut raw = json!({ "auth_function_id": "p::auth" });
        for (key, _) in REMOVED_KEYS {
            raw[key] = json!("p::hook");
        }
        assert!(serde_json::from_value::<RbacConfig>(raw.clone()).is_err());
        drop_removed_keys(&mut raw);
        let c: RbacConfig = serde_json::from_value(raw).unwrap();
        assert_eq!(c.auth_function_id.as_deref(), Some("p::auth"));

        let mut typo = json!({ "on_observer_registration_fn": "p::hook" });
        drop_removed_keys(&mut typo);
        assert!(serde_json::from_value::<RbacConfig>(typo).is_err());
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

    #[test]
    fn a_grant_cannot_name_a_proxied_worker() {
        let r: AuthResult = serde_json::from_value(json!({
            "worker_names": ["mcp-server", "rbac-proxy:tenant2::queue", "rbac-proxy:x"]
        }))
        .unwrap();
        assert_eq!(r.into_session("ip".into()).worker_names, vec!["mcp-server"]);
    }

    #[test]
    fn one_live_session_at_a_time_holds_a_granted_name() {
        let grant = |prefix: &str| ProxySession {
            worker_names: vec!["granted-once".to_string()],
            function_registration_prefix: Some(prefix.to_string()),
            ..ProxySession::permissive("ip".into())
        };
        let (a, b) = (grant("tenant1"), grant("tenant2"));
        assert_eq!(a.proxied_worker_name("granted-once"), "granted-once");
        // Declaring it again keeps it.
        assert_eq!(a.proxied_worker_name("granted-once"), "granted-once");
        // Another session gets its own prefixed name while `a` holds it.
        assert_eq!(
            b.proxied_worker_name("granted-once"),
            "rbac-proxy:tenant2::granted-once"
        );
        release_worker_names(a.session_id);
        assert_eq!(b.proxied_worker_name("granted-once"), "granted-once");
        release_worker_names(b.session_id);
    }

    // --- loopback_auth -----------------------------------------------------

    /// The only addresses that count as loopback.
    const LOOPBACK_PEERS: [&str; 3] = ["127.0.0.1", "::1", "::ffff:127.0.0.1"];
    const REMOTE_PEERS: [&str; 17] = [
        // The rest of 127.0.0.0/8 is not loopback here: an Istio sidecar
        // connects from 127.0.0.6 on behalf of remote clients.
        "127.0.0.6",
        "127.5.5.5",
        "127.255.255.254",
        "127.0.0.0",
        "::ffff:127.0.0.2",
        "::ffff:127.9.9.9",
        "0:0:0:0:0:0:0:2",
        "10.0.0.5",
        "192.168.1.2",
        "::ffff:10.0.0.1",
        "fe80::1",
        "0.0.0.0",
        "::",
        "128.0.0.1",
        "2001:db8::1",
        "not-an-ip",
        "",
    ];

    fn loopback_rbac(grants: Value) -> RbacConfig {
        serde_json::from_value(json!({ "loopback_auth": grants })).unwrap()
    }

    async fn resolve_from(rbac: &RbacConfig, ip: &str) -> Result<ProxySession, AuthRejection> {
        let iii = iii_sdk::register_worker("ws://127.0.0.1:1", iii_sdk::InitOptions::default());
        resolve_session(
            &iii,
            Some(rbac),
            HashMap::new(),
            HashMap::new(),
            ip.to_string(),
            "/",
        )
        .await
    }

    #[test]
    fn loopback_auth_is_absent_by_default() {
        assert!(RbacConfig::default().loopback_auth.is_none());
        let parsed: RbacConfig = serde_json::from_value(json!({})).unwrap();
        assert!(parsed.loopback_auth.is_none());
        // An absent key is not written back: stored entries stay as they were.
        let back = serde_json::to_value(RbacConfig::default()).unwrap();
        assert!(back.get("loopback_auth").is_none(), "{back}");
    }

    #[test]
    fn an_empty_loopback_auth_has_the_auth_result_defaults() {
        let grants = loopback_rbac(json!({})).loopback_auth.unwrap();
        assert!(grants.namespaces.is_empty());
        assert!(grants.allowed_functions.is_empty());
        assert!(grants.forbidden_functions.is_empty());
        // Unlike an auth function's result: registration is opt-in.
        assert!(!grants.allow_function_registration);
        assert_eq!(grants.context, json!({}));
        assert!(grants.function_registration_prefix.is_none());
        assert!(grants.worker_names.is_empty());
        assert_eq!(grants, LoopbackAuth::default());
    }

    #[test]
    fn loopback_auth_reads_every_grant_field() {
        let rbac: RbacConfig = serde_yaml_ng::from_str(
            r#"
            loopback_auth:
              namespaces:
                orders: ["svc::*"]
              allowed_functions: [api::a]
              forbidden_functions: [secrets::resolve]
              allow_function_registration: false
              context: { tenant: local }
              function_registration_prefix: local
              worker_names: [mcp-server]
            "#,
        )
        .unwrap();
        let g = rbac.loopback_auth.clone().unwrap();
        assert_eq!(g.namespaces, grants(&[("orders", &["svc::*"])]));
        assert_eq!(g.allowed_functions, vec!["api::a"]);
        assert_eq!(g.forbidden_functions, vec!["secrets::resolve"]);
        assert!(!g.allow_function_registration);
        assert_eq!(g.context, json!({ "tenant": "local" }));
        assert_eq!(g.function_registration_prefix.as_deref(), Some("local"));
        assert_eq!(g.worker_names, vec!["mcp-server"]);
        // And back again.
        let back: RbacConfig =
            serde_json::from_value(serde_json::to_value(&rbac).unwrap()).unwrap();
        assert_eq!(back, rbac);
    }

    async fn resolve_with_headers(
        rbac: &RbacConfig,
        ip: &str,
        headers: &[(&str, &str)],
    ) -> Result<ProxySession, AuthRejection> {
        let iii = iii_sdk::register_worker("ws://127.0.0.1:1", iii_sdk::InitOptions::default());
        let headers = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        resolve_session(
            &iii,
            Some(rbac),
            headers,
            HashMap::new(),
            ip.to_string(),
            "/",
        )
        .await
    }

    /// What a non-loopback peer gets, for comparison.
    async fn refused_like_a_remote_peer(outcome: Result<ProxySession, AuthRejection>, what: &str) {
        let refused = outcome
            .err()
            .unwrap_or_else(|| panic!("{what} was admitted"));
        let remote = resolve_from(&RbacConfig::default(), "10.0.0.5")
            .await
            .unwrap_err();
        assert_eq!(refused.code, remote.code, "{what}");
        assert_eq!(refused.message, remote.message, "{what}");
    }

    #[test]
    fn loopback_auth_origin_and_forwarded_keys_default_closed() {
        let g = loopback_rbac(json!({})).loopback_auth.unwrap();
        assert!(g.allowed_origins.is_empty());
        assert!(!g.allow_forwarded);
        let g = loopback_rbac(json!({
            "allowed_origins": ["http://localhost:3000"],
            "allow_forwarded": true
        }))
        .loopback_auth
        .unwrap();
        assert_eq!(g.allowed_origins, vec!["http://localhost:3000"]);
        assert!(g.allow_forwarded);
    }

    #[tokio::test]
    async fn a_loopback_upgrade_with_a_foreign_origin_is_refused() {
        let rbac = loopback_rbac(json!({ "allowed_origins": ["http://localhost:3000"] }));
        // SDK workers send no Origin: unaffected.
        resolve_with_headers(&rbac, "127.0.0.1", &[("user-agent", "iii-sdk")])
            .await
            .unwrap();
        for origin in [
            "http://evil.example",
            "null",
            "http://localhost:3001",
            "https://localhost:3000",
            "",
        ] {
            let outcome = resolve_with_headers(&rbac, "127.0.0.1", &[("origin", origin)]).await;
            refused_like_a_remote_peer(outcome, origin).await;
        }
        // Listed origins pass, scheme and host compared case-insensitively.
        for origin in ["http://localhost:3000", "HTTP://LocalHost:3000"] {
            resolve_with_headers(&rbac, "::1", &[("origin", origin)])
                .await
                .unwrap_or_else(|e| panic!("{origin} refused: {}", e.message));
        }
        // No list: every Origin is refused.
        let closed = loopback_rbac(json!({}));
        let outcome =
            resolve_with_headers(&closed, "127.0.0.1", &[("origin", "http://localhost:3000")])
                .await;
        refused_like_a_remote_peer(outcome, "origin with an empty allowed_origins").await;
    }

    #[tokio::test]
    async fn a_loopback_upgrade_with_forwarding_headers_is_refused() {
        let closed = loopback_rbac(json!({}));
        let open = loopback_rbac(json!({ "allow_forwarded": true }));
        for header in [
            "forwarded",
            "x-forwarded-for",
            "x-forwarded-host",
            "x-forwarded-proto",
            "x-real-ip",
            "via",
            "X-Forwarded-For",
        ] {
            let outcome = resolve_with_headers(&closed, "127.0.0.1", &[(header, "1.2.3.4")]).await;
            refused_like_a_remote_peer(outcome, header).await;
            resolve_with_headers(&open, "127.0.0.1", &[(header, "1.2.3.4")])
                .await
                .unwrap_or_else(|e| panic!("{header} refused with allow_forwarded: {}", e.message));
        }
        // allow_forwarded does not lift the Origin check.
        let outcome = resolve_with_headers(
            &open,
            "127.0.0.1",
            &[
                ("x-forwarded-for", "1.2.3.4"),
                ("origin", "http://evil.example"),
            ],
        )
        .await;
        refused_like_a_remote_peer(outcome, "forwarded + foreign origin").await;
        // Neither header admits a remote peer.
        let open_origin =
            loopback_rbac(json!({ "allow_forwarded": true, "allowed_origins": ["http://a"] }));
        let outcome =
            resolve_with_headers(&open_origin, "10.0.0.5", &[("origin", "http://a")]).await;
        refused_like_a_remote_peer(outcome, "remote peer").await;
    }

    #[test]
    fn loopback_registration_requires_a_prefix() {
        let parse = |grants: Value| {
            serde_json::from_value::<RbacConfig>(json!({ "loopback_auth": grants }))
        };
        for bad in [
            json!({ "allow_function_registration": true }),
            json!({ "allow_function_registration": true, "function_registration_prefix": null }),
            json!({ "allow_function_registration": true, "function_registration_prefix": "" }),
            json!({ "allow_function_registration": true, "function_registration_prefix": "  " }),
        ] {
            let err = parse(bad.clone()).expect_err(&bad.to_string()).to_string();
            assert!(
                err.contains("function_registration_prefix")
                    && err.contains("allow_function_registration"),
                "{bad}: {err}"
            );
        }
        let ok = parse(
            json!({ "allow_function_registration": true, "function_registration_prefix": "mcp" }),
        )
        .unwrap()
        .loopback_auth
        .unwrap();
        assert!(ok.allow_function_registration);
        assert_eq!(ok.function_registration_prefix.as_deref(), Some("mcp"));
        // Without registration no prefix is needed, and a prefix alone is fine.
        parse(json!({ "allow_function_registration": false })).unwrap();
        parse(json!({ "function_registration_prefix": "mcp" })).unwrap();
        // An auth function's result keeps its own defaults: registration on,
        // no prefix required.
        let r: AuthResult =
            serde_json::from_value(json!({ "allow_function_registration": true })).unwrap();
        assert!(r.allow_function_registration && r.function_registration_prefix.is_none());
        assert!(
            serde_json::from_value::<AuthResult>(json!({}))
                .unwrap()
                .allow_function_registration
        );
    }

    #[test]
    fn loopback_auth_refuses_unknown_keys() {
        // A typo must not silently drop a restriction.
        for raw in [
            json!({ "loopback_auth": { "forbidden_function": ["secrets::resolve"] } }),
            json!({ "loopback_auth": { "auth_function_id": "p::auth" } }),
        ] {
            assert!(
                serde_json::from_value::<RbacConfig>(raw.clone()).is_err(),
                "{raw}"
            );
        }
        for not_an_object in [json!(true), json!("127.0.0.1"), json!(1)] {
            let raw = json!({ "loopback_auth": not_an_object });
            assert!(
                serde_json::from_value::<RbacConfig>(raw.clone()).is_err(),
                "{raw}"
            );
        }
    }

    #[test]
    fn loopback_peers_are_recognised() {
        for ip in LOOPBACK_PEERS {
            assert!(is_loopback_peer(ip), "{ip} is loopback");
        }
        for ip in REMOTE_PEERS {
            assert!(!is_loopback_peer(ip), "{ip} is not loopback");
        }
    }

    #[tokio::test]
    async fn a_loopback_peer_gets_exactly_the_configured_grants() {
        let rbac = loopback_rbac(json!({
            "namespaces": { "orders": ["svc::*"] },
            "allowed_functions": ["api::a"],
            "forbidden_functions": ["secrets::resolve"],
            "allow_function_registration": false,
            "context": { "tenant": "local" },
            "function_registration_prefix": "local",
            "worker_names": ["mcp-server"]
        }));
        let mut ids = std::collections::HashSet::new();
        for ip in LOOPBACK_PEERS {
            let s = resolve_from(&rbac, ip)
                .await
                .unwrap_or_else(|e| panic!("{ip} refused: {}", e.message));
            assert_eq!(s.ip_address, ip);
            assert_eq!(s.namespaces, grants(&[("orders", &["svc::*"])]));
            assert_eq!(s.allowed_functions, vec!["api::a"]);
            assert_eq!(s.forbidden_functions, vec!["secrets::resolve"]);
            assert!(!s.allow_function_registration);
            assert_eq!(s.context, json!({ "tenant": "local" }));
            assert_eq!(s.function_registration_prefix.as_deref(), Some("local"));
            assert_eq!(s.worker_names, vec!["mcp-server"]);
            // Each connection is a session of its own.
            assert!(ids.insert(s.session_id));
        }
    }

    #[tokio::test]
    async fn an_empty_loopback_grant_is_the_empty_auth_result() {
        let s = resolve_from(&loopback_rbac(json!({})), "127.0.0.1")
            .await
            .unwrap();
        let expected = serde_json::from_value::<AuthResult>(json!({}))
            .unwrap()
            .into_session("127.0.0.1".to_string());
        assert_eq!(s.namespaces, expected.namespaces);
        assert_eq!(s.allowed_functions, expected.allowed_functions);
        assert_eq!(s.forbidden_functions, expected.forbidden_functions);
        // The one difference: an empty loopback grant registers nothing.
        assert!(expected.allow_function_registration);
        assert!(!s.allow_function_registration);
        assert_eq!(s.context, expected.context);
        assert_eq!(
            s.function_registration_prefix,
            expected.function_registration_prefix
        );
        assert_eq!(s.worker_names, expected.worker_names);
    }

    #[tokio::test]
    async fn a_remote_peer_is_refused_as_without_loopback_auth() {
        let with = loopback_rbac(json!({ "worker_names": ["mcp-server"] }));
        let without = RbacConfig::default();
        for ip in REMOTE_PEERS {
            let refused = resolve_from(&with, ip).await.unwrap_err();
            let today = resolve_from(&without, ip).await.unwrap_err();
            assert_eq!(refused.code, "AUTH_ERROR", "{ip}");
            assert_eq!(refused.code, today.code, "{ip}");
            assert_eq!(refused.message, today.message, "{ip}");
        }
    }

    #[tokio::test]
    async fn without_loopback_auth_a_loopback_peer_is_still_refused() {
        for ip in LOOPBACK_PEERS {
            let refused = resolve_from(&RbacConfig::default(), ip).await.unwrap_err();
            assert_eq!(refused.code, "AUTH_ERROR");
            assert_eq!(refused.message, "rbac.auth_function_id is not set");
        }
    }

    #[test]
    fn the_loopback_grant_applies_only_without_an_auth_function() {
        let mut rbac = loopback_rbac(json!({ "worker_names": ["mcp-server"] }));
        assert!(loopback_session(&rbac, "127.0.0.1", &HashMap::new()).is_some());
        assert!(loopback_session(&rbac, "10.0.0.5", &HashMap::new()).is_none());
        rbac.auth_function_id = Some("p::auth".to_string());
        for ip in LOOPBACK_PEERS.into_iter().chain(REMOTE_PEERS) {
            assert!(
                loopback_session(&rbac, ip, &HashMap::new()).is_none(),
                "{ip}"
            );
        }
        assert!(loopback_session(&RbacConfig::default(), "127.0.0.1", &HashMap::new()).is_none());
    }

    #[tokio::test]
    async fn an_auth_function_wins_over_loopback_auth() {
        // The auth function is called (and here fails: nothing answers on the
        // control connection), the loopback grant is never used.
        let mut rbac = loopback_rbac(json!({}));
        rbac.auth_function_id = Some("p::auth".to_string());
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            resolve_from(&rbac, "127.0.0.1"),
        )
        .await;
        assert!(
            !matches!(outcome, Ok(Ok(_))),
            "loopback_auth admitted a connection while auth_function_id is set"
        );
    }

    #[test]
    fn both_auth_keys_set_is_warned_about() {
        let mut rbac = loopback_rbac(json!({}));
        assert!(!warn_auth_conflict(&rbac));
        rbac.auth_function_id = Some("p::auth".to_string());
        assert!(warn_auth_conflict(&rbac));
        // Once per applied configuration, so again for the next one.
        assert!(warn_auth_conflict(&rbac));
        rbac.loopback_auth = None;
        assert!(!warn_auth_conflict(&rbac));
        assert!(!warn_auth_conflict(&RbacConfig::default()));
    }

    #[test]
    fn the_auth_mode_names_what_admits_connections() {
        let mut rbac = RbacConfig::default();
        assert_eq!(rbac.auth_mode(), AuthMode::None);
        rbac.loopback_auth = Some(LoopbackAuth::default());
        assert_eq!(rbac.auth_mode(), AuthMode::Loopback);
        rbac.auth_function_id = Some("p::auth".to_string());
        assert_eq!(rbac.auth_mode(), AuthMode::Function);
        rbac.loopback_auth = None;
        assert_eq!(rbac.auth_mode(), AuthMode::Function);
        assert_eq!(
            serde_json::to_value([AuthMode::Function, AuthMode::Loopback, AuthMode::None]).unwrap(),
            json!(["function", "loopback", "none"])
        );
    }

    #[test]
    fn loopback_worker_names_follow_the_auth_function_rules() {
        let rbac = loopback_rbac(json!({
            "worker_names": ["loopback-granted", "rbac-proxy:tenant2::queue", "rbac-proxy:x"]
        }));
        // The reserved `rbac-proxy:` names are dropped, as from an auth function.
        let a = loopback_session(&rbac, "127.0.0.1", &HashMap::new()).unwrap();
        assert_eq!(a.worker_names, vec!["loopback-granted"]);
        // A granted name passes as declared; another is pinned.
        assert_eq!(
            a.proxied_worker_name("loopback-granted"),
            "loopback-granted"
        );
        assert_eq!(
            a.proxied_worker_name("queue"),
            format!("rbac-proxy:{}:queue", a.session_id)
        );
        // One live holder: a second loopback session gets its own name.
        let b = loopback_session(&rbac, "::1", &HashMap::new()).unwrap();
        assert_eq!(
            b.proxied_worker_name("loopback-granted"),
            format!("rbac-proxy:{}:loopback-granted", b.session_id)
        );
        release_worker_names(a.session_id);
        assert_eq!(
            b.proxied_worker_name("loopback-granted"),
            "loopback-granted"
        );
        release_worker_names(b.session_id);
    }

    #[test]
    fn loopback_grants_are_enforced_like_an_auth_functions() {
        let mut rbac = loopback_rbac(json!({
            "forbidden_functions": ["secrets::resolve"],
            "namespaces": { "orders": ["svc::*"] }
        }));
        rbac.expose_functions = vec![FunctionFilter::match_pattern("*")];
        let s = loopback_session(&rbac, "127.0.0.1", &HashMap::new()).unwrap();
        assert!(access_allowed(Some(&rbac), &s, "default", "api::x", None));
        assert!(!access_allowed(
            Some(&rbac),
            &s,
            "default",
            "secrets::resolve",
            None
        ));
        assert!(access_allowed(Some(&rbac), &s, "orders", "svc::a", None));
        assert!(!access_allowed(
            Some(&rbac),
            &s,
            "orders",
            "secrets::resolve",
            None
        ));
        assert_eq!(
            remediation(&s, "secrets::resolve"),
            "remove from rbac.forbidden_functions"
        );
    }
}
