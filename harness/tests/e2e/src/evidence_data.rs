//! Collected data handed to scenario `verify` functions.

use serde::Serialize;
use serde_json::Value;

use crate::types::trace::{strip_engine_fields, TraceEvidenceV1, TraceSpanV1, TraceSummaryV1};

/// Everything a scenario's `verify` function may inspect.
#[derive(Debug, Clone)]
pub struct RunEvidence {
    pub run_id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    /// `harness::send` response — present when the direct runner owns Send.
    pub send_response: Option<Value>,
    /// Final `harness::status` report (JSON null when the session is unknown).
    pub status: Value,
    /// All transcript `MessageItem`s across pages, in order.
    pub transcript: Vec<Value>,
    pub generations_consumed: u64,
    pub generations_total: u64,
    /// Complete trace trees for the run's session.
    pub traces: TraceEvidenceV1,
    /// Every controlled-function invocation payload the probe served, in
    /// arrival order — WHOLE-RUN evidence. Session traces miss dispatches
    /// that never touch the tracked session (a call-mode reaction); the
    /// serving process sees them all.
    pub target_calls: Vec<Value>,
}

/// Compact form published in `playground-result.json`.
#[derive(Serialize)]
pub struct RunEvidenceSummary<'a> {
    pub run_id: &'a str,
    pub session_id: &'a str,
    pub turn_id: Option<&'a str>,
    pub send_response: Option<&'a Value>,
    pub status: &'a Value,
    pub transcript: &'a [Value],
    pub generations_consumed: u64,
    pub generations_total: u64,
    pub trace_summary: &'a TraceSummaryV1,
}

#[derive(Debug, Clone)]
pub struct FunctionCallEvidence<'a> {
    pub span: &'a TraceSpanV1,
    pub payload: Option<Value>,
    pub output: Option<Value>,
}

impl RunEvidence {
    pub fn summary(&self) -> RunEvidenceSummary<'_> {
        RunEvidenceSummary {
            run_id: &self.run_id,
            session_id: &self.session_id,
            turn_id: self.turn_id.as_deref(),
            send_response: self.send_response.as_ref(),
            status: &self.status,
            transcript: &self.transcript,
            generations_consumed: self.generations_consumed,
            generations_total: self.generations_total,
            trace_summary: &self.traces.summary,
        }
    }

    /// Concatenated text blocks of each assistant message, in transcript order.
    pub fn assistant_texts(&self) -> Vec<String> {
        self.messages()
            .filter(|message| role(message) == Some("assistant"))
            .filter_map(|message| {
                let blocks = message.get("content").and_then(Value::as_array)?;
                let texts: Vec<&str> = blocks
                    .iter()
                    .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|block| block.get("text").and_then(Value::as_str))
                    .collect();
                (!texts.is_empty()).then(|| texts.concat())
            })
            .collect()
    }

    /// Durable `(user, assistant, function_result)` message counts.
    pub fn message_counts(&self) -> (u64, u64, u64) {
        let mut counts = (0, 0, 0);
        for message in self.messages() {
            match role(message) {
                Some("user") => counts.0 += 1,
                Some("assistant") => counts.1 += 1,
                Some("function_result") => counts.2 += 1,
                _ => {}
            }
        }
        counts
    }

    pub fn spans_named(&self, name: &str) -> Vec<&TraceSpanV1> {
        self.traces.spans_named(name)
    }

    /// Executions of the controlled function registered under `alias`.
    pub fn calls(&self, alias: &str) -> Vec<FunctionCallEvidence<'_>> {
        let name = format!("execute {}::{alias}", self.run_id);
        self.traces
            .spans_named(&name)
            .into_iter()
            .map(|span| {
                let mut payload = span.invocation_input();
                if let Some(payload) = &mut payload {
                    strip_engine_fields(payload);
                }
                FunctionCallEvidence {
                    span,
                    payload,
                    output: span.invocation_output(),
                }
            })
            .collect()
    }

    /// True when any transcript `entry_id` appears more than once.
    pub fn has_duplicate_messages(&self) -> bool {
        let mut seen = std::collections::BTreeSet::new();
        self.transcript
            .iter()
            .filter_map(|item| item.get("entry_id").and_then(Value::as_str))
            .any(|entry_id| !seen.insert(entry_id))
    }

    pub fn expect_assistant_texts(
        &self,
        expected: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> anyhow::Result<()> {
        let expected: Vec<String> = expected
            .into_iter()
            .map(|text| text.as_ref().to_string())
            .collect();
        let actual = self.assistant_texts();
        anyhow::ensure!(
            actual == expected,
            "assistant texts {actual:?} != {expected:?}"
        );
        Ok(())
    }

    pub fn expect_message_counts(
        &self,
        user: u64,
        assistant: u64,
        function_result: u64,
    ) -> anyhow::Result<()> {
        let actual = self.message_counts();
        let expected = (user, assistant, function_result);
        anyhow::ensure!(
            actual == expected,
            "message counts (user, assistant, function_result) {actual:?} != {expected:?}"
        );
        Ok(())
    }

    /// Exact probe-side (whole-run) invocation count of the controlled
    /// function — use instead of [`expect_function_calls`](Self::expect_function_calls)
    /// when dispatches bypass the tracked session entirely.
    pub fn expect_target_calls(&self, count: usize) -> anyhow::Result<()> {
        let actual = self.target_calls.len();
        anyhow::ensure!(
            actual == count,
            "controlled function served {actual} call(s), expected {count}"
        );
        Ok(())
    }

    pub fn expect_function_calls(&self, alias: &str, count: usize) -> anyhow::Result<()> {
        let actual = self.calls(alias).len();
        anyhow::ensure!(
            actual == count,
            "{alias} ran {actual} times, expected {count}"
        );
        Ok(())
    }

    pub fn expect_call_payload(&self, alias: &str, expected: Value) -> anyhow::Result<()> {
        let calls = self.calls(alias);
        let call = calls
            .first()
            .ok_or_else(|| anyhow::anyhow!("{alias} did not run"))?;
        let payload = call
            .payload
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("{alias} trace has no complete invocation input"))?;
        anyhow::ensure!(
            payload == &expected,
            "{alias} payload {payload} != {expected}"
        );
        Ok(())
    }

    pub fn expect_no_duplicate_messages(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.has_duplicate_messages(),
            "transcript contains duplicate entry ids"
        );
        Ok(())
    }

    /// Replace concrete run identities so failure text stays byte-comparable.
    pub fn scrub(&self, text: &str) -> String {
        let mut text = text.to_string();
        replace_identity(&mut text, &self.run_id, "{{run_id}}");
        replace_identity(&mut text, &self.session_id, "{{session_id}}");
        for turn_id in &self.traces.summary.turn_ids {
            replace_identity(&mut text, turn_id, "{{turn_id}}");
        }
        if let Some(turn_id) = &self.turn_id {
            replace_identity(&mut text, turn_id, "{{turn_id}}");
        }
        text
    }

    fn messages(&self) -> impl Iterator<Item = &Value> {
        self.transcript
            .iter()
            .filter_map(|item| item.get("message"))
    }

    /// Durable `function_result` messages for one function id, in transcript
    /// order. For harness-intercepted builtins (e.g. `engine::register_trigger`)
    /// the transcript is the only evidence surface — they never produce
    /// `execute` spans like controlled functions do.
    pub fn function_results(&self, function_id: &str) -> Vec<&Value> {
        self.messages()
            .filter(|message| role(message) == Some("function_result"))
            .filter(|message| {
                message.get("function_id").and_then(Value::as_str) == Some(function_id)
            })
            .collect()
    }
}

