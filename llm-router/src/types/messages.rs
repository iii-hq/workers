use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::content::ContentBlock;
use crate::types::events::{ErrorKind, StopReason, Usage};

/// Single-variant role tags: exact-match on deserialize, correct wire string on
/// serialize, and they let `AgentMessage` be an untagged union.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum UserRoleTag {
    #[serde(rename = "user")]
    User,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum AssistantRoleTag {
    #[serde(rename = "assistant")]
    Assistant,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum FunctionResultRoleTag {
    #[serde(rename = "function_result")]
    FunctionResult,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum CustomRoleTag {
    #[serde(rename = "custom")]
    Custom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UserMessage {
    pub role: UserRoleTag,
    pub content: Vec<ContentBlock>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AssistantMessage {
    pub role: AssistantRoleTag,
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_stop_reason: Option<String>, // provider's raw finish reason, untouched
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<ErrorKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>, // report-and-continue notices
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    pub model: String,
    pub provider: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FunctionResultMessage {
    pub role: FunctionResultRoleTag,
    pub function_call_id: String,
    pub function_id: String,
    pub content: Vec<ContentBlock>,
    pub details: serde_json::Value,
    pub is_error: bool,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CustomMessage {
    pub role: CustomRoleTag,
    pub custom_type: String, // app-defined discriminator
    pub content: Vec<ContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    pub timestamp: i64,
}

/// The canonical transcript message union. Untagged: the single-variant role
/// tags disambiguate deserialization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum AgentMessage {
    Assistant(AssistantMessage),
    FunctionResult(FunctionResultMessage),
    Custom(CustomMessage),
    User(UserMessage),
}

/// Reorder function results displaced behind interleaved user messages.
///
/// A notification or steering message injected while a call window is open
/// (e.g. a parked `harness::spawn`) lands between an assistant's
/// `function_call` and its `function_result` in the durable transcript. Every
/// provider wire format requires results directly after the emitting
/// assistant message (Anthropic 400: "tool_use ids were found without
/// tool_result blocks immediately after"), so each provider's wire mapper
/// runs this pass first: move every result up to directly follow its call's
/// assistant message, preserving relative result order and leaving everything
/// else in place. Results whose call is absent (compaction cut it) keep their
/// original position.
pub fn reorder_displaced_results(messages: &[AgentMessage]) -> Vec<&AgentMessage> {
    use std::collections::{HashMap, HashSet};
    let mut call_owner: HashMap<&str, usize> = HashMap::new();
    for (i, m) in messages.iter().enumerate() {
        if let AgentMessage::Assistant(a) = m {
            for b in &a.content {
                if let ContentBlock::FunctionCall { id, .. } = b {
                    call_owner.insert(id.as_str(), i);
                }
            }
        }
    }
    let mut attached: HashMap<usize, Vec<&AgentMessage>> = HashMap::new();
    let mut moved: HashSet<usize> = HashSet::new();
    for (i, m) in messages.iter().enumerate() {
        if let AgentMessage::FunctionResult(r) = m {
            if let Some(&owner) = call_owner.get(r.function_call_id.as_str()) {
                if owner < i {
                    attached.entry(owner).or_default().push(m);
                    moved.insert(i);
                }
            }
        }
    }
    if moved.is_empty() {
        return messages.iter().collect();
    }
    let mut out = Vec::with_capacity(messages.len());
    for (i, m) in messages.iter().enumerate() {
        if !moved.contains(&i) {
            out.push(m);
        }
        if let Some(results) = attached.remove(&i) {
            out.extend(results);
        }
    }
    out
}

/// Degrade unparseable tool-call arguments to a safe OBJECT placeholder.
///
/// Providers must never emit null or a bare string for a call's arguments —
/// a non-object `tool_use.input` is a hard Anthropic 400 once the turn is
/// replayed (sessions switch providers). Two-step recovery:
///
/// 1. Salvage the complete leading fields of a partial object — a call cut
///    mid-stream (`{"function":"state::set","payload":{"key":`) keeps its
///    known prefix (`{"function":"state::set"}`), so long-streaming calls
///    stay identifiable in UIs instead of rendering as an anonymous `{}`.
///    The salvaged object is stamped `"_partial": true` so dispatch layers
///    can refuse to execute partial intent (a truncated call must surface a
///    teachable error, not run with whatever fields happened to complete).
/// 2. Otherwise carry the malformed text as `{"_raw": <text ≤2KB>}` so the
///    evidence of what the model actually sent survives for rendering and
///    for the harness's teachable no-target error.
pub fn degraded_arguments(args_json: &str) -> serde_json::Value {
    if let Some(mut map) = salvage_leading_object_fields(args_json) {
        map.insert("_partial".to_string(), serde_json::Value::Bool(true));
        return serde_json::Value::Object(map);
    }
    serde_json::json!({ "_raw": utf8_head(args_json, 2048) })
}

/// The complete leading fields of a partial JSON object string, if any.
/// Scans only the first 4KB (a partial is rebuilt per stream event, so this
/// must stay O(head); leading fields live in the prefix), cutting at
/// top-level commas and taking the longest prefix that parses once closed.
pub fn salvage_leading_object_fields(
    args: &str,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let head = utf8_head(args, 4096);
    let bytes = head.as_bytes();
    let (mut depth, mut in_str, mut esc, mut started) = (0usize, false, false, false);
    let mut cuts: Vec<usize> = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        if esc {
            esc = false;
            continue;
        }
        match b {
            b'\\' if in_str => esc = true,
            b'"' => in_str = !in_str,
            _ if in_str => {}
            b'{' | b'[' => {
                depth += 1;
                started = true;
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            b',' if depth == 1 => cuts.push(i),
            _ => {}
        }
    }
    if !started {
        return None;
    }
    // Longest salvageable prefix wins; candidates are few (top-level commas).
    for &cut in cuts.iter().rev() {
        let candidate = format!("{}}}", &head[..cut]);
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str(&candidate) {
            if !map.is_empty() {
                return Some(map);
            }
        }
    }
    None
}

fn utf8_head(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degraded_arguments_salvages_leading_fields_or_keeps_raw() {
        // Mid-stream cut: the known prefix survives as a real object, marked
        // partial so it renders but never executes.
        assert_eq!(
            degraded_arguments(r#"{"function":"state::set","payload":{"key":"art"#),
            serde_json::json!({ "function": "state::set", "_partial": true })
        );
        // Longest complete prefix wins, nested commas/strings don't cut.
        assert_eq!(
            degraded_arguments(
                r#"{"function":"a::b","payload":{"x":"1,2","y":[3,4]},"extra":{"cut":"#
            ),
            serde_json::json!({ "function": "a::b", "payload": { "x": "1,2", "y": [3, 4] }, "_partial": true })
        );
        // Nothing salvageable (no complete top-level field yet, or not JSON):
        // the raw text survives as evidence, always inside an object.
        assert_eq!(
            degraded_arguments(r#"{"function":"state::se"#),
            serde_json::json!({ "_raw": r#"{"function":"state::se"# })
        );
        assert_eq!(
            degraded_arguments("{'key': 'v'}"),
            serde_json::json!({ "_raw": "{'key': 'v'}" })
        );
    }
    use crate::types::events::StopReason;

    fn assistant(content: Vec<ContentBlock>) -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            role: AssistantRoleTag::Assistant,
            content,
            stop_reason: StopReason::End,
            native_stop_reason: None,
            error_message: None,
            error_kind: None,
            warnings: None,
            usage: None,
            model: "m".into(),
            provider: "p".into(),
            timestamp: 1,
        })
    }
    fn user_text(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage {
            role: UserRoleTag::User,
            content: vec![ContentBlock::Text { text: text.into() }],
            timestamp: 2,
        })
    }
    fn result(id: &str) -> AgentMessage {
        AgentMessage::FunctionResult(FunctionResultMessage {
            role: FunctionResultRoleTag::FunctionResult,
            function_call_id: id.into(),
            function_id: "f".into(),
            content: vec![],
            details: serde_json::Value::Null,
            is_error: false,
            timestamp: 3,
        })
    }
    fn call(id: &str) -> ContentBlock {
        ContentBlock::FunctionCall {
            id: id.into(),
            function_id: "f".into(),
            arguments: serde_json::json!({}),
        }
    }
    fn is_result(m: &AgentMessage, id: &str) -> bool {
        matches!(m, AgentMessage::FunctionResult(r) if r.function_call_id == id)
    }

    #[test]
    fn displaced_result_moves_directly_after_its_assistant() {
        let msgs = vec![
            assistant(vec![call("t1")]),
            user_text("[notification] progress"),
            result("t1"),
        ];
        let out = reorder_displaced_results(&msgs);
        assert_eq!(out.len(), 3);
        assert!(matches!(out[0], AgentMessage::Assistant(_)));
        assert!(is_result(out[1], "t1"));
        assert!(matches!(out[2], AgentMessage::User(_)));
    }

    #[test]
    fn adjacent_results_and_unmatched_results_keep_positions() {
        let msgs = vec![
            assistant(vec![call("t1"), call("t2")]),
            result("t1"),
            result("t2"),
            result("orphan"),
            user_text("hi"),
        ];
        let out = reorder_displaced_results(&msgs);
        assert!(is_result(out[1], "t1"));
        assert!(is_result(out[2], "t2"));
        assert!(is_result(out[3], "orphan"));
        assert!(matches!(out[4], AgentMessage::User(_)));
    }

    #[test]
    fn multi_owner_displaced_results_each_return_to_their_assistant() {
        // Two assistants with open calls, results displaced across interleaved
        // user messages — including a result for assistant #1 arriving after
        // assistant #2. Each result must land behind ITS OWN assistant, in
        // call order, with everything else keeping relative order.
        let msgs = vec![
            assistant(vec![call("t1"), call("t2")]),
            user_text("n1"),
            result("t1"),
            assistant(vec![call("t3")]),
            user_text("n2"),
            result("t3"),
            result("t2"),
        ];
        let out = reorder_displaced_results(&msgs);
        assert_eq!(out.len(), 7);
        assert!(matches!(out[0], AgentMessage::Assistant(_)));
        assert!(is_result(out[1], "t1"));
        assert!(is_result(out[2], "t2"));
        assert!(matches!(out[3], AgentMessage::User(_)));
        assert!(matches!(out[4], AgentMessage::Assistant(_)));
        assert!(is_result(out[5], "t3"));
        assert!(matches!(out[6], AgentMessage::User(_)));
    }
}
