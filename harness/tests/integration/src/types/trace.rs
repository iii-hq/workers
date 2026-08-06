//! Trace evidence returned by the engine's in-memory observability worker.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceEvidenceV1 {
    pub traces: Vec<TraceTreeV1>,
    pub summary: TraceSummaryV1,
}

impl TraceEvidenceV1 {
    pub fn new(mut traces: Vec<TraceTreeV1>) -> Self {
        for trace in &mut traces {
            trace.sort();
        }
        traces.sort_by(|left, right| {
            left.first_started_at()
                .cmp(&right.first_started_at())
                .then_with(|| left.trace_id.cmp(&right.trace_id))
        });
        let summary = TraceSummaryV1::from_traces(&traces);
        Self { traces, summary }
    }

    /// [`Self::new`] with the turn count scoped to `session_id`'s own spans —
    /// the collector's constructor, so a child session nested in the tracked
    /// session's trace never counts as one of its turns.
    pub fn for_session(traces: Vec<TraceTreeV1>, session_id: &str) -> Self {
        let mut evidence = Self::new(traces);
        evidence.summary = TraceSummaryV1::from_traces_owned(&evidence.traces, Some(session_id));
        evidence
    }

    pub fn spans(&self) -> impl Iterator<Item = &TraceSpanV1> {
        self.traces.iter().flat_map(TraceTreeV1::spans)
    }

    pub fn spans_named(&self, name: &str) -> Vec<&TraceSpanV1> {
        self.spans().filter(|span| span.name == name).collect()
    }

