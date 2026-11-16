//! Which engine endpoints the proxy serves: `rbac.expose_endpoints`.
//!
//! The engine's worker port has three WebSocket endpoints: `/` (the worker
//! protocol), `/ws/channels/{id}` (channels) and `/otel` (telemetry), and the
//! proxy serves those three and nothing else ([`Endpoint::of`]). Of them it
//! serves a path only when a rule matches it. Rules are the
//! `expose_functions` wildcards over the request path: `match("/otel")` or
//! `{ match: "/otel" }`, `*` matching any run of characters, anchored at
//! both ends, case-sensitive. Without the key it serves `/` only (what
//! earlier versions served, minus channels); an empty list serves nothing.
//!
//! How an exposed endpoint is handled:
//! - `/`: authenticated, then every frame goes through the interceptor;
//! - `/ws/channels/{id}`: relayed as is; the engine checks the channel's
//!   `access_key`, as it does for direct connections;
//! - `/otel`: authenticated before the upgrade with the same auth function
//!   as `/` (told the endpoint), then relayed as is.

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

/// The engine endpoints the proxy can serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// `/`: the worker protocol, through the interceptor.
    Worker,
    /// `/otel`: authenticated, then relayed.
    Telemetry,
    /// `/ws/channels/{id}`: relayed; the engine checks the access key.
    Channel(String),
}

impl Endpoint {
    /// The endpoint at `path`, or `None`: any other path is not one, whatever
    /// the rules match, so an engine route added later stays closed until
    /// the proxy learns to handle it.
    pub fn of(path: &str) -> Option<Self> {
        match path {
            "/" => Some(Self::Worker),
            "/otel" => Some(Self::Telemetry),
            _ => match path.strip_prefix(CHANNEL_PREFIX) {
                Some(id) if !id.is_empty() && !id.contains('/') && id != "." && id != ".." => {
                    Some(Self::Channel(id.to_string()))
                }
                _ => None,
            },
        }
    }
}

/// The rule that would expose `path`: one rule covers every channel.
pub fn suggested_rule(path: &str) -> String {
    let pattern = if matches!(Endpoint::of(path), Some(Endpoint::Channel(_))) {
        "/ws/channels/*"
    } else {
        path
    };
    format!("match(\"{pattern}\")")
}

/// The log line for an endpoint no rule exposes. The client gets a bare 404:
/// the hint is for the operator, not for whoever is probing.
pub fn not_exposed_hint(path: &str) -> String {
    format!(
        "endpoint {path} is not exposed; add {} to rbac.expose_endpoints to serve it",
        suggested_rule(path)
    )
}

/// The rules in force: the configured ones, or `match("/")` without the key.
pub fn effective_endpoints(rules: Option<&[EndpointFilter]>) -> Vec<EndpointFilter> {
    match rules {
        Some(rules) => rules.to_vec(),
        None => vec![EndpointFilter::new("/").expect("a valid rule")],
    }
}

/// The warning for a missing key (only `/` served) or an empty list
/// (nothing served).
pub fn endpoints_warning(rules: Option<&[EndpointFilter]>) -> Option<&'static str> {
    match rules {
        None => Some(
            "rbac.expose_endpoints is not set, so only the worker endpoint is served, as if it \
             were [match(\"/\")]. Set it to serve channels (match(\"/ws/channels/*\")) or \
             telemetry (match(\"/otel\")) as well.",
        ),
        Some([]) => Some(
            "rbac.expose_endpoints is empty, so every connection is refused. Add \
             match(\"/\") for workers, match(\"/ws/channels/*\") for channels and \
             match(\"/otel\") for telemetry.",
        ),
        Some(_) => None,
    }
}

