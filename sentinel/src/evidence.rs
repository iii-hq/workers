//! The frozen evidence.
//!
//! The engine's span store is a ring of ten thousand spans: by the time a
//! person opens a group, the trace that produced it is usually gone. So the
//! bundle is built at capture and stored — it is the whole reason this worker
//! exists rather than a saved query over the traces page.
//!
//! Three filters shape what goes in. Attributes are kept by an allowlist, so
//! a worker that stamps something unexpected onto a span cannot widen what
//! this stores. Values are capped, because one `db.statement` should not
//! crowd out the rest of the trace. And the bundle as a whole has a byte
//! budget, spent on the spans nearest the failure: when a trace is too big,
//! what goes first is what is furthest from the origin.

use std::collections::{BTreeMap, HashMap, VecDeque};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Redactor;

pub const EVIDENCE_VERSION: u32 = 1;

/// Attribute prefixes worth keeping on a span.
const KEPT_PREFIXES: [&str; 8] = [
    "iii.",
    "exception.",
    "code.",
    "error.",
    "rpc.",
    "http.",
    "db.statement",
    "tool.",
];

/// Kept by exact name rather than prefix.
const KEPT_EXACT: [&str; 1] = ["function_id"];

/// Dropped even though `http.` is kept: request headers are where credentials
/// live, and a header allowlist would be a second denylist to maintain.
const DROPPED_PREFIXES: [&str; 1] = ["http.request.header."];

