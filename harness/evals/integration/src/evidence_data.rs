//! The collected run dataset handed to scenario `verify` functions.
//!
//! [`RunEvidence`] is everything a run produced, exactly as persisted to the
//! artifact directory: verifying twice over the same evidence is
//! byte-deterministic. Accessors expose the recurring read patterns; scenario
//! authors combine them with plain Rust assertions.

use serde::Serialize;
use serde_json::Value;

use crate::types::recorder::{RecorderEventKind, RecorderEventV1};

/// Everything a scenario's `verify` function may look at.
#[derive(Debug, Clone, Serialize)]
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
    pub recorder_events: Vec<RecorderEventV1>,
}

impl RunEvidence {
    /// Concatenated text blocks of each assistant message, in transcript
    /// order. Assistant messages without a text block (pure function-call
    /// messages) contribute no entry.
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

    /// Recorded executions of the controlled function registered under
    /// `alias` (recorder function ids are `<run_id>::<alias>`).
    pub fn calls(&self, alias: &str) -> Vec<&RecorderEventV1> {
        let function_id = format!("{}::{alias}", self.run_id);
        self.recorder_events
            .iter()
            .filter(|event| {
                event.kind == RecorderEventKind::TargetCall && event.function_id == function_id
            })
            .collect()
    }

    /// Every `harness::turn-completed` delivery, in receipt order.
    pub fn lifecycle_events(&self) -> Vec<&RecorderEventV1> {
        self.recorder_events
            .iter()
            .filter(|event| event.kind == RecorderEventKind::Lifecycle)
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
        anyhow::ensure!(
            call.payload == expected,
            "{alias} payload {} != {expected}",
            call.payload
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

    /// Replace this run's concrete ids with `{{run_id}}` / `{{session_id}}` /
    /// `{{turn_id}}` placeholders so persisted failure text stays
    /// byte-comparable across runs.
    pub fn scrub(&self, text: &str) -> String {
        let mut text = text.to_string();
        replace_identity(&mut text, &self.run_id, "{{run_id}}");
        replace_identity(&mut text, &self.session_id, "{{session_id}}");
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
    use serde_json::json;

    use crate::types::script::SchemaVersion1;

    use super::*;

    fn event(kind: RecorderEventKind, function_id: &str, payload: Value) -> RecorderEventV1 {
        RecorderEventV1 {
            schema_version: SchemaVersion1::V1,
            run_id: "r".into(),
            sequence: 1,
            kind,
            function_id: function_id.into(),
            payload,
            received_at: "2026-07-15T00:00:00Z".into(),
        }
    }

    fn base_evidence() -> RunEvidence {
        RunEvidence {
            run_id: "r".into(),
            session_id: "s_1".into(),
            turn_id: Some("t_1".into()),
            send_response: Some(json!({
                "session_id": "s_1",
                "turn_id": "t_1",
                "accepted": true
            })),
            status: json!({
                "status": "completed",
                "pending_function_calls": [],
                "children": []
            }),
            transcript: vec![],
            generations_consumed: 1,
            generations_total: 1,
            recorder_events: vec![],
        }
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
    fn calls_select_only_the_aliased_target_events() {
        let mut evidence = base_evidence();
        let target = || {
            event(
                RecorderEventKind::TargetCall,
                "r::record",
                json!({ "value": "expected" }),
            )
        };
        evidence.recorder_events = vec![
            target(),
            target(),
            event(RecorderEventKind::TargetCall, "r::other", json!({})),
            event(
                RecorderEventKind::Lifecycle,
                "integration-recorder::lifecycle",
                json!({}),
            ),
        ];
        // Duplicate executions stay visible: exactly-once checks read the length.
        assert_eq!(evidence.calls("record").len(), 2);
        assert_eq!(evidence.calls("other").len(), 1);
        assert_eq!(evidence.calls("missing").len(), 0);
        assert_eq!(evidence.lifecycle_events().len(), 1);
    }

    #[test]
    fn message_counts_and_assistant_texts_read_the_transcript() {
        let mut evidence = base_evidence();
        evidence.transcript = vec![
            json!({ "message": { "role": "user", "content": [] } }),
            json!({
                "message": {
                    "role": "assistant",
                    "content": [{ "type": "function_call", "id": "call-1" }]
                }
            }),
            json!({
                "message": { "role": "function_result", "function_call_id": "call-1" }
            }),
            json!({
                "message": {
                    "role": "assistant",
                    "content": [
                        { "type": "text", "text": "recorded " },
                        { "type": "text", "text": "once" }
                    ]
                }
            }),
        ];
        assert_eq!(evidence.message_counts(), (1, 2, 1));
        // The function-call-only assistant message contributes no text entry.
        assert_eq!(evidence.assistant_texts(), ["recorded once"]);
    }

    #[test]
    fn scrub_replaces_every_run_scoped_id() {
        let mut evidence = base_evidence();
        evidence.run_id = "ir0011aabbcc".into();
        assert_eq!(
            evidence.scrub("ir0011aabbcc::record payload for s_1 in t_1 and again t_1"),
            "{{run_id}}::record payload for {{session_id}} in {{turn_id}} and again {{turn_id}}"
        );

        evidence.turn_id = None;
        assert_eq!(evidence.scrub("turn t_1"), "turn t_1");
    }

    #[test]
    fn expectation_helpers_report_the_observed_values() {
        let mut evidence = base_evidence();
        evidence.transcript = vec![
            json!({ "message": { "role": "user", "content": [] } }),
            json!({
                "message": {
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "complete" }]
                }
            }),
        ];
        evidence.expect_assistant_texts(["complete"]).unwrap();
        evidence.expect_message_counts(1, 1, 0).unwrap();
        evidence.expect_no_duplicate_messages().unwrap();
        assert!(evidence
            .expect_assistant_texts(["different"])
            .unwrap_err()
            .to_string()
            .contains("complete"));
    }
}