/// `{engine_url}{path}?{query}`, for relaying an exposed endpoint. A query on
/// `engine_url` itself stays a query, merged with the request's.
pub fn upstream_url(engine_url: &str, path: &str, query: Option<&str>) -> String {
    let (base, own) = match engine_url.split_once('?') {
        Some((base, own)) => (base, Some(own)),
        None => (engine_url, None),
    };
    let mut url = format!("{}{path}", base.trim_end_matches('/'));
    let queries: Vec<&str> = [own, query]
        .into_iter()
        .flatten()
        .filter(|q| !q.is_empty())
        .collect();
    if !queries.is_empty() {
        url.push('?');
        url.push_str(&queries.join("&"));
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
    fn the_refusal_suggests_one_rule_for_all_channels() {
        assert_eq!(
            suggested_rule("/ws/channels/abc"),
            "match(\"/ws/channels/*\")"
        );
        assert_eq!(suggested_rule("/otel"), "match(\"/otel\")");
        assert_eq!(suggested_rule("/"), "match(\"/\")");
    }

    #[test]
    fn only_the_engines_three_endpoints_exist() {
        assert_eq!(Endpoint::of("/"), Some(Endpoint::Worker));
        assert_eq!(Endpoint::of("/otel"), Some(Endpoint::Telemetry));
        assert_eq!(
            Endpoint::of("/ws/channels/abc"),
            Some(Endpoint::Channel("abc".into()))
        );
        // Anything else is not an endpoint, whatever the rules say.
        for path in [
            "/ws/channels/a/b",
            "/ws/channels/",
            "/ws/channels/.",
            "/ws/channels/..",
            "/ws/channels/x/../../",
            "/ws/channels/abc/ack",
            "/otel/v2",
            "/ws",
            "/ws/admin",
            "//",
            "/anything",
        ] {
            assert_eq!(Endpoint::of(path), None, "{path}");
        }
    }

    #[test]
    fn the_log_hint_names_the_rule_to_add() {
        let m = not_exposed_hint("/otel");
        assert!(
            m.contains("rbac.expose_endpoints") && m.contains("match(\"/otel\")"),
            "{m}"
        );
    }

    #[test]
    fn a_missing_key_serves_the_worker_endpoint_and_says_so() {
        let served = effective_endpoints(None);
        assert!(endpoint_exposed(&served, "/"));
        assert!(!endpoint_exposed(&served, "/otel"));
        assert!(!endpoint_exposed(&served, "/ws/channels/abc"));
        let w = endpoints_warning(None).expect("a warning");
        assert!(w.contains("not set") && w.contains("match(\"/\")"), "{w}");
    }

    #[test]
    fn an_empty_list_serves_nothing_and_warns_with_the_rules_to_add() {
        assert!(effective_endpoints(Some(&[])).is_empty());
        let w = endpoints_warning(Some(&[])).expect("a warning");
        for rule in [
            "match(\"/\")",
            "match(\"/ws/channels/*\")",
            "match(\"/otel\")",
        ] {
            assert!(w.contains(rule), "{w}");
        }
        assert!(endpoints_warning(Some(&rules(json!(["match(\"/\")"])))).is_none());
    }

    #[test]
    fn the_upstream_url_keeps_path_and_query() {
        assert_eq!(upstream_url("ws://e:1", "/otel", None), "ws://e:1/otel");
        assert_eq!(upstream_url("ws://e:1/", "/otel", None), "ws://e:1/otel");
        assert_eq!(
            upstream_url("ws://e:1", "/ws/channels/c", Some("key=k&dir=read")),
            "ws://e:1/ws/channels/c?key=k&dir=read"
        );
    }

    #[test]
    fn the_engine_urls_own_query_stays_a_query() {
        assert_eq!(
            upstream_url("ws://e:1/?token=t", "/otel", None),
            "ws://e:1/otel?token=t"
        );
        assert_eq!(
            upstream_url("ws://e:1?token=t", "/ws/channels/c", Some("key=k&dir=read")),
            "ws://e:1/ws/channels/c?token=t&key=k&dir=read"
        );
    }
}