const VALUE_MAX_BYTES: usize = 4 * 1024;
/// Tool payloads are the one attribute family that is routinely enormous.
const TOOL_VALUE_MAX_BYTES: usize = 1024;
const LOG_LIMIT: usize = 200;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceTruncatedV1 {
    pub spans: u32,
    pub logs: u32,
    pub attributes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceWorkerV1 {
    pub service_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceEventV1 {
    pub name: String,
    pub timestamp_unix_nano: u64,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceSpanV1 {
    pub span_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    pub name: String,
    pub service_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function_id: Option<String>,
    pub start_time_unix_nano: u64,
    /// Zero while the span is still open — the engine's own convention for a
    /// pending span, kept rather than normalized so a half-finished trace
    /// reads as one.
    pub end_time_unix_nano: u64,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_description: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub events: Vec<EvidenceEventV1>,
    pub depth: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceLogV1 {
    pub timestamp_unix_nano: u64,
    pub severity_text: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceBundleV1 {
    pub version: u32,
    pub captured_at_ms: i64,
    /// True once the settle pass re-read the trace: ancestors that were still
    /// open at capture are closed here.
    pub settled: bool,
    pub trace_id: String,
    /// The leaf error span this bundle is about.
    pub origin_span_id: String,
    /// Ancestors that carry the same failure upward. They are context, not
    /// groups of their own.
    pub propagated_through: Vec<String>,
    pub trace_tags: BTreeMap<String, String>,
    pub spans: Vec<EvidenceSpanV1>,
    pub logs: Vec<EvidenceLogV1>,
    pub worker: EvidenceWorkerV1,
    pub truncated: EvidenceTruncatedV1,
}

impl EvidenceBundleV1 {
    /// Drop spans until the bundle fits, furthest from the origin first.
    ///
    /// Distance is measured through the tree rather than by depth, so a
    /// sibling subtree goes before the failure's own parent — the ancestors
    /// are the story of how the error propagated and are the last thing worth
    /// losing.
    pub fn fit_within(&mut self, max_bytes: usize) {
        if self.encoded_len() <= max_bytes {
            return;
        }
        let order = self.drop_order();
        for span_id in order {
            if self.encoded_len() <= max_bytes || self.spans.len() <= 1 {
                break;
            }
            if span_id == self.origin_span_id {
                continue;
            }
            self.spans.retain(|span| span.span_id != span_id);
            self.truncated.spans += 1;
        }
        // Logs are cheaper to lose than the shape of the trace, but if the
        // spans alone blow the budget the logs go too.
        while self.encoded_len() > max_bytes && !self.logs.is_empty() {
            self.logs.pop();
            self.truncated.logs += 1;
        }
    }

    fn encoded_len(&self) -> usize {
        serde_json::to_string(self)
            .map(|json| json.len())
            .unwrap_or(0)
    }

    /// Span ids furthest from the origin first.
    fn drop_order(&self) -> Vec<String> {
        let mut neighbours: HashMap<&str, Vec<&str>> = HashMap::new();
        for span in &self.spans {
            if let Some(parent) = span.parent_span_id.as_deref() {
                neighbours.entry(parent).or_default().push(&span.span_id);
                neighbours.entry(&span.span_id).or_default().push(parent);
            }
        }

        let mut distance: HashMap<&str, usize> = HashMap::new();
        let mut queue = VecDeque::new();
        distance.insert(self.origin_span_id.as_str(), 0);
        queue.push_back(self.origin_span_id.as_str());
        while let Some(current) = queue.pop_front() {
            let step = distance[current] + 1;
            for next in neighbours.get(current).into_iter().flatten() {
                if !distance.contains_key(next) {
                    distance.insert(next, step);
                    queue.push_back(next);
                }
            }
        }

        let mut ordered: Vec<(usize, String)> = self
            .spans
            .iter()
            .map(|span| {
                (
                    // A span the walk never reached is unattached to the
                    // origin: the first thing to lose.
                    distance
                        .get(span.span_id.as_str())
                        .copied()
                        .unwrap_or(usize::MAX),
                    span.span_id.clone(),
                )
            })
            .collect();
        ordered.sort_by_key(|(distance, _)| std::cmp::Reverse(*distance));
        ordered.into_iter().map(|(_, span_id)| span_id).collect()
    }
}

/// Keep, drop, or cap one attribute.
pub fn keep_attribute(key: &str, extra_allowlist: &[String]) -> Option<usize> {
    if Redactor::is_sensitive_key(key) {
        return None;
    }
    if DROPPED_PREFIXES
        .iter()
        .any(|prefix| key.starts_with(prefix))
    {
        return None;
    }
    let kept = KEPT_EXACT.contains(&key)
        || KEPT_PREFIXES.iter().any(|prefix| key.starts_with(prefix))
        || extra_allowlist.iter().any(|allowed| allowed == key);
    if !kept {
        return None;
    }
    Some(if key.starts_with("tool.") {
        TOOL_VALUE_MAX_BYTES
    } else {
        VALUE_MAX_BYTES
    })
}

/// Filter, cap and redact a set of attributes, counting what was dropped.
pub fn filter_attributes<I, K, V>(
    attributes: I,
    extra_allowlist: &[String],
    redactor: &Redactor,
    truncated: &mut EvidenceTruncatedV1,
    redactions: &mut u64,
) -> BTreeMap<String, String>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: Into<String>,
{
    let mut kept = BTreeMap::new();
    for (key, value) in attributes {
        let key = key.as_ref();
        let Some(cap) = keep_attribute(key, extra_allowlist) else {
            truncated.attributes += 1;
            continue;
        };
        let value: String = value.into();
        let (value, hits) = redactor.redact(&value);
        *redactions += hits;
        kept.insert(key.to_string(), cap_bytes(value, cap));
    }
    kept
}

/// Truncate on a character boundary, marking that it happened. The marker is
/// part of the budget, not added on top of it: a cap that can be exceeded is
/// not a cap.
fn cap_bytes(value: String, max: usize) -> String {
    const MARKER: &str = "…";
    if value.len() <= max {
        return value;
    }
    let mut end = max.saturating_sub(MARKER.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{MARKER}", &value[..end])
}

/// The upper bound on logs carried with a bundle.
pub const fn log_limit() -> usize {
    LOG_LIMIT
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(id: &str, parent: Option<&str>, depth: u32) -> EvidenceSpanV1 {
        EvidenceSpanV1 {
            span_id: id.into(),
            parent_span_id: parent.map(str::to_string),
            name: format!("span {id}"),
            service_name: "harness".into(),
            function_id: None,
            start_time_unix_nano: 1,
            end_time_unix_nano: 2,
            status: "ok".into(),
            status_description: None,
            attributes: BTreeMap::new(),
            events: Vec::new(),
            depth,
        }
    }

    fn bundle(spans: Vec<EvidenceSpanV1>, origin: &str) -> EvidenceBundleV1 {
        EvidenceBundleV1 {
            version: EVIDENCE_VERSION,
            captured_at_ms: 1_789_000_000_000,
            settled: false,
            trace_id: "t1".into(),
            origin_span_id: origin.into(),
            propagated_through: Vec::new(),
            trace_tags: BTreeMap::new(),
            spans,
            logs: Vec::new(),
            worker: EvidenceWorkerV1 {
                service_name: "harness".into(),
                version: Some("0.23.0".into()),
            },
            truncated: EvidenceTruncatedV1::default(),
        }
    }

    #[test]
    fn the_allowlist_keeps_diagnosis_and_drops_everything_else() {
        let extra = vec!["billing.plan".to_string()];
        assert!(keep_attribute("function_id", &extra).is_some());
        assert!(keep_attribute("exception.type", &extra).is_some());
        assert!(keep_attribute("iii.session.id", &extra).is_some());
        assert!(keep_attribute("db.statement", &extra).is_some());
        assert!(keep_attribute("http.status_code", &extra).is_some());
        assert!(keep_attribute("billing.plan", &extra).is_some());

        assert!(keep_attribute("customer.email", &extra).is_none());
        assert!(
            keep_attribute("http.request.header.authorization", &extra).is_none(),
            "request headers are where credentials live"
        );
        assert!(keep_attribute("api_key", &extra).is_none());
    }

    #[test]
    fn tool_payloads_get_a_tighter_cap_than_everything_else() {
        assert_eq!(keep_attribute("tool.arguments", &[]), Some(1024));
        assert_eq!(keep_attribute("db.statement", &[]), Some(4096));
    }

    #[test]
    fn filtering_redacts_caps_and_counts_what_it_dropped() {
        let redactor = Redactor::default();
        let mut truncated = EvidenceTruncatedV1::default();
        let mut redactions = 0;
        let kept = filter_attributes(
            [
                ("function_id", "state::compare-and-set"),
                ("db.statement", "connect postgres://u:p@h/db"),
                ("customer.email", "someone@example.com"),
                ("authorization", "Bearer abc"),
            ],
            &[],
            &redactor,
            &mut truncated,
            &mut redactions,
        );

        assert_eq!(kept.len(), 2);
        assert_eq!(
            truncated.attributes, 2,
            "two keys were not on the allowlist"
        );
        assert_eq!(redactions, 1, "the kept statement carried a credential");
        assert!(!kept["db.statement"].contains("u:p@h"), "{kept:?}");
    }

    #[test]
    fn a_long_value_is_cut_on_a_character_boundary_and_marked() {
        let mut truncated = EvidenceTruncatedV1::default();
        let mut redactions = 0;
        let kept = filter_attributes(
            [("tool.arguments", "ç".repeat(2_000))],
            &[],
            &Redactor::default(),
            &mut truncated,
            &mut redactions,
        );
        let value = &kept["tool.arguments"];
        assert!(value.len() <= 1024, "{} bytes", value.len());
        assert!(value.ends_with('…'));
    }

    #[test]
    fn a_bundle_within_budget_is_left_alone() {
        let mut bundle = bundle(vec![span("a", None, 0), span("b", Some("a"), 1)], "b");
        bundle.fit_within(10_000);
        assert_eq!(bundle.spans.len(), 2);
        assert_eq!(bundle.truncated.spans, 0);
    }

    #[test]
    fn over_budget_the_furthest_spans_go_and_the_origins_ancestors_stay() {
        // root ── parent ── origin
        //   └───── far ──── further
        let mut bundle = bundle(
            vec![
                span("root", None, 0),
                span("parent", Some("root"), 1),
                span("origin", Some("parent"), 2),
                span("far", Some("root"), 1),
                span("further", Some("far"), 2),
            ],
            "origin",
        );
        let budget = serde_json::to_string(&bundle).unwrap().len() - 200;
        bundle.fit_within(budget);

        let kept: Vec<&str> = bundle.spans.iter().map(|s| s.span_id.as_str()).collect();
        assert!(
            kept.contains(&"origin"),
            "the failure itself is never dropped"
        );
        assert!(
            kept.contains(&"parent"),
            "the propagation path is the last thing worth losing: {kept:?}"
        );
        assert!(
            !kept.contains(&"further"),
            "the furthest span goes first: {kept:?}"
        );
        assert!(bundle.truncated.spans > 0);
    }

    #[test]
    fn an_impossible_budget_keeps_the_origin_rather_than_emptying_the_bundle() {
        let mut bundle = bundle(
            vec![span("root", None, 0), span("origin", Some("root"), 1)],
            "origin",
        );
        bundle.fit_within(1);
        assert_eq!(bundle.spans.len(), 1);
        assert_eq!(bundle.spans[0].span_id, "origin");
    }
}
