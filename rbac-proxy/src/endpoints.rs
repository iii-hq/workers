//! Which engine endpoints the proxy serves: `rbac.expose_endpoints`.
//!
//! The engine's worker port has three WebSocket endpoints: `/` (the worker
//! protocol), `/ws/channels/{id}` (channels) and `/otel` (telemetry). The
//! proxy serves a path only when a rule matches it; with no rules it serves
//! nothing. Rules are the `expose_functions` wildcards over the request path:
//! `match("/otel")` or `{ match: "/otel" }`, `*` matching any run of
//! characters, anchored at both ends, case-sensitive.
//!
//! How an exposed path is handled ([`Endpoint::of`]):
//! - `/`: authenticated, then every frame goes through the interceptor;
//! - `/ws/channels/{id}`: relayed as is; the engine checks the channel's
//!   `access_key`, as it does for direct connections;
//! - anything else (`/otel`): authenticated with the same auth function as
//!   `/`, then relayed as is, as the engine does for `/otel` itself.

use schemars::JsonSchema;
use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::json;

use crate::rbac::{parse_match_pattern, WildcardPattern};

/// The prefix of a channel endpoint; the id is the one segment after it.
const CHANNEL_PREFIX: &str = "/ws/channels/";

/// One `expose_endpoints` rule: a wildcard over the request path.
#[derive(Debug, Clone, PartialEq)]
pub struct EndpointFilter(WildcardPattern);

impl EndpointFilter {
    /// A rule for `pattern`: a path starting with `/`, or `*`.
    pub fn new(pattern: &str) -> Result<Self, String> {
        if pattern == "*" || pattern.starts_with('/') {
            Ok(Self(WildcardPattern::new(pattern)))
        } else {
            Err(format!(
                "an expose_endpoints pattern is a path starting with '/' (or *), got: {pattern:?}"
            ))
        }
    }

    pub fn pattern(&self) -> &str {
        self.0.raw()
    }

    pub fn matches(&self, path: &str) -> bool {
        self.0.matches(path)
    }
}

/// Whether any rule exposes `path`. No rules expose nothing.
pub fn endpoint_exposed(rules: &[EndpointFilter], path: &str) -> bool {
    rules.iter().any(|rule| rule.matches(path))
}

/// How the proxy handles an exposed path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// `/`: the worker protocol, through the interceptor.
    Worker,
    /// `/ws/channels/{id}`: relayed; the engine checks the access key.
    Channel(String),
    /// Anything else: authenticated, then relayed.
    Relay,
}

impl Endpoint {
    pub fn of(path: &str) -> Self {
        if path == "/" {
            return Self::Worker;
        }
        match path.strip_prefix(CHANNEL_PREFIX) {
            Some(id) if !id.is_empty() && !id.contains('/') => Self::Channel(id.to_string()),
            _ => Self::Relay,
        }
    }
}

/// The rule that would expose `path`: one rule covers every channel.
pub fn suggested_rule(path: &str) -> String {
    let pattern = if matches!(Endpoint::of(path), Endpoint::Channel(_)) {
        "/ws/channels/*"
    } else {
        path
    };
    format!("match(\"{pattern}\")")
}

/// The body of the 403 for a path no rule exposes.
pub fn not_exposed_message(path: &str) -> String {
    format!(
        "rbac-proxy: endpoint {path} is not exposed. Add {} to rbac.expose_endpoints.",
        suggested_rule(path)
    )
}

/// The warning for a configuration that exposes no endpoint at all.
pub fn empty_warning(rules: &[EndpointFilter]) -> Option<&'static str> {
    rules.is_empty().then_some(
        "rbac.expose_endpoints is empty, so every connection is refused. Add \
         match(\"/\") for workers, match(\"/ws/channels/*\") for channels and \
         match(\"/otel\") for telemetry.",
    )
}

/// `{engine_url}{path}?{query}`, for relaying an exposed path.
pub fn upstream_url(engine_url: &str, path: &str, query: Option<&str>) -> String {
    let mut url = format!("{}{path}", engine_url.trim_end_matches('/'));
    if let Some(q) = query.filter(|q| !q.is_empty()) {
        url.push('?');
        url.push_str(q);
    }
    url
}

impl<'de> Deserialize<'de> for EndpointFilter {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RuleVisitor;

        impl<'de> Visitor<'de> for RuleVisitor {
            type Value = EndpointFilter;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a match(\"/path\") string or a map with a 'match' key")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                let pattern = parse_match_pattern(v).ok_or_else(|| {
                    E::custom(format!(
                        "expected match(\"pattern\"), got: {v}; write match(\"{v}\")"
                    ))
                })?;
                EndpointFilter::new(&pattern).map_err(E::custom)
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut pattern: Option<String> = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key == "match" {
                        pattern = Some(map.next_value()?);
                    } else {
                        let _: serde_json::Value = map.next_value()?;
                        return Err(de::Error::custom(format!(
                            "unknown key '{key}' in an expose_endpoints rule; expected 'match'"
                        )));
                    }
                }
                let pattern = pattern
                    .ok_or_else(|| de::Error::custom("an expose_endpoints rule needs 'match'"))?;
                EndpointFilter::new(&pattern).map_err(de::Error::custom)
            }
        }

        deserializer.deserialize_any(RuleVisitor)
    }
}

impl Serialize for EndpointFilter {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("match(\"{}\")", self.pattern()))
    }
}

