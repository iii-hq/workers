//! `harness::metrics` — aggregate durable model usage and function outcomes
//! plus trace/span observability over one complete root-and-descendant session
//! tree.

use std::collections::{BTreeSet, HashMap};

use iii_sdk::protocol::TriggerRequest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::types::content::ContentBlock;
use crate::types::message::AgentMessage;

use super::session_tree::{self, SessionTreeNodeV1};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionMetricsRequestV1 {
    pub root_session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionMetricsResponseV1 {
    pub root_session_id: String,
    pub complete: bool,
    pub totals: SessionUsageTotalsV1,
    pub by_session: Vec<SessionUsageV1>,
    /// Trace/span aggregates when the engine's in-memory observability exporter
    /// is available. Usage metrics remain available when it is not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traces: Option<SessionTraceMetricsV1>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionUsageTotalsV1 {
    pub sessions: u64,
    pub turns: u64,
    pub function_calls: u64,
    pub function_call_errors: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionUsageV1 {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    pub depth: u32,
    pub turns: u64,
    pub function_calls: u64,
    pub function_call_errors: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionTraceMetricsV1 {
    /// Distinct traces across the root session and all descendants.
    pub trace_count: u64,
    pub span_count: u64,
    pub error_span_count: u64,
    /// Elapsed window from the first observed span to the last observed span.
    pub duration_ms: u64,
    pub by_session: Vec<SessionTraceUsageV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionTraceUsageV1 {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    pub depth: u32,
    pub trace_count: u64,
    pub span_count: u64,
    pub error_span_count: u64,
    /// Elapsed window from the session's first observed span to its last.
    pub duration_ms: u64,
}

#[derive(Debug, Deserialize)]
struct TraceGroupByResponse {
    groups: Vec<TraceGroup>,
}

#[derive(Debug, Deserialize)]
struct TraceGroup {
    value: String,
    trace_ids: Vec<String>,
    span_count: u64,
    first_seen_ms: u64,
    last_seen_ms: u64,
    duration_ms: u64,
    error_count: u64,
}

pub async fn handle(
    deps: &Deps,
    req: SessionMetricsRequestV1,
) -> Result<SessionMetricsResponseV1, HarnessError> {
    let tree = session_tree::collect(deps, &req.root_session_id).await?;
    if !tree.complete {
        return Ok(incomplete(&req.root_session_id));
    }

    let session = deps.session().await;
    let cfg = deps.cfg().await;
    let mut total = UsageAccumulator::default();
    let mut by_session = Vec::with_capacity(tree.sessions.len());
    for node in &tree.sessions {
        if !session.exists(&node.session_id).await? {
            return Ok(incomplete(&req.root_session_id));
        }
        let Some(turn) =
            crate::state::get_turn(&deps.iii, &node.session_id, cfg.session_timeout_ms).await?
        else {
            return Ok(incomplete(&req.root_session_id));
        };
        if !terminal_status(Some(turn.status)) {
            return Ok(incomplete(&req.root_session_id));
        }
        let entries = session.messages_strict(&node.session_id).await?;
        let mut current = UsageAccumulator::default();
        for entry in entries {
            if let Some(message) = entry.message.as_ref() {
                current.observe(message);
                total.observe(message);
            }
        }
        by_session.push(current.finish_session(node));
    }
    let traces = collect_trace_metrics(deps, &tree.sessions, &req.root_session_id).await;

    Ok(SessionMetricsResponseV1 {
        root_session_id: req.root_session_id,
        complete: true,
        totals: total.finish_totals(tree.sessions.len() as u64),
        by_session,
        traces,
    })
}

fn terminal_status(status: Option<crate::types::turn::TurnStatus>) -> bool {
    status.is_some_and(crate::types::turn::TurnStatus::is_terminal)
}

fn incomplete(root_session_id: &str) -> SessionMetricsResponseV1 {
    SessionMetricsResponseV1 {
        root_session_id: root_session_id.to_string(),
        complete: false,
        totals: SessionUsageTotalsV1::default(),
        by_session: Vec::new(),
        traces: None,
    }
}

async fn collect_trace_metrics(
    deps: &Deps,
    sessions: &[SessionTreeNodeV1],
    root_session_id: &str,
) -> Option<SessionTraceMetricsV1> {
    let cfg = deps.cfg().await;
    let response = deps
        .iii
        .trigger(TriggerRequest {
            function_id: "engine::traces::group_by".into(),
            payload: json!({
                "attribute": "iii.session.id",
                "limit": u32::MAX,
                "include_internal": true,
            }),
            action: None,
            timeout_ms: Some(cfg.dispatch_timeout_ms),
        })
        .await;
    let value = match response {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                %root_session_id,
                %error,
                "harness::metrics: trace metrics unavailable"
            );
            return None;
        }
    };
    let response: TraceGroupByResponse = match serde_json::from_value(value) {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(
                %root_session_id,
                %error,
                "harness::metrics: invalid trace metrics response"
            );
            return None;
        }
    };
    Some(summarize_trace_groups(sessions, response.groups))
}

fn summarize_trace_groups(
    sessions: &[SessionTreeNodeV1],
    groups: Vec<TraceGroup>,
) -> SessionTraceMetricsV1 {
    let mut groups: HashMap<_, _> = groups
        .into_iter()
        .map(|group| (group.value.clone(), group))
        .collect();
    let mut trace_ids = BTreeSet::new();
    let mut span_count = 0u64;
    let mut error_span_count = 0u64;
    let mut first_seen_ms = None::<u64>;
    let mut last_seen_ms = None::<u64>;
    let mut by_session = Vec::with_capacity(sessions.len());

    for session in sessions {
        let group = groups.remove(&session.session_id);
        let (session_trace_count, session_span_count, session_errors, duration_ms) = match group {
            Some(group) => {
                trace_ids.extend(group.trace_ids.iter().cloned());
                span_count = span_count.saturating_add(group.span_count);
                error_span_count = error_span_count.saturating_add(group.error_count);
                first_seen_ms = Some(first_seen_ms.map_or(group.first_seen_ms, |current| {
                    current.min(group.first_seen_ms)
                }));
                last_seen_ms = Some(last_seen_ms.map_or(group.last_seen_ms, |current| {
                    current.max(group.last_seen_ms)
                }));
                (
                    group.trace_ids.len() as u64,
                    group.span_count,
                    group.error_count,
                    group.duration_ms,
                )
            }
            None => (0, 0, 0, 0),
        };
        by_session.push(SessionTraceUsageV1 {
            session_id: session.session_id.clone(),
            parent_session_id: session.parent_session_id.clone(),
            depth: session.depth,
            trace_count: session_trace_count,
            span_count: session_span_count,
            error_span_count: session_errors,
            duration_ms,
        });
    }

    SessionTraceMetricsV1 {
        trace_count: trace_ids.len() as u64,
        span_count,
        error_span_count,
        duration_ms: match (first_seen_ms, last_seen_ms) {
            (Some(first), Some(last)) => last.saturating_sub(first),
            _ => 0,
        },
        by_session,
    }
}

#[derive(Debug, Default)]
struct UsageAccumulator {
    turns: u64,
    function_calls: u64,
    function_call_errors: u64,
    input: OptionalU64Sum,
    output: OptionalU64Sum,
    cache_read: OptionalU64Sum,
    cache_write: OptionalU64Sum,
    reasoning: OptionalU64Sum,
    cost_usd: OptionalF64Sum,
}

impl UsageAccumulator {
    fn observe(&mut self, message: &AgentMessage) {
        match message {
            AgentMessage::Assistant(assistant) => {
                self.turns += 1;
                self.function_calls += assistant
                    .content
                    .iter()
                    .filter(|block| matches!(block, ContentBlock::FunctionCall { .. }))
                    .count() as u64;
                let usage = assistant.usage.as_ref();
                self.input.observe(usage.and_then(|value| value.input));
                self.output.observe(usage.and_then(|value| value.output));
                self.cache_read
                    .observe(usage.and_then(|value| value.cache_read));
                self.cache_write
                    .observe(usage.and_then(|value| value.cache_write));
                self.reasoning
                    .observe(usage.and_then(|value| value.reasoning));
                self.cost_usd
                    .observe(usage.and_then(|value| value.cost_usd));
            }
            AgentMessage::FunctionResult(result) if result.is_error => {
                self.function_call_errors += 1;
            }
            _ => {}
        }
    }

    fn finish_session(self, node: &SessionTreeNodeV1) -> SessionUsageV1 {
        SessionUsageV1 {
            session_id: node.session_id.clone(),
            parent_session_id: node.parent_session_id.clone(),
            depth: node.depth,
            turns: self.turns,
            function_calls: self.function_calls,
            function_call_errors: self.function_call_errors,
            input_tokens: self.input.finish(),
            output_tokens: self.output.finish(),
            cache_read_tokens: self.cache_read.finish(),
            cache_write_tokens: self.cache_write.finish(),
            reasoning_tokens: self.reasoning.finish(),
            cost_usd: self.cost_usd.finish(),
        }
    }

    fn finish_totals(self, sessions: u64) -> SessionUsageTotalsV1 {
        SessionUsageTotalsV1 {
            sessions,
            turns: self.turns,
            function_calls: self.function_calls,
            function_call_errors: self.function_call_errors,
            input_tokens: self.input.finish(),
            output_tokens: self.output.finish(),
            cache_read_tokens: self.cache_read.finish(),
            cache_write_tokens: self.cache_write.finish(),
            reasoning_tokens: self.reasoning.finish(),
            cost_usd: self.cost_usd.finish(),
        }
    }
}

#[derive(Debug, Default)]
struct OptionalU64Sum {
    value: u64,
    observations: u64,
    all_present: bool,
}

impl OptionalU64Sum {
    fn observe(&mut self, value: Option<u64>) {
        if self.observations == 0 {
            self.all_present = true;
        }
        self.observations += 1;
        match value {
            Some(value) => self.value = self.value.saturating_add(value),
            None => self.all_present = false,
        }
    }

    fn finish(self) -> Option<u64> {
        (self.observations > 0 && self.all_present).then_some(self.value)
    }
}

#[derive(Debug, Default)]
struct OptionalF64Sum {
    value: f64,
    observations: u64,
    all_present: bool,
}

impl OptionalF64Sum {
    fn observe(&mut self, value: Option<f64>) {
        if self.observations == 0 {
            self.all_present = true;
        }
        self.observations += 1;
        match value {
            Some(value) => self.value += value,
            None => self.all_present = false,
        }
    }

    fn finish(self) -> Option<f64> {
        (self.observations > 0 && self.all_present).then_some(self.value)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::types::turn::TurnStatus;

    fn message(value: serde_json::Value) -> AgentMessage {
        serde_json::from_value(value).unwrap()
    }

    fn session(id: &str, parent: Option<&str>, depth: u32) -> SessionTreeNodeV1 {
        SessionTreeNodeV1 {
            session_id: id.into(),
            parent_session_id: parent.map(str::to_string),
            parent_turn_id: None,
            depth,
        }
    }

    #[test]
    fn counts_generations_calls_errors_and_usage() {
        let mut usage = UsageAccumulator::default();
        usage.observe(&message(json!({
            "role": "assistant",
            "content": [{
                "type": "function_call",
                "id": "call-1",
                "function_id": "orders::refund",
                "arguments": {"order_id": 4512}
            }],
            "stop_reason": "function_call",
            "usage": {"input": 10, "output": 2, "cost_usd": 0.1},
            "model": "model",
            "provider": "provider",
            "timestamp": 1
        })));
        usage.observe(&message(json!({
            "role": "function_result",
            "function_call_id": "call-1",
            "function_id": "orders::refund",
            "content": [],
            "details": {},
            "is_error": true,
            "timestamp": 2
        })));
        let totals = usage.finish_totals(1);
        assert_eq!(totals.sessions, 1);
        assert_eq!(totals.turns, 1);
        assert_eq!(totals.function_calls, 1);
        assert_eq!(totals.function_call_errors, 1);
        assert_eq!(totals.input_tokens, Some(10));
        assert_eq!(totals.output_tokens, Some(2));
        assert_eq!(totals.cost_usd, Some(0.1));
        assert_eq!(totals.cache_read_tokens, None);
    }

    #[test]
    fn optional_usage_never_reports_partial_sums() {
        let mut usage = UsageAccumulator::default();
        for input in [Some(10), None] {
            usage.observe(&message(json!({
                "role": "assistant",
                "content": [],
                "stop_reason": "end",
                "usage": {"input": input, "output": 1},
                "model": "model",
                "provider": "provider",
                "timestamp": 1
            })));
        }
        let totals = usage.finish_totals(1);
        assert_eq!(totals.turns, 2);
        assert_eq!(totals.input_tokens, None);
        assert_eq!(totals.output_tokens, Some(2));
    }

    #[test]
    fn incomplete_metrics_never_contain_partial_values() {
        let response = incomplete("root");
        assert!(!response.complete);
        assert!(response.by_session.is_empty());
        assert_eq!(response.totals, SessionUsageTotalsV1::default());
        assert_eq!(response.traces, None);
    }

    #[test]
    fn metrics_complete_only_for_present_terminal_turns() {
        assert!(!terminal_status(None));
        assert!(!terminal_status(Some(TurnStatus::Running)));
        assert!(!terminal_status(Some(TurnStatus::AwaitingFunctions)));
        assert!(terminal_status(Some(TurnStatus::Completed)));
        assert!(terminal_status(Some(TurnStatus::Cancelled)));
        assert!(terminal_status(Some(TurnStatus::Failed)));
    }

    #[test]
    fn trace_summary_covers_only_the_session_tree_and_deduplicates_traces() {
        let sessions = [
            session("root", None, 0),
            session("child", Some("root"), 1),
            session("without-spans", Some("root"), 1),
        ];
        let summary = summarize_trace_groups(
            &sessions,
            vec![
                TraceGroup {
                    value: "root".into(),
                    trace_ids: vec!["trace-a".into(), "trace-shared".into()],
                    span_count: 5,
                    first_seen_ms: 100,
                    last_seen_ms: 300,
                    duration_ms: 200,
                    error_count: 1,
                },
                TraceGroup {
                    value: "child".into(),
                    trace_ids: vec!["trace-shared".into(), "trace-b".into()],
                    span_count: 7,
                    first_seen_ms: 250,
                    last_seen_ms: 600,
                    duration_ms: 350,
                    error_count: 2,
                },
                TraceGroup {
                    value: "unrelated".into(),
                    trace_ids: vec!["trace-other".into()],
                    span_count: 99,
                    first_seen_ms: 1,
                    last_seen_ms: 999,
                    duration_ms: 998,
                    error_count: 9,
                },
            ],
        );

        assert_eq!(summary.trace_count, 3);
        assert_eq!(summary.span_count, 12);
        assert_eq!(summary.error_span_count, 3);
        assert_eq!(summary.duration_ms, 500);
        assert_eq!(summary.by_session.len(), 3);
        assert_eq!(summary.by_session[0].trace_count, 2);
        assert_eq!(summary.by_session[1].span_count, 7);
        assert_eq!(
            summary.by_session[2],
            SessionTraceUsageV1 {
                session_id: "without-spans".into(),
                parent_session_id: Some("root".into()),
                depth: 1,
                trace_count: 0,
                span_count: 0,
                error_span_count: 0,
                duration_ms: 0,
            }
        );
    }
}