/// Concatenated text blocks of one message's `content` array.
pub fn message_text(message: &Value) -> String {
    message
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .concat()
        })
        .unwrap_or_default()
}

fn role(message: &Value) -> Option<&str> {
    message.get("role").and_then(Value::as_str)
}

fn replace_identity(text: &mut String, identity: &str, placeholder: &str) {
    if !identity.is_empty() && text.contains(identity) {
        *text = text.replace(identity, placeholder);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use crate::types::trace::{TraceEventV1, TraceSpanV1, TraceTreeV1};

    use super::*;

    fn span(name: &str, payload: Value) -> TraceSpanV1 {
        TraceSpanV1 {
            trace_id: "trace-1".into(),
            span_id: name.into(),
            parent_span_id: None,
            name: name.into(),
            start_time_unix_nano: 1,
            end_time_unix_nano: 2,
            status: "ok".into(),
            status_description: None,
            attributes: BTreeMap::from([("iii.message.id".into(), "turn-1".into())]),
            service_name: "integration-probe".into(),
            events: vec![TraceEventV1 {
                name: "iii.invocation.input".into(),
                timestamp_unix_nano: 1,
                attributes: BTreeMap::from([
                    ("iii.payload.json".into(), payload.to_string()),
                    ("iii.payload.truncated".into(), "false".into()),
                ]),
            }],
            links: Vec::new(),
            instrumentation_scope_name: None,
            instrumentation_scope_version: None,
            flags: None,
            trace_state: None,
            pending: false,
            children: Vec::new(),
        }
    }

    fn base_evidence() -> RunEvidence {
        RunEvidence {
            run_id: "r".into(),
            session_id: "session-1".into(),
            turn_id: Some("turn-1".into()),
            send_response: Some(json!({ "accepted": true })),
            status: json!({
                "status": "completed",
                "pending_function_calls": [],
                "children": []
            }),
            transcript: Vec::new(),
            generations_consumed: 1,
            generations_total: 1,
            traces: TraceEvidenceV1::new(Vec::new()),
            target_calls: Vec::new(),
        }
    }

    #[test]
    fn calls_use_execute_spans_and_strip_engine_fields() {
        let mut evidence = base_evidence();
        evidence.traces = TraceEvidenceV1::new(vec![TraceTreeV1 {
            trace_id: "trace-1".into(),
            roots: vec![span(
                "execute r::record",
                json!({ "value": "expected", "_caller_worker_id": "worker" }),
            )],
        }]);
        let calls = evidence.calls("record");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].payload, Some(json!({ "value": "expected" })));
    }

    #[test]
    fn summary_omits_full_traces() {
        let evidence = base_evidence();
        let value = serde_json::to_value(evidence.summary()).unwrap();
        assert!(value.get("traces").is_none());
        assert!(value.get("trace_summary").is_some());
    }

    #[test]
    fn duplicate_entry_ids_are_detected() {
        let mut evidence = base_evidence();
        evidence.transcript = vec![json!({ "entry_id": "e_1" }), json!({ "entry_id": "e_2" })];
        assert!(!evidence.has_duplicate_messages());
        evidence.transcript.push(json!({ "entry_id": "e_1" }));
        assert!(evidence.has_duplicate_messages());
    }
}
