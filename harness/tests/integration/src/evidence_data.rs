//! Collected data handed to scenario `verify` functions.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{json, Value};

use crate::canonical::{canonical_json, sha256_of_bytes};
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
    /// Runner-side intervention evidence, when the fixture drives a
    /// synchronized mid-flight operation.
    pub control: Value,
    /// Durable status reports for the root and any descendant sessions.
    pub tree_sessions: Vec<String>,
    pub tree_statuses: Vec<Value>,
    /// Raw scripted-router calls and abort acknowledgements.
    pub router_evidence: Value,
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
    pub control: &'a Value,
    pub tree_sessions: &'a [String],
    pub tree_statuses: &'a [Value],
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
            control: &self.control,
            tree_sessions: &self.tree_sessions,
            tree_statuses: &self.tree_statuses,
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

    /// `(request_id, what)` wherever a router request's bound prefix is not an
    /// extension of the same session's previous request: `system_prompt`,
    /// `system_sections`, `tools`, and every message that request carried
    /// (minus the wire-invisible `timestamp`, and `details` unless a denied
    /// result's, which providers serialize into the result text). Models that
    /// bind thinking to its prefix (Opus 5.5, Fable 5.1) reject or drop on any
    /// such edit.
    pub fn append_only_violations(&self) -> Vec<(String, String)> {
        let bound = |message: &Value| {
            let mut message = message.clone();
            if let Some(fields) = message.as_object_mut() {
                fields.remove("timestamp");
                let status = fields.get("details").and_then(|d| d.get("status"));
                if status.and_then(Value::as_str) != Some("denied") {
                    fields.remove("details");
                }
            }
            message
        };
        let requests = self
            .router_evidence
            .get("calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|call| call.get("request"));
        let mut previous: BTreeMap<&str, &Value> = BTreeMap::new();
        let mut violations = Vec::new();
        for request in requests {
            let session = request["session_id"].as_str().unwrap_or_default();
            let Some(prev) = previous.insert(session, request) else {
                continue;
            };
            let id = request["request_id"].as_str().unwrap_or_default();
            for key in ["system_prompt", "system_sections", "tools"] {
                if prev[key] != request[key] {
                    violations.push((id.to_string(), format!("{key} changed")));
                }
            }
            let (sent, now) = (messages_of(prev), messages_of(request));
            if let Some(i) =
                (0..sent.len()).find(|&i| now.get(i).map(bound) != Some(bound(&sent[i])))
            {
                violations.push((id.to_string(), format!("messages[{i}] changed")));
            }
        }
        violations
    }

    pub fn expect_append_only_requests(&self) -> anyhow::Result<()> {
        let violations = self.append_only_violations();
        anyhow::ensure!(
            violations.is_empty(),
            "bound prefix edited between requests: {violations:?}"
        );
        Ok(())
    }

    /// Every assistant entry of the root session carries, as `origin.req`, the
    /// fingerprint (sha256 of the canonical `system_prompt`, `tools` and
    /// `messages`, plus the message count) of the router request that produced
    /// it, and names its `build`. The request is the raw one the scripted
    /// router received, so a transformation between the stamp and the call
    /// shows here.
    pub fn expect_request_fingerprints(&self) -> anyhow::Result<()> {
        let sha = |value: &Value| sha256_of_bytes(canonical_json(value).as_bytes());
        let mut checked = 0;
        let requests = self
            .router_evidence
            .get("calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|call| call["outcome"] == "matched")
            .filter_map(|call| call.get("request"))
            .filter(|request| request["session_id"] == self.session_id.as_str());
        for request in requests {
            let id = request["request_id"].as_str().unwrap_or_default();
            let (turn_id, step) = id
                .rsplit_once(':')
                .ok_or_else(|| anyhow::anyhow!("request_id {id:?} is not <turn_id>:<step>"))?;
            let entry_id = format!("e_{turn_id}_{step}_assistant");
            let origin = self
                .transcript
                .iter()
                .find(|item| item["entry_id"] == entry_id.as_str())
                .map(|item| &item["origin"])
                .ok_or_else(|| anyhow::anyhow!("no assistant entry {entry_id} for {id}"))?;
            let req = json!({
                "system_sha": sha(&request["system_prompt"]),
                "tools_sha": sha(&request["tools"]),
                "messages_sha": sha(&request["messages"]),
                "n": request["messages"].as_array().map_or(0, Vec::len),
            });
            anyhow::ensure!(
                origin["req"] == req,
                "{entry_id}: origin.req {} is not the fingerprint of the request sent for {id}: {req}",
                origin["req"]
            );
            anyhow::ensure!(
                origin["build"]
                    .as_str()
                    .is_some_and(|build| !build.is_empty()),
                "{entry_id}: origin.build is missing"
            );
            checked += 1;
        }
        anyhow::ensure!(checked > 0, "no router request to fingerprint");
        Ok(())
    }

    /// The raw router requests of every step-0 generation (`request_id` ends in
    /// `:0`): each turn's opening call, across sessions, in arrival order.
    pub fn step_zero_requests(&self) -> Vec<Value> {
        self.router_evidence
            .get("calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|call| call.get("request").cloned())
            .filter(|request| {
                request["request_id"]
                    .as_str()
                    .is_some_and(|id| id.ends_with(":0"))
            })
            .collect()
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
        for session_id in &self.tree_sessions {
            replace_identity(&mut text, session_id, "{{session_id}}");
        }
        for status in &self.tree_statuses {
            if let Some(session_id) = status.get("session_id").and_then(Value::as_str) {
                replace_identity(&mut text, session_id, "{{session_id}}");
            }
            if let Some(turn_id) = status.get("turn_id").and_then(Value::as_str) {
                replace_identity(&mut text, turn_id, "{{turn_id}}");
            }
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

fn messages_of(request: &Value) -> &[Value] {
    request["messages"].as_array().map_or(&[], Vec::as_slice)
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
            control: Value::Null,
            tree_sessions: Vec::new(),
            tree_statuses: Vec::new(),
            router_evidence: Value::Null,
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

    #[test]
    fn append_only_violations_ignore_wire_invisible_fields_and_other_sessions() {
        let request = |id: &str, session: &str, prompt: &str, messages: Value| {
            json!({ "request": {
                "request_id": id, "session_id": session, "system_prompt": prompt,
                "tools": [], "messages": messages
            } })
        };
        let mut evidence = base_evidence();
        evidence.router_evidence = json!({ "calls": [
            request("t_a:0", "s", "p", json!([{ "role": "user", "timestamp": 1 }])),
            request("t_c:0", "child", "other", json!([])),
            request("t_a:1", "s", "p", json!([
                { "role": "user", "timestamp": 2 },
                { "role": "function_result", "details": { "raw": 1 } }
            ])),
            request("t_a:2", "s", "p2", json!([
                { "role": "user" },
                { "role": "function_result", "is_error": true },
                { "role": "function_result", "details": { "status": "denied", "reason": "a" } }
            ])),
            // A denied result's details reach the wire in its text.
            request("t_a:3", "s", "p2", json!([
                { "role": "user" },
                { "role": "function_result", "is_error": true },
                { "role": "function_result", "details": { "status": "denied", "reason": "b" } }
            ])),
        ] });
        assert_eq!(
            evidence.append_only_violations(),
            [
                ("t_a:2".to_string(), "system_prompt changed".to_string()),
                ("t_a:2".to_string(), "messages[1] changed".to_string()),
                ("t_a:3".to_string(), "messages[2] changed".to_string()),
            ]
        );
    }

    #[test]
    fn request_fingerprints_must_match_the_request_the_router_received() {
        // sha256 of `null`, of `[]` and of the compact canonical messages: the
        // vectors the harness's own unit test pins on the writing side.
        const NULL: &str = "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b";
        const EMPTY: &str = "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945";
        const MESSAGES: &str = "4e79873118cd9be7a1f0308b9cd772950c5410c74ca3fe1ba2626cba009a9237";
        let mut evidence = base_evidence();
        evidence.router_evidence = json!({ "calls": [
            { "outcome": "matched", "request": {
                "request_id": "t1:0", "session_id": "session-1", "tools": [],
                "messages": [{ "role": "user", "content": "hi" }]
            } },
            // Another session's request and an unmatched call have no entry here.
            { "outcome": "matched", "request": {
                "request_id": "c1:0", "session_id": "child", "tools": [], "messages": []
            } },
            { "outcome": "unexpected_call", "request": {
                "request_id": "t1:9", "session_id": "session-1", "tools": [], "messages": []
            } },
        ] });
        let entry = |messages_sha: &str| {
            json!({ "entry_id": "e_t1_0_assistant", "origin": { "build": "abc", "req": {
                "system_sha": NULL, "tools_sha": EMPTY, "messages_sha": messages_sha, "n": 1
            } } })
        };

        evidence.transcript = vec![entry(MESSAGES)];
        evidence.expect_request_fingerprints().unwrap();

        evidence.transcript = vec![entry(EMPTY)];
        let error = evidence.expect_request_fingerprints().unwrap_err();
        assert!(error.to_string().contains("e_t1_0_assistant"), "{error}");

        evidence.transcript = Vec::new();
        assert!(evidence.expect_request_fingerprints().is_err());
    }
}
