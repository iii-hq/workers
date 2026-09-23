//! The span adapter: a trace tree in, failures out.
//!
//! Two rules here decide what becomes a group, and both are about *not*
//! creating them. A failure propagates upward — `execute shell::run` fails,
//! the turn step fails, the root fails — so only **leaf** error spans become
//! events and the ancestors ride along as context. And a trace whose **root**
//! is this worker is dropped whole, because the error the Sentinel causes
//! surfaces in the worker it called (a failing `database::execute` is a
//! `database` span, not a `sentinel` one), so excluding by span would leave
//! the loop wide open.

use std::collections::{BTreeMap, HashSet};

use serde_json::Value;

use crate::adapters::{function_of, AttributeSource, ErrorEvent};
use crate::evidence::{
    filter_attributes, log_limit, EvidenceBundleV1, EvidenceEventV1, EvidenceLogV1, EvidenceSpanV1,
    EvidenceTruncatedV1, EvidenceWorkerV1, EVIDENCE_VERSION,
};
use crate::{ids, ErrorSourceV1, Redactor};

/// Why a whole trace was not ingested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exclusion {
    /// The root belongs to this worker: ingesting it would feed itself.
    OwnTrace,
    /// The trace is an investigation this worker started.
    Investigation,
    /// The operator asked for this service to be left alone.
    IgnoredService,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanEvent {
    pub name: String,
    pub timestamp_unix_nano: u64,
    pub attributes: Vec<(String, String)>,
}

/// One span of a trace, with the depth the tree gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanNode {
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub service_name: String,
    pub status: String,
    pub status_description: Option<String>,
    pub start_time_unix_nano: u64,
    pub end_time_unix_nano: u64,
    pub attributes: Vec<(String, String)>,
    pub events: Vec<SpanEvent>,
    pub depth: u32,
}

impl AttributeSource for SpanNode {
    fn attribute(&self, key: &str) -> Option<String> {
        self.attributes
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    }
}

impl SpanNode {
    pub fn is_error(&self) -> bool {
        self.status.eq_ignore_ascii_case("error")
    }

    /// Engine plumbing rather than a worker's own work.
    pub fn is_internal(&self) -> bool {
        if self
            .attribute("iii.function.kind")
            .is_some_and(|kind| kind == "internal")
        {
            return true;
        }
        self.function_id()
            .is_some_and(|function| function.starts_with("engine::"))
    }

    pub fn function_id(&self) -> Option<String> {
        function_of(&self.name, self)
    }

    /// When the failure happened: the close, or the start while it is open.
    pub fn at_ms(&self) -> i64 {
        let nanos = if self.end_time_unix_nano > 0 {
            self.end_time_unix_nano
        } else {
            self.start_time_unix_nano
        };
        (nanos / 1_000_000) as i64
    }

    fn exception(&self) -> Option<&SpanEvent> {
        self.events.iter().find(|event| event.name == "exception")
    }

    fn exception_attribute(&self, key: &str) -> Option<String> {
        self.exception()?
            .attributes
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    }
}

/// Flatten the `roots` of `engine::traces::tree` into depth-tagged spans.
pub fn flatten(roots: &[Value]) -> Vec<SpanNode> {
    let mut spans = Vec::new();
    for root in roots {
        push_node(root, 0, &mut spans);
    }
    spans
}