    /// Stable when every expected completion has arrived: `total_turns`
    /// distinct turn ids in the traces (parked completions included), of
    /// which exactly `terminal_turns` carry a `terminal: true` lifecycle
    /// payload. The split matters for park-then-wake runs — the parked turn
    /// is real trace evidence but never terminal.
    pub fn is_stable_for(
        &self,
        terminal_turns: usize,
        total_turns: usize,
        lifecycle_sink: &str,
    ) -> bool {
        if self.summary.pending_span_count != 0 || self.summary.turn_ids.len() != total_turns {
            return false;
        }

        let lifecycle_name = format!("execute {lifecycle_sink}");
        let lifecycle_turns = self
            .spans_named(&lifecycle_name)
            .into_iter()
            .filter_map(TraceSpanV1::invocation_input)
            .filter_map(|mut payload| {
                strip_engine_fields(&mut payload);
                let terminal = payload.get("terminal").and_then(Value::as_bool) == Some(true);
                terminal
                    .then(|| {
                        payload
                            .get("turn_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .flatten()
            })
            .collect::<BTreeSet<_>>();

        lifecycle_turns.len() == terminal_turns
            && lifecycle_turns
                .iter()
                .all(|turn_id| self.summary.turn_ids.contains(turn_id))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceSummaryV1 {
    pub trace_count: usize,
    pub span_count: usize,
    pub error_count: usize,
    pub pending_span_count: usize,
    pub turn_ids: Vec<String>,
}

impl TraceSummaryV1 {
    fn from_traces(traces: &[TraceTreeV1]) -> Self {
        Self::from_traces_owned(traces, None)
    }

    /// `owner: Some(session)` counts turn ids only from that session's spans:
    /// an in-turn `harness::spawn` nests the child's whole turn under the
    /// parent's trace, and a child's turns are not the tracked session's
    /// completions. `None` keeps the count-everything behavior for callers
    /// with no session to scope by.
    fn from_traces_owned(traces: &[TraceTreeV1], owner: Option<&str>) -> Self {
        let spans: Vec<_> = traces.iter().flat_map(TraceTreeV1::spans).collect();
        let mut turn_ids = spans
            .iter()
            .filter(|span| match owner {
                Some(owner) => span.attribute("iii.session.id") == Some(owner),
                None => true,
            })
            .filter_map(|span| span.attribute("iii.message.id"))
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        turn_ids.sort_by_key(|turn_id| {
            spans
                .iter()
                .filter(|span| span.attribute("iii.message.id") == Some(turn_id.as_str()))
                .map(|span| span.start_time_unix_nano)
                .min()
                .unwrap_or(u64::MAX)
        });

        Self {
            trace_count: traces.len(),
            span_count: spans.len(),
            error_count: spans
                .iter()
                .filter(|span| span.status.eq_ignore_ascii_case("error"))
                .count(),
            pending_span_count: spans.iter().filter(|span| span.pending).count(),
            turn_ids,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceTreeV1 {
    pub trace_id: String,
    pub roots: Vec<TraceSpanV1>,
}

impl TraceTreeV1 {
    pub fn from_engine_response(trace_id: String, response: Value) -> anyhow::Result<Self> {
        let roots = response
            .get("roots")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("trace tree response has no roots"))?;
        Ok(Self {
            trace_id,
            roots: serde_json::from_value(roots)?,
        })
    }

    pub fn spans(&self) -> impl Iterator<Item = &TraceSpanV1> {
        self.roots
            .iter()
            .flat_map(TraceSpanV1::self_and_descendants)
    }

    fn first_started_at(&self) -> u64 {
        self.spans()
            .map(|span| span.start_time_unix_nano)
            .min()
            .unwrap_or(u64::MAX)
    }

    fn sort(&mut self) {
        for root in &mut self.roots {
            root.sort_children();
        }
        self.roots.sort_by(TraceSpanV1::compare);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TraceSpanV1 {
    pub trace_id: String,
    pub span_id: String,
    #[serde(default)]
    pub parent_span_id: Option<String>,
    pub name: String,
    pub start_time_unix_nano: u64,
    pub end_time_unix_nano: u64,
    pub status: String,
    #[serde(default)]
    pub status_description: Option<String>,
    #[serde(
        default,
        deserialize_with = "attributes::deserialize",
        serialize_with = "attributes::serialize"
    )]
    #[schemars(with = "BTreeMap<String, String>")]
    pub attributes: BTreeMap<String, String>,
    pub service_name: String,
    #[serde(default)]
    pub events: Vec<TraceEventV1>,
    #[serde(default)]
    pub links: Vec<TraceLinkV1>,
    #[serde(default)]
    pub instrumentation_scope_name: Option<String>,
    #[serde(default)]
    pub instrumentation_scope_version: Option<String>,
    #[serde(default)]
    pub flags: Option<u32>,
    #[serde(default)]
    pub trace_state: Option<String>,
    #[serde(default)]
    pub pending: bool,
    #[serde(default)]
    pub children: Vec<TraceSpanV1>,
}

impl TraceSpanV1 {
    pub fn attribute(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).map(String::as_str)
    }

    pub fn event(&self, name: &str) -> Option<&TraceEventV1> {
        self.events.iter().find(|event| event.name == name)
    }

    pub fn invocation_input(&self) -> Option<Value> {
        self.event("iii.invocation.input")?.payload()
    }

    pub fn invocation_output(&self) -> Option<Value> {
        self.event("iii.invocation.output")?.payload()
    }

    fn self_and_descendants(&self) -> Box<dyn Iterator<Item = &TraceSpanV1> + '_> {
        Box::new(
            std::iter::once(self).chain(
                self.children
                    .iter()
                    .flat_map(TraceSpanV1::self_and_descendants),
            ),
        )
    }

    fn sort_children(&mut self) {
        for child in &mut self.children {
            child.sort_children();
        }
        self.children.sort_by(Self::compare);
        self.events.sort_by(|left, right| {
            left.timestamp_unix_nano
                .cmp(&right.timestamp_unix_nano)
                .then_with(|| left.name.cmp(&right.name))
        });
    }

    fn compare(left: &Self, right: &Self) -> std::cmp::Ordering {
        left.start_time_unix_nano
            .cmp(&right.start_time_unix_nano)
            .then_with(|| left.span_id.cmp(&right.span_id))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TraceEventV1 {
    pub name: String,
    pub timestamp_unix_nano: u64,
    #[serde(
        default,
        deserialize_with = "attributes::deserialize",
        serialize_with = "attributes::serialize"
    )]
    #[schemars(with = "BTreeMap<String, String>")]
    pub attributes: BTreeMap<String, String>,
}

impl TraceEventV1 {
    pub fn payload(&self) -> Option<Value> {
        let truncated = self
            .attributes
            .get("iii.payload.truncated")
            .is_some_and(|value| value == "true");
        if truncated {
            return None;
        }
        self.attributes
            .get("iii.payload.json")
            .and_then(|payload| serde_json::from_str(payload).ok())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TraceLinkV1 {
    pub trace_id: String,
    pub span_id: String,
    #[serde(default)]
    pub trace_state: Option<String>,
    #[serde(
        default,
        deserialize_with = "attributes::deserialize",
        serialize_with = "attributes::serialize"
    )]
    #[schemars(with = "BTreeMap<String, String>")]
    pub attributes: BTreeMap<String, String>,
}

pub fn strip_engine_fields(value: &mut Value) {
    if let Some(map) = value.as_object_mut() {
        map.retain(|key, _| !key.starts_with('_'));
    }
}

mod attributes {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum EncodedAttributes {
            Pairs(Vec<(String, String)>),
            Map(BTreeMap<String, String>),
        }

        EncodedAttributes::deserialize(deserializer).map(|attributes| match attributes {
            EncodedAttributes::Pairs(pairs) => pairs.into_iter().collect(),
            EncodedAttributes::Map(map) => map,
        })
    }

    pub(super) fn serialize<S>(
        attributes: &BTreeMap<String, String>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        attributes.serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn span(name: &str, pending: bool) -> TraceSpanV1 {
        TraceSpanV1 {
            trace_id: "trace-1".into(),
            span_id: name.into(),
            parent_span_id: None,
            name: name.into(),
            start_time_unix_nano: 1,
            end_time_unix_nano: 2,
            status: "ok".into(),
            status_description: None,
            attributes: BTreeMap::from([
                ("iii.session.id".into(), "session-1".into()),
                ("iii.message.id".into(), "turn-1".into()),
            ]),
            service_name: "integration".into(),
            events: Vec::new(),
            links: Vec::new(),
            instrumentation_scope_name: None,
            instrumentation_scope_version: None,
            flags: None,
            trace_state: None,
            pending,
            children: Vec::new(),
        }
    }

    #[test]
    fn engine_tree_attributes_are_normalized_to_maps() {
        let tree = TraceTreeV1::from_engine_response(
            "trace-1".into(),
            json!({
                "roots": [{
                    "trace_id": "trace-1",
                    "span_id": "span-1",
                    "parent_span_id": null,
                    "name": "root",
                    "start_time_unix_nano": 1,
                    "end_time_unix_nano": 2,
                    "status": "ok",
                    "attributes": [["iii.session.id", "session-1"]],
                    "service_name": "harness",
                    "events": [],
                    "links": [],
                    "children": []
                }]
            }),
        )
        .unwrap();
        assert_eq!(tree.roots[0].attribute("iii.session.id"), Some("session-1"));
        let encoded = serde_json::to_value(&tree).unwrap();
        assert!(encoded["roots"][0]["attributes"].is_object());
        let decoded: TraceTreeV1 = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, tree);
    }

    #[test]
    fn stability_requires_terminal_lifecycle_and_no_pending_spans() {
        let mut lifecycle = span("execute integration-probe::turn-completed", false);
        lifecycle.events.push(TraceEventV1 {
            name: "iii.invocation.input".into(),
            timestamp_unix_nano: 1,
            attributes: BTreeMap::from([
                (
                    "iii.payload.json".into(),
                    json!({
                        "session_id": "session-1",
                        "turn_id": "turn-1",
                        "status": "completed",
                        "terminal": true,
                        "timestamp": 1
                    })
                    .to_string(),
                ),
                ("iii.payload.truncated".into(), "false".into()),
            ]),
        });
        let evidence = TraceEvidenceV1::new(vec![TraceTreeV1 {
            trace_id: "trace-1".into(),
            roots: vec![lifecycle],
        }]);
        assert!(evidence.is_stable_for(1, 1, "integration-probe::turn-completed"));
        // A parked completion counts toward the TOTAL but not the terminal
        // count: this all-terminal evidence must fail a declared park.
        assert!(!evidence.is_stable_for(0, 1, "integration-probe::turn-completed"));

        let mut pending = evidence.clone();
        pending.traces[0].roots[0].pending = true;
        pending.summary = TraceSummaryV1::from_traces(&pending.traces);
        assert!(!pending.is_stable_for(1, 1, "integration-probe::turn-completed"));
    }
}
