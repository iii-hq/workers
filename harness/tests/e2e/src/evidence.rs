use std::collections::{HashMap, HashSet};

use harness::functions::metrics::SessionMetricsResponseV1;
use harness::functions::session_tree::SessionTreeResponseV1;
use harness::functions::status::StatusReport;
use harness::types::turn::TurnStatus;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::report::HardGateReport;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTranscript {
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub messages: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessExecutionEvidence {
    pub root: RootLifecycleEvidence,
    pub sessions: Vec<SessionLifecycleEvidence>,
    pub calls: Vec<FunctionCallEvidence>,
    pub timeline: Vec<TimelineEvent>,
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootLifecycleEvidence {
    pub session_id: String,
    pub terminal_turn_id: Option<String>,
    pub terminal_status: String,
    pub turn_count: u32,
    pub max_turns: u32,
    pub validation_retries: u32,
    pub transient_resumes: u32,
    pub pending_function_calls: usize,
    pub queued_messages: usize,
    pub expects_wake: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionLifecycleEvidence {
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub depth: u32,
    pub terminal: bool,
    pub status: String,
    pub turn_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCallEvidence {
    pub session_id: String,
    pub turn_id: Option<String>,
    pub call_id: Option<String>,
    pub transport_function_id: String,
    pub effective_function_id: String,
    pub arguments: Value,
    pub result: FunctionResultEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FunctionResultEvidence {
    Succeeded {
        result_entry_id: Option<String>,
    },
    Failed {
        result_entry_id: Option<String>,
        error: Option<String>,
    },
    Missing,
    Duplicate {
        count: usize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub sequence: u64,
    pub timestamp_ms: Option<u64>,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub entry_id: Option<String>,
    #[serde(flatten)]
    pub event: TimelineEventKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TimelineEventKind {
    Message,
    FunctionCall {
        call_id: Option<String>,
        function_id: String,
    },
    FunctionResult {
        call_id: Option<String>,
        function_id: String,
        is_error: bool,
    },
    ValidationNudge,
    WakeNotification,
    TriggerFired,
}

pub fn build_evidence(
    root_status: &StatusReport,
    tree: &SessionTreeResponseV1,
    transcripts: &[SessionTranscript],
    statuses: &HashMap<String, StatusReport>,
) -> HarnessExecutionEvidence {
    let sessions = tree
        .sessions
        .iter()
        .map(|node| {
            let status = statuses.get(&node.session_id);
            SessionLifecycleEvidence {
                session_id: node.session_id.clone(),
                parent_session_id: node.parent_session_id.clone(),
                depth: node.depth,
                terminal: status.is_some_and(is_terminal),
                status: status.map(status_name).unwrap_or_else(|| "missing".into()),
                turn_count: status.map(|status| status.turn_count),
            }
        })
        .collect::<Vec<_>>();
    let (calls, timeline) = normalize_transcripts(transcripts);
    let complete = tree.complete && sessions.iter().all(|session| session.terminal);
    HarnessExecutionEvidence {
        root: RootLifecycleEvidence {
            session_id: root_status.session_id.clone(),
            terminal_turn_id: root_status.turn_id.clone(),
            terminal_status: status_name(root_status),
            turn_count: root_status.turn_count,
            max_turns: root_status.max_turns,
            validation_retries: root_status.validation_retries,
            transient_resumes: root_status.transient_resumes,
            pending_function_calls: root_status.pending_function_calls.len(),
            queued_messages: root_status.queued.len(),
            expects_wake: root_status.expects_wake,
        },
        sessions,
        calls,
        timeline,
        complete,
    }
}

pub fn evaluate_structural_gates(
    metrics: &SessionMetricsResponseV1,
    tree: &SessionTreeResponseV1,
    evidence: &HarnessExecutionEvidence,
) -> Vec<HardGateReport> {
    let tree_complete =
        metrics.complete && tree.complete && evidence.sessions.iter().all(|s| s.terminal);
    let ownership_error = ownership_error(tree);
    let integrity_error = call_integrity_error(evidence);
    vec![
        gate(
            "harness.session_tree_complete",
            tree_complete,
            format!(
                "metrics_complete={}; tree_complete={}; terminal_sessions={}/{}",
                metrics.complete,
                tree.complete,
                evidence.sessions.iter().filter(|s| s.terminal).count(),
                evidence.sessions.len()
            ),
        ),
        gate(
            "harness.session_ownership",
            ownership_error.is_none(),
            ownership_error.unwrap_or_else(|| {
                format!("validated ownership for {} session(s)", tree.sessions.len())
            }),
        ),
        gate(
            "harness.call_result_integrity",
            integrity_error.is_none(),
            integrity_error
                .unwrap_or_else(|| format!("validated {} function call(s)", evidence.calls.len())),
        ),
    ]
}

fn gate(id: &str, passed: bool, reason: String) -> HardGateReport {
    HardGateReport {
        id: id.into(),
        passed,
        reason,
    }
}

fn is_terminal(status: &StatusReport) -> bool {
    matches!(
        status.status,
        TurnStatus::Completed | TurnStatus::Failed | TurnStatus::Cancelled
    ) && !status.expects_wake
}

fn status_name(status: &StatusReport) -> String {
    format!("{:?}", status.status).to_ascii_lowercase()
}

fn ownership_error(tree: &SessionTreeResponseV1) -> Option<String> {
    if tree.root_session_id.is_empty() {
        return Some("tree has an empty root session id".into());
    }
    let nodes = tree
        .sessions
        .iter()
        .map(|n| (n.session_id.as_str(), n))
        .collect::<HashMap<_, _>>();
    let Some(root) = nodes.get(tree.root_session_id.as_str()) else {
        return Some(format!(
            "root session {} is absent from tree",
            tree.root_session_id
        ));
    };
    if root.parent_session_id.is_some() || root.depth != 0 {
        return Some("root has a parent or non-zero depth".into());
    }
    if nodes.len() != tree.sessions.len() {
        return Some("tree contains duplicate session ids".into());
    }
    for node in &tree.sessions {
        if node.session_id == tree.root_session_id {
            continue;
        }
        let Some(parent_id) = node.parent_session_id.as_deref() else {
            return Some(format!("session {} has no parent", node.session_id));
        };
        let Some(parent) = nodes.get(parent_id) else {
            return Some(format!(
                "session {} references missing parent {parent_id}",
                node.session_id
            ));
        };
        if node.depth != parent.depth + 1 {
            return Some(format!(
                "session {} has depth {}, expected {}",
                node.session_id,
                node.depth,
                parent.depth + 1
            ));
        }
        let mut seen = HashSet::new();
        let mut current = Some(node);
        while let Some(item) = current {
            if !seen.insert(item.session_id.as_str()) {
                return Some(format!("cycle contains session {}", item.session_id));
            }
            current = item
                .parent_session_id
                .as_deref()
                .and_then(|id| nodes.get(id).copied());
        }
    }
    None
}

fn call_integrity_error(evidence: &HarnessExecutionEvidence) -> Option<String> {
    let bad = evidence
        .calls
        .iter()
        .filter(|call| {
            !matches!(
                call.result,
                FunctionResultEvidence::Succeeded { .. } | FunctionResultEvidence::Failed { .. }
            )
        })
        .count();
    let calls = evidence
        .calls
        .iter()
        .filter_map(|call| {
            call.call_id
                .as_ref()
                .map(|id| (call.session_id.as_str(), id.as_str()))
        })
        .collect::<HashSet<_>>();
    let orphaned = evidence
        .timeline
        .iter()
        .filter_map(|event| match &event.event {
            TimelineEventKind::FunctionResult {
                call_id: Some(id), ..
            } => Some((event.session_id.as_str(), id.as_str())),
            _ => None,
        })
        .filter(|key| !calls.contains(key))
        .count();
    if bad > 0 || orphaned > 0 {
        Some(format!(
            "{bad} call(s) have missing or duplicate results; {orphaned} result(s) are orphaned"
        ))
    } else {
        None
    }
}

fn normalize_transcripts(
    transcripts: &[SessionTranscript],
) -> (Vec<FunctionCallEvidence>, Vec<TimelineEvent>) {
    let mut calls = Vec::new();
    let mut timeline = Vec::new();
    let mut sequence = 0_u64;
    for transcript in transcripts {
        let entries = transcript
            .messages
            .get("messages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let mut results: HashMap<String, Vec<&Value>> = HashMap::new();
        for entry in &entries {
            if let Some(message) = entry.get("message") {
                if message.get("role").and_then(Value::as_str) == Some("function_result") {
                    if let Some(id) = message.get("function_call_id").and_then(Value::as_str) {
                        results.entry(id.to_string()).or_default().push(entry);
                    }
                }
            }
        }
        for entry in entries {
            let entry_id = entry
                .get("entry_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let turn_id = entry
                .get("turn_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let timestamp_ms = entry.get("timestamp_ms").and_then(Value::as_u64);
            let event = classify_entry(entry);
            timeline.push(TimelineEvent {
                sequence,
                timestamp_ms,
                session_id: transcript.session_id.clone(),
                turn_id: turn_id.clone(),
                entry_id: entry_id.clone(),
                event,
            });
            sequence += 1;
            let Some(message) = entry.get("message") else {
                continue;
            };
            if message.get("role").and_then(Value::as_str) != Some("assistant") {
                continue;
            }
            for block in message
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if block.get("type").and_then(Value::as_str) != Some("function_call") {
                    continue;
                }
                let Some(transport) = block.get("function_id").and_then(Value::as_str) else {
                    continue;
                };
                let call_id = block.get("id").and_then(Value::as_str).map(str::to_owned);
                let raw_arguments = block.get("arguments").cloned().unwrap_or_else(|| json!({}));
                let (effective, arguments) = if transport == "agent_trigger" {
                    (
                        raw_arguments
                            .get("function")
                            .and_then(Value::as_str)
                            .unwrap_or(transport)
                            .to_string(),
                        raw_arguments
                            .get("payload")
                            .cloned()
                            .unwrap_or_else(|| json!({})),
                    )
                } else {
                    (transport.to_string(), raw_arguments)
                };
                let matched = call_id
                    .as_ref()
                    .and_then(|id| results.get(id))
                    .cloned()
                    .unwrap_or_default();
                let result = match matched.as_slice() {
                    [] => FunctionResultEvidence::Missing,
                    [entry] => {
                        let result_message = &entry["message"];
                        let result_entry_id = entry
                            .get("entry_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        if result_message
                            .get("is_error")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                        {
                            FunctionResultEvidence::Failed {
                                result_entry_id,
                                error: result_message
                                    .get("error")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned),
                            }
                        } else {
                            FunctionResultEvidence::Succeeded { result_entry_id }
                        }
                    }
                    many => FunctionResultEvidence::Duplicate { count: many.len() },
                };
                calls.push(FunctionCallEvidence {
                    session_id: transcript.session_id.clone(),
                    turn_id: turn_id.clone(),
                    call_id,
                    transport_function_id: transport.into(),
                    effective_function_id: effective,
                    arguments,
                    result,
                });
            }
        }
    }
    (calls, timeline)
}

fn classify_entry(entry: &Value) -> TimelineEventKind {
    if entry
        .get("entry_id")
        .and_then(Value::as_str)
        .is_some_and(|id| id.contains("_nudge_"))
        || entry.pointer("/origin/validation").and_then(Value::as_bool) == Some(true)
    {
        return TimelineEventKind::ValidationNudge;
    }
    if entry.pointer("/custom/custom_type").and_then(Value::as_str) == Some("trigger_fired") {
        return TimelineEventKind::TriggerFired;
    }
    let Some(message) = entry.get("message") else {
        return TimelineEventKind::Message;
    };
    if message.get("role").and_then(Value::as_str) == Some("function_result") {
        return TimelineEventKind::FunctionResult {
            call_id: message
                .get("function_call_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            function_id: message
                .get("function_id")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .into(),
            is_error: message
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        };
    }
    if message.get("role").and_then(Value::as_str) == Some("assistant") {
        if let Some(block) = message
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|b| b.get("type").and_then(Value::as_str) == Some("function_call"))
        {
            return TimelineEventKind::FunctionCall {
                call_id: block.get("id").and_then(Value::as_str).map(str::to_owned),
                function_id: block
                    .get("function_id")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .into(),
            };
        }
    }
    TimelineEventKind::Message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transcript(messages: Value) -> SessionTranscript {
        SessionTranscript {
            session_id: "s".into(),
            parent_session_id: None,
            messages,
        }
    }

    #[test]
    fn normalizes_agent_trigger_and_pairs_result() {
        let input = transcript(json!({"messages":[
            {"entry_id":"call-entry","message":{"role":"assistant","content":[{"type":"function_call","id":"c1","function_id":"agent_trigger","arguments":{"function":"state::set","payload":{"key":"k"}}}]}},
            {"entry_id":"result-entry","message":{"role":"function_result","function_call_id":"c1","function_id":"state::set","is_error":false}}
        ]}));
        let (calls, _) = normalize_transcripts(&[input]);
        assert_eq!(calls[0].transport_function_id, "agent_trigger");
        assert_eq!(calls[0].effective_function_id, "state::set");
        assert!(matches!(
            calls[0].result,
            FunctionResultEvidence::Succeeded { .. }
        ));
    }

    #[test]
    fn detects_missing_and_duplicate_results() {
        let input = transcript(json!({"messages":[
            {"message":{"role":"assistant","content":[{"type":"function_call","id":"missing","function_id":"x","arguments":{}},{"type":"function_call","id":"duplicate","function_id":"y","arguments":{}}]}},
            {"message":{"role":"function_result","function_call_id":"duplicate","function_id":"y","is_error":false}},
            {"message":{"role":"function_result","function_call_id":"duplicate","function_id":"y","is_error":false}}
        ]}));
        let (calls, _) = normalize_transcripts(&[input]);
        assert!(matches!(calls[0].result, FunctionResultEvidence::Missing));
        assert!(matches!(
            calls[1].result,
            FunctionResultEvidence::Duplicate { count: 2 }
        ));
    }

    #[test]
    fn sequence_is_stable_when_timestamps_match() {
        let input = transcript(
            json!({"messages":[{"timestamp_ms":1,"message":{"role":"user"}},{"timestamp_ms":1,"message":{"role":"assistant","content":[]}}]}),
        );
        let (_, timeline) = normalize_transcripts(&[input]);
        assert_eq!(
            timeline
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
    }
}