fn push_node(node: &Value, depth: u32, out: &mut Vec<SpanNode>) {
    let Some(span_id) = node.get("span_id").and_then(Value::as_str) else {
        return;
    };
    out.push(SpanNode {
        span_id: span_id.to_string(),
        parent_span_id: node
            .get("parent_span_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        name: node
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        service_name: node
            .get("service_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        status: node
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unset")
            .to_string(),
        status_description: node
            .get("status_description")
            .and_then(Value::as_str)
            .map(str::to_string),
        start_time_unix_nano: node
            .get("start_time_unix_nano")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        end_time_unix_nano: node
            .get("end_time_unix_nano")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        attributes: pairs(node.get("attributes")),
        events: node
            .get("events")
            .and_then(Value::as_array)
            .map(|events| {
                events
                    .iter()
                    .map(|event| SpanEvent {
                        name: event
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        timestamp_unix_nano: event
                            .get("timestamp_unix_nano")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                        attributes: pairs(event.get("attributes")),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        depth,
    });
    for child in node
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        push_node(child, depth + 1, out);
    }
}

/// Span attributes arrive as an array of `[key, value]` pairs.
fn pairs(value: Option<&Value>) -> Vec<(String, String)> {
    value
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let pair = entry.as_array()?;
                    let key = pair.first()?.as_str()?;
                    let value = pair.get(1)?;
                    let value = value
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| value.to_string());
                    Some((key.to_string(), value))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What the exclusion rules need to know about the trace and the operator's
/// wishes.
pub struct TraceContext<'a> {
    pub trace_id: &'a str,
    pub trace_tags: &'a BTreeMap<String, String>,
    /// This worker's registered name.
    pub own_service: &'a str,
    pub ignore_services: &'a [String],
    pub attribute_allowlist: &'a [String],
    pub aliases: &'a BTreeMap<String, String>,
}

impl TraceContext<'_> {
    fn service_of(&self, span: &SpanNode) -> String {
        self.aliases
            .get(&span.service_name)
            .cloned()
            .unwrap_or_else(|| span.service_name.clone())
    }

    fn session_id(&self) -> Option<&str> {
        self.trace_tags.get("iii.session.id").map(String::as_str)
    }
}

/// Whether a function is this worker's own: one of its `<worker>::*`
/// functions, or a handler its console page registers from the browser
/// (`iii::<worker>-ui::*`, the live-update subscription of every open tab).
///
/// An error in one of them never becomes a group. The monitor watching
/// itself turns one failure into a loop: a closed tab leaves its handler
/// behind, delivering a group change to it fails, the engine logs the
/// failure, the log changes a group, and the change is delivered again.
pub fn is_own_function(function: &str, own: &str) -> bool {
    function.starts_with(&format!("{own}::")) || function.starts_with(&format!("iii::{own}-ui::"))
}

/// Whether the whole trace must be left alone, judged from its **root**.
pub fn exclusion(spans: &[SpanNode], context: &TraceContext) -> Option<Exclusion> {
    if context
        .session_id()
        .is_some_and(ids::is_investigation_session)
    {
        return Some(Exclusion::Investigation);
    }
    let roots: Vec<&SpanNode> = spans.iter().filter(|span| span.depth == 0).collect();
    for root in &roots {
        let service = context.service_of(root);
        let function = root.function_id().unwrap_or_default();
        // A queue step runs as its own trace, named for the queue rather than
        // the worker, so the name has to be checked as well as the service.
        if service == context.own_service
            || is_own_function(&function, context.own_service)
            || root
                .name
                .starts_with(&format!("fn_queue {}-", context.own_service))
        {
            return Some(Exclusion::OwnTrace);
        }
        if context.ignore_services.contains(&service) {
            return Some(Exclusion::IgnoredService);
        }
    }
    None
}

/// The error spans that are the failure itself rather than its propagation.
pub fn leaf_errors(spans: &[SpanNode]) -> Vec<&SpanNode> {
    let error_parents: HashSet<&str> = spans
        .iter()
        .filter(|span| span.is_error())
        .filter_map(|span| span.parent_span_id.as_deref())
        .collect();
    spans
        .iter()
        .filter(|span| span.is_error() && !span.is_internal())
        // A span whose child also failed is the failure travelling upward.
        .filter(|span| !error_parents.contains(span.span_id.as_str()))
        .collect()
}

/// The ancestors that carry the same failure upward.
pub fn propagated_through<'a>(spans: &'a [SpanNode], leaf: &SpanNode) -> Vec<&'a SpanNode> {
    let by_id: BTreeMap<&str, &SpanNode> = spans
        .iter()
        .map(|span| (span.span_id.as_str(), span))
        .collect();
    let mut chain = Vec::new();
    let mut current = leaf.parent_span_id.as_deref();
    while let Some(parent_id) = current {
        let Some(parent) = by_id.get(parent_id) else {
            break;
        };
        if parent.is_error() {
            chain.push(*parent);
        }
        current = parent.parent_span_id.as_deref();
    }
    chain
}

/// Build the event for one leaf error span. `service_name` and `namespace`
/// are filled in by the caller from the function registry — the span cannot
/// be trusted for them.
pub fn event_for(
    leaf: &SpanNode,
    context: &TraceContext,
    redactor: &Redactor,
    redactions: &mut u64,
) -> ErrorEvent {
    let raw_message = leaf
        .exception_attribute("exception.message")
        .or_else(|| leaf.status_description.clone())
        .unwrap_or_else(|| leaf.name.clone());
    let (message, hits) = redactor.redact(&raw_message);
    *redactions += hits;

    let stacktrace = leaf
        .exception_attribute("exception.stacktrace")
        .map(|stack| {
            let (redacted, hits) = redactor.redact(&stack);
            *redactions += hits;
            redacted
        });

    ErrorEvent {
        source: ErrorSourceV1::Trace,
        dedupe_key: format!("trace:{}:{}", context.trace_id, leaf.span_id),
        at_ms: leaf.at_ms(),
        // Replaced by the owning worker once the registry answers.
        service_name: context.service_of(leaf),
        namespace: String::new(),
        function_id: leaf.function_id(),
        span_name: Some(leaf.name.clone()),
        exception_type: leaf.exception_attribute("exception.type"),
        message,
        stacktrace,
        trace_id: Some(context.trace_id.to_string()),
        span_id: Some(leaf.span_id.clone()),
        session_id: context.session_id().map(str::to_string),
        turn_id: context.trace_tags.get("iii.message.id").cloned(),
        worker_version: None,
        namespace_ambiguous: false,
        evidence: None,
    }
}

/// Freeze the trace around one failure.
pub fn bundle_for(
    leaf: &SpanNode,
    spans: &[SpanNode],
    logs: &[Value],
    context: &TraceContext,
    redactor: &Redactor,
    captured_at_ms: i64,
    redactions: &mut u64,
) -> EvidenceBundleV1 {
    let mut truncated = EvidenceTruncatedV1::default();
    let evidence_spans = spans
        .iter()
        .map(|span| EvidenceSpanV1 {
            span_id: span.span_id.clone(),
            parent_span_id: span.parent_span_id.clone(),
            name: span.name.clone(),
            service_name: span.service_name.clone(),
            function_id: span.function_id(),
            start_time_unix_nano: span.start_time_unix_nano,
            end_time_unix_nano: span.end_time_unix_nano,
            status: span.status.clone(),
            status_description: span.status_description.clone().map(|description| {
                let (redacted, hits) = redactor.redact(&description);
                *redactions += hits;
                redacted
            }),
            attributes: filter_attributes(
                span.attributes.clone(),
                context.attribute_allowlist,
                redactor,
                &mut truncated,
                redactions,
            ),
            events: span
                .events
                .iter()
                .map(|event| EvidenceEventV1 {
                    name: event.name.clone(),
                    timestamp_unix_nano: event.timestamp_unix_nano,
                    attributes: filter_attributes(
                        event.attributes.clone(),
                        context.attribute_allowlist,
                        redactor,
                        &mut truncated,
                        redactions,
                    ),
                })
                .collect(),
            depth: span.depth,
        })
        .collect();

    let kept_logs: Vec<EvidenceLogV1> = logs
        .iter()
        .take(log_limit())
        .map(|log| EvidenceLogV1 {
            timestamp_unix_nano: log
                .get("timestamp_unix_nano")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            severity_text: log
                .get("severity_text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            body: {
                let raw = log.get("body").and_then(Value::as_str).unwrap_or_default();
                let (redacted, hits) = redactor.redact(raw);
                *redactions += hits;
                redacted
            },
            span_id: log
                .get("span_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            attributes: filter_attributes(
                log.get("attributes")
                    .and_then(Value::as_object)
                    .map(|fields| {
                        fields
                            .iter()
                            .map(|(key, value)| {
                                (
                                    key.clone(),
                                    value
                                        .as_str()
                                        .map(str::to_string)
                                        .unwrap_or_else(|| value.to_string()),
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
                context.attribute_allowlist,
                redactor,
                &mut truncated,
                redactions,
            ),
        })
        .collect();
    truncated.logs += logs.len().saturating_sub(kept_logs.len()) as u32;

    EvidenceBundleV1 {
        version: EVIDENCE_VERSION,
        captured_at_ms,
        settled: false,
        trace_id: context.trace_id.to_string(),
        origin_span_id: leaf.span_id.clone(),
        propagated_through: propagated_through(spans, leaf)
            .into_iter()
            .map(|span| span.span_id.clone())
            .collect(),
        trace_tags: context.trace_tags.clone(),
        spans: evidence_spans,
        logs: kept_logs,
        worker: EvidenceWorkerV1 {
            service_name: context.service_of(leaf),
            version: None,
        },
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_monitors_own_functions_include_its_pages_handlers() {
        assert!(is_own_function("sentinel::groups::list", "sentinel"));
        assert!(is_own_function(
            "iii::sentinel-ui::events::tab-c28f:pane:0::console-3691",
            "sentinel"
        ));
        assert!(
            !is_own_function("iii::kanban-ui::events::tab", "sentinel"),
            "another page's handler"
        );
        assert!(
            !is_own_function("sentinel-e2e::boom", "sentinel"),
            "a worker that merely shares the prefix"
        );
        assert!(!is_own_function("state::get", "sentinel"));
    }

    fn span(id: &str, name: &str, service: &str, status: &str, children: Value) -> Value {
        json!({
            "span_id": id,
            "name": name,
            "service_name": service,
            "status": status,
            "start_time_unix_nano": 1_789_000_000_000_000_000u64,
            "end_time_unix_nano": 1_789_000_000_500_000_000u64,
            "attributes": [],
            "events": [],
            "links": [],
            "children": children,
        })
    }

    fn with_parent(mut value: Value, parent: &str) -> Value {
        value["parent_span_id"] = json!(parent);
        value
    }

    fn context<'a>(
        trace_tags: &'a BTreeMap<String, String>,
        ignore: &'a [String],
        aliases: &'a BTreeMap<String, String>,
    ) -> TraceContext<'a> {
        TraceContext {
            trace_id: "t1",
            trace_tags,
            own_service: "sentinel",
            ignore_services: ignore,
            attribute_allowlist: &[],
            aliases,
        }
    }

    /// `execute shell::run` fails, its turn step fails, the root fails.
    fn cascade() -> Vec<SpanNode> {
        let leaf = with_parent(
            span("leaf", "execute shell::run", "shell", "error", json!([])),
            "step",
        );
        let step = with_parent(
            span(
                "step",
                "harness::turn step",
                "harness",
                "error",
                json!([leaf]),
            ),
            "root",
        );
        let root = span(
            "root",
            "execute harness::send",
            "harness",
            "error",
            json!([step]),
        );
        flatten(&[root])
    }

    #[test]
    fn a_tree_flattens_with_its_depths_and_pairs() {
        let spans = cascade();
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].span_id, "root");
        assert_eq!(spans[0].depth, 0);
        assert_eq!(spans[2].span_id, "leaf");
        assert_eq!(spans[2].depth, 2);
        assert_eq!(spans[2].parent_span_id.as_deref(), Some("step"));
    }

    #[test]
    fn only_the_deepest_failure_becomes_a_group() {
        let spans = cascade();
        let leaves = leaf_errors(&spans);
        assert_eq!(leaves.len(), 1, "the cascade is one failure, not three");
        assert_eq!(leaves[0].span_id, "leaf");

        let chain = propagated_through(&spans, leaves[0]);
        assert_eq!(
            chain.iter().map(|s| s.span_id.as_str()).collect::<Vec<_>>(),
            vec!["step", "root"],
            "the ancestors are context, in order"
        );
    }

    #[test]
    fn two_failures_in_parallel_are_two_groups() {
        let left = with_parent(
            span("left", "execute a::x", "a", "error", json!([])),
            "root",
        );
        let right = with_parent(
            span("right", "execute b::y", "b", "error", json!([])),
            "root",
        );
        let root = span(
            "root",
            "execute harness::send",
            "harness",
            "error",
            json!([left, right]),
        );
        let spans = flatten(&[root]);
        let leaves = leaf_errors(&spans);
        assert_eq!(leaves.len(), 2);
    }

    #[test]
    fn a_parent_that_fails_after_its_child_succeeded_is_itself_the_failure() {
        let ok_child = with_parent(
            span("child", "execute a::x", "a", "ok", json!([])),
            "parent",
        );
        let parent = span("parent", "execute b::y", "b", "error", json!([ok_child]));
        let spans = flatten(&[parent]);
        let leaves = leaf_errors(&spans);
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].span_id, "parent");
    }

    #[test]
    fn engine_plumbing_never_becomes_a_group_of_its_own() {
        let mut internal = span(
            "internal",
            "call engine::queue::enqueue",
            "iii",
            "error",
            json!([]),
        );
        internal["attributes"] = json!([["iii.function.kind", "internal"]]);
        let spans = flatten(&[internal]);
        assert!(leaf_errors(&spans).is_empty());
    }

    #[test]
    fn a_trace_rooted_in_this_worker_is_dropped_whole() {
        let tags = BTreeMap::new();
        let aliases = BTreeMap::new();
        let context = context(&tags, &[], &aliases);

        // The failure surfaces in `database`, not in the sentinel: excluding
        // by span would leave the loop open.
        let db = with_parent(
            span(
                "db",
                "execute database::execute",
                "database",
                "error",
                json!([]),
            ),
            "own",
        );
        let own = span(
            "own",
            "execute sentinel::ingest",
            "sentinel",
            "error",
            json!([db]),
        );
        assert_eq!(
            exclusion(&flatten(&[own]), &context),
            Some(Exclusion::OwnTrace)
        );

        // The queue step runs as its own trace, named for the queue.
        let queued = span("q", "fn_queue sentinel-ingest", "queue", "error", json!([]));
        assert_eq!(
            exclusion(&flatten(&[queued]), &context),
            Some(Exclusion::OwnTrace)
        );
    }

    #[test]
    fn an_investigation_trace_is_dropped_by_its_session_tag() {
        let mut tags = BTreeMap::new();
        tags.insert(
            "iii.session.id".to_string(),
            "sentinel-inv-01j8m2".to_string(),
        );
        let aliases = BTreeMap::new();
        let context = context(&tags, &[], &aliases);
        let spans = flatten(&[span(
            "a",
            "execute coder::read-file",
            "ide",
            "error",
            json!([]),
        )]);
        assert_eq!(exclusion(&spans, &context), Some(Exclusion::Investigation));
    }

    #[test]
    fn an_ignored_service_is_honoured_through_its_alias() {
        let tags = BTreeMap::new();
        let mut aliases = BTreeMap::new();
        aliases.insert("noisy-bin".to_string(), "noisy".to_string());
        let ignore = vec!["noisy".to_string()];
        let context = context(&tags, &ignore, &aliases);
        let spans = flatten(&[span(
            "a",
            "execute noisy::x",
            "noisy-bin",
            "error",
            json!([]),
        )]);
        assert_eq!(exclusion(&spans, &context), Some(Exclusion::IgnoredService));
    }

    #[test]
    fn an_ordinary_trace_is_not_excluded() {
        let tags = BTreeMap::new();
        let aliases = BTreeMap::new();
        assert_eq!(exclusion(&cascade(), &context(&tags, &[], &aliases)), None);
    }

    #[test]
    fn the_event_prefers_the_exception_then_the_status_then_the_name() {
        let mut tags = BTreeMap::new();
        tags.insert("iii.session.id".into(), "s_7a1".into());
        tags.insert("iii.message.id".into(), "t_12".into());
        let aliases = BTreeMap::new();
        let context = context(&tags, &[], &aliases);
        let redactor = Redactor::default();
        let mut redactions = 0;

        let mut raised = span("leaf", "execute state::cas", "state", "error", json!([]));
        raised["events"] = json!([{
            "name": "exception",
            "timestamp_unix_nano": 1u64,
            "attributes": [
                ["exception.type", "CasMismatch"],
                ["exception.message", "expected version 41, found 42"],
                ["exception.stacktrace", "at state::cas"],
            ],
        }]);
        let spans = flatten(&[raised]);
        let event = event_for(&spans[0], &context, &redactor, &mut redactions);
        assert_eq!(event.exception_type.as_deref(), Some("CasMismatch"));
        assert_eq!(event.message, "expected version 41, found 42");
        assert_eq!(event.stacktrace.as_deref(), Some("at state::cas"));
        assert_eq!(event.dedupe_key, "trace:t1:leaf");
        assert_eq!(event.session_id.as_deref(), Some("s_7a1"));
        assert_eq!(event.turn_id.as_deref(), Some("t_12"));
        assert_eq!(event.function_id.as_deref(), Some("state::cas"));

        let mut described = span("leaf2", "execute state::cas", "state", "error", json!([]));
        described["status_description"] = json!("connect postgres://u:p@h/db");
        let spans = flatten(&[described]);
        let event = event_for(&spans[0], &context, &redactor, &mut redactions);
        assert!(
            !event.message.contains("u:p@h"),
            "the message is redacted before it is ever stored: {}",
            event.message
        );

        let bare = flatten(&[span(
            "leaf3",
            "harness::turn step",
            "harness",
            "error",
            json!([]),
        )]);
        let event = event_for(&bare[0], &context, &redactor, &mut redactions);
        assert_eq!(event.message, "harness::turn step");
    }

    #[test]
    fn the_bundle_keeps_the_whole_trace_with_the_origin_named() {
        let tags = BTreeMap::new();
        let aliases = BTreeMap::new();
        let context = context(&tags, &[], &aliases);
        let spans = cascade();
        let leaf = leaf_errors(&spans)[0];
        let logs = vec![json!({
            "timestamp_unix_nano": 2u64,
            "severity_text": "ERROR",
            "body": "redelivery exhausted",
            "span_id": "leaf",
            "attributes": { "code.function": "deliver", "customer.email": "a@b.com" },
        })];
        let mut redactions = 0;
        let bundle = bundle_for(
            leaf,
            &spans,
            &logs,
            &context,
            &Redactor::default(),
            1_789_000_000_000,
            &mut redactions,
        );

        assert_eq!(bundle.origin_span_id, "leaf");
        assert_eq!(bundle.propagated_through, vec!["step", "root"]);
        assert_eq!(bundle.spans.len(), 3, "the whole trace is the evidence");
        assert_eq!(bundle.logs.len(), 1);
        assert!(!bundle.settled, "the ancestors may still be open");
        assert_eq!(
            bundle.logs[0].attributes.keys().collect::<Vec<_>>(),
            vec!["code.function"],
            "an off-allowlist log attribute is dropped like a span's"
        );
        assert_eq!(bundle.truncated.attributes, 1);
    }
}