impl JsonSchema for EndpointFilter {
    fn schema_name() -> String {
        "EndpointFilter".to_string()
    }

    fn is_referenceable() -> bool {
        false
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        serde_json::from_value(json!({
            "anyOf": [
                {
                    "type": "string",
                    "description": "Path wildcard, e.g. match(\"/otel\") or match(\"/ws/channels/*\"). `*` matches any run of characters; anchored at both ends."
                },
                {
                    "type": "object",
                    "properties": { "match": { "type": "string" } },
                    "required": ["match"],
                    "additionalProperties": false
                }
            ]
        }))
        .expect("EndpointFilter schema is valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rules(v: serde_json::Value) -> Vec<EndpointFilter> {
        serde_json::from_value(v).expect("valid rules")
    }

    #[test]
    fn rules_parse_from_the_expose_functions_forms() {
        let r = rules(json!(["match(\"/\")", "match('/otel')", { "match": "/ws/channels/*" }]));
        assert_eq!(
            r.iter().map(EndpointFilter::pattern).collect::<Vec<_>>(),
            vec!["/", "/otel", "/ws/channels/*"]
        );
    }

    #[test]
    fn a_bare_path_is_refused_with_the_fix() {
        let err = serde_json::from_value::<Vec<EndpointFilter>>(json!(["/otel"]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("match(\"/otel\")"), "{err}");
    }

    #[test]
    fn unknown_keys_and_relative_patterns_are_refused() {
        for bad in [
            json!([{ "match": "/otel", "namespace": "x" }]),
            json!([{ "metadata": {} }]),
            json!([{}]),
            json!(["match(\"otel\")"]),
            json!([{ "match": "" }]),
        ] {
            assert!(
                serde_json::from_value::<Vec<EndpointFilter>>(bad.clone()).is_err(),
                "{bad}"
            );
        }
        // `*` alone is allowed: it exposes every endpoint.
        assert!(serde_json::from_value::<Vec<EndpointFilter>>(json!(["match(\"*\")"])).is_ok());
    }

    #[test]
    fn rules_serialize_back_to_match_strings() {
        let r = rules(json!([{ "match": "/otel" }, "match(\"/\")"]));
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            json!(["match(\"/otel\")", "match(\"/\")"])
        );
    }

    #[test]
    fn nothing_is_exposed_without_rules() {
        for path in ["/", "/otel", "/ws/channels/abc"] {
            assert!(!endpoint_exposed(&[], path), "{path}");
        }
    }

    #[test]
    fn rules_are_anchored_wildcards_over_the_path() {
        let r = rules(json!(["match(\"/\")", "match(\"/ws/channels/*\")"]));
        assert!(endpoint_exposed(&r, "/"));
        assert!(endpoint_exposed(&r, "/ws/channels/abc"));
        assert!(!endpoint_exposed(&r, "/otel"));
        assert!(!endpoint_exposed(&r, "/ws/channels"));
        assert!(!endpoint_exposed(&r, "/OTEL"));
        let all = rules(json!(["match(\"*\")"]));
        for path in ["/", "/otel", "/ws/channels/abc", "/anything"] {
            assert!(endpoint_exposed(&all, path), "{path}");
        }
    }

    #[test]
    fn each_path_is_handled_by_its_kind() {
        assert_eq!(Endpoint::of("/"), Endpoint::Worker);
        assert_eq!(
            Endpoint::of("/ws/channels/abc"),
            Endpoint::Channel("abc".into())
        );
        // A channel id is one segment; anything else is relayed as is.
        assert_eq!(Endpoint::of("/ws/channels/a/b"), Endpoint::Relay);
        assert_eq!(Endpoint::of("/ws/channels/"), Endpoint::Relay);
        assert_eq!(Endpoint::of("/otel"), Endpoint::Relay);
        assert_eq!(Endpoint::of("/anything"), Endpoint::Relay);
    }

    #[test]
    fn the_refusal_names_the_rule_to_add() {
        let m = not_exposed_message("/otel");
        assert!(
            m.contains("rbac.expose_endpoints") && m.contains("match(\"/otel\")"),
            "{m}"
        );
    }

    #[test]
    fn the_refusal_suggests_one_rule_for_all_channels() {
        assert_eq!(
            suggested_rule("/ws/channels/abc"),
            "match(\"/ws/channels/*\")"
        );
        assert_eq!(suggested_rule("/otel"), "match(\"/otel\")");
        assert_eq!(suggested_rule("/"), "match(\"/\")");
    }

    #[test]
    fn an_empty_list_warns_with_the_rules_to_add() {
        let w = empty_warning(&[]).expect("a warning");
        for rule in [
            "match(\"/\")",
            "match(\"/ws/channels/*\")",
            "match(\"/otel\")",
        ] {
            assert!(w.contains(rule), "{w}");
        }
        assert!(empty_warning(&rules(json!(["match(\"/\")"]))).is_none());
    }

    #[test]
    fn the_upstream_url_keeps_path_and_query() {
        assert_eq!(upstream_url("ws://e:1", "/otel", None), "ws://e:1/otel");
        assert_eq!(
            upstream_url("ws://e:1/", "/otel", Some("")),
            "ws://e:1/otel"
        );
        assert_eq!(
            upstream_url("ws://e:1/", "/ws/channels/c", Some("key=k&dir=read")),
            "ws://e:1/ws/channels/c?key=k&dir=read"
        );
    }
}
