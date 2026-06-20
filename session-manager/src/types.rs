//! Cross-cutting wire contracts shared by the agentic worker family.
//!
//! These shapes mirror the spec's "Cross-cutting contracts"
//! (tech-specs/2026-06-agentic/README.md): `ContentBlock`,
//! `AgentMessage`, `SessionEntry`, `SessionMeta`. They are the exact
//! JSON shapes stored and emitted by this worker — serde renames keep
//! the wire format byte-compatible with the TypeScript definitions.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON object used for app-defined metadata / origin correlation.
pub type JsonMap = serde_json::Map<String, Value>;

/// Subset-equality metadata match used by `session::list` and every
/// trigger config: every key in `want` must equal (deep JSON equality)
/// the value stored under the same key in `have`. An empty `want`
/// matches everything; a non-empty `want` never matches a missing map.
pub fn metadata_matches(want: &JsonMap, have: Option<&JsonMap>) -> bool {
    want.iter()
        .all(|(key, value)| have.and_then(|m| m.get(key)) == Some(value))
}

/// Message role discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    FunctionResult,
    Custom,
}

/// Why an assistant message stopped generating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    End,
    Length,
    FunctionCall,
    Aborted,
    Error,
}

/// Coarse error classification carried by failed assistant messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    AuthExpired,
    RateLimited,
    ContextOverflow,
    Transient,
    Permanent,
}

/// Token / cost accounting reported by providers.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Usage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// The atomic unit of message content. A message's `content` is an
/// ordered array of these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        /// MIME type, e.g. `image/png`.
        mime: String,
        /// Base64-encoded image bytes.
        data: String,
    },
    Thinking {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    FunctionCall {
        /// Unique per call, echoed by the result.
        id: String,
        /// The iii function id to invoke.
        function_id: String,
        /// Model-produced arguments (JSON).
        #[serde(default)]
        arguments: Value,
    },
    FunctionResult {
        function_call_id: String,
        content: Vec<ContentBlock>,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

/// The canonical transcript message union, discriminated by `role`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum AgentMessage {
    User {
        content: Vec<ContentBlock>,
        /// Milliseconds since epoch.
        timestamp: i64,
    },
    Assistant {
        content: Vec<ContentBlock>,
        stop_reason: StopReason,
        /// Provider's raw finish reason, passed through untouched.
        #[serde(skip_serializing_if = "Option::is_none")]
        native_stop_reason: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_kind: Option<ErrorKind>,
        /// Report-and-continue notices (e.g. dropped unsupported param).
        #[serde(skip_serializing_if = "Option::is_none")]
        warnings: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        model: String,
        provider: String,
        /// Milliseconds since epoch.
        timestamp: i64,
    },
    FunctionResult {
        /// Echoes the `function_call` block id this result answers.
        function_call_id: String,
        function_id: String,
        content: Vec<ContentBlock>,
        /// Opaque structured payload kept alongside the rendered content.
        #[serde(default)]
        details: Value,
        #[serde(default)]
        is_error: bool,
        /// Milliseconds since epoch.
        timestamp: i64,
    },
    /// Escape hatch for app-specific transcript items (system notices,
    /// UI markers, attachments, ...).
    Custom {
        /// App-defined discriminator.
        custom_type: String,
        content: Vec<ContentBlock>,
        #[serde(skip_serializing_if = "Option::is_none")]
        display: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        details: Option<Value>,
        /// Milliseconds since epoch.
        timestamp: i64,
    },
}

impl AgentMessage {
    pub fn role(&self) -> Role {
        match self {
            AgentMessage::User { .. } => Role::User,
            AgentMessage::Assistant { .. } => Role::Assistant,
            AgentMessage::FunctionResult { .. } => Role::FunctionResult,
            AgentMessage::Custom { .. } => Role::Custom,
        }
    }

    pub fn content(&self) -> &[ContentBlock] {
        match self {
            AgentMessage::User { content, .. }
            | AgentMessage::Assistant { content, .. }
            | AgentMessage::FunctionResult { content, .. }
            | AgentMessage::Custom { content, .. } => content,
        }
    }

    pub fn set_content(&mut self, new_content: Vec<ContentBlock>) {
        match self {
            AgentMessage::User { content, .. }
            | AgentMessage::Assistant { content, .. }
            | AgentMessage::FunctionResult { content, .. }
            | AgentMessage::Custom { content, .. } => *content = new_content,
        }
    }
}

/// Bookkeeping payload of a `kind: "custom"` session entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CustomPayload {
    /// App-defined discriminator (e.g. `"compaction"`).
    pub custom_type: String,
    /// Opaque app data.
    #[serde(default)]
    pub data: Value,
}

/// The entry envelope giving each stored item identity, ordering and a
/// parent link (used for forking), discriminated by `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionEntry {
    /// A transcript message.
    Message {
        id: String,
        parent_id: Option<String>,
        /// Milliseconds since epoch (entry creation time).
        timestamp: i64,
        /// Starts at 0; increments on every content update.
        #[serde(default)]
        revision: u64,
        /// Opaque writer-supplied correlation (e.g. `{ turn_id }`).
        #[serde(skip_serializing_if = "Option::is_none")]
        origin: Option<JsonMap>,
        message: AgentMessage,
    },
    /// Bookkeeping *about* the conversation that is not a message at
    /// all (e.g. the harness's compaction record).
    Custom {
        id: String,
        parent_id: Option<String>,
        /// Milliseconds since epoch (entry creation time).
        timestamp: i64,
        /// Starts at 0; increments on every content update.
        #[serde(default)]
        revision: u64,
        /// Opaque writer-supplied correlation (e.g. `{ turn_id }`).
        #[serde(skip_serializing_if = "Option::is_none")]
        origin: Option<JsonMap>,
        custom_type: String,
        #[serde(default)]
        data: Value,
    },
}

impl SessionEntry {
    pub fn id(&self) -> &str {
        match self {
            SessionEntry::Message { id, .. } | SessionEntry::Custom { id, .. } => id,
        }
    }

    pub fn parent_id(&self) -> Option<&str> {
        match self {
            SessionEntry::Message { parent_id, .. } | SessionEntry::Custom { parent_id, .. } => {
                parent_id.as_deref()
            }
        }
    }

    pub fn timestamp(&self) -> i64 {
        match self {
            SessionEntry::Message { timestamp, .. } | SessionEntry::Custom { timestamp, .. } => {
                *timestamp
            }
        }
    }

    pub fn revision(&self) -> u64 {
        match self {
            SessionEntry::Message { revision, .. } | SessionEntry::Custom { revision, .. } => {
                *revision
            }
        }
    }

    pub fn origin(&self) -> Option<&JsonMap> {
        match self {
            SessionEntry::Message { origin, .. } | SessionEntry::Custom { origin, .. } => {
                origin.as_ref()
            }
        }
    }

    pub fn is_message(&self) -> bool {
        matches!(self, SessionEntry::Message { .. })
    }

    /// Role of the wrapped message, when this is a `kind: "message"` entry.
    pub fn role(&self) -> Option<Role> {
        match self {
            SessionEntry::Message { message, .. } => Some(message.role()),
            SessionEntry::Custom { .. } => None,
        }
    }
}

/// Coarse session lifecycle status, rendered directly by consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Idle,
    Working,
    /// Parked mid-turn awaiting a user decision (e.g. the harness reached
    /// `max_turns` and is asking whether to continue).
    Waiting,
    Done,
    Error,
}

/// A session's metadata record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionMeta {
    pub session_id: String,
    pub title: String,
    pub description: String,
    pub status: SessionStatus,
    /// Short cause, set on `error`, cleared on any other status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    /// App-defined; the tenancy hook (e.g. `{ "owner": "u_1" }`) that
    /// `session::list` and every trigger config can filter on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonMap>,
    /// Source session id when created by `session::fork`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<String>,
    /// Milliseconds since epoch.
    pub created_at: i64,
    /// Milliseconds since epoch.
    pub updated_at: i64,
    /// Number of `kind: "message"` entries (custom entries are
    /// bookkeeping and not counted).
    pub message_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn content_block_wire_shapes_match_spec() {
        let blocks: Vec<ContentBlock> = serde_json::from_value(json!([
            { "type": "text", "text": "hi" },
            { "type": "image", "mime": "image/png", "data": "aGk=" },
            { "type": "thinking", "text": "hmm", "signature": "sig" },
            { "type": "function_call", "id": "c1", "function_id": "f::g", "arguments": { "a": 1 } },
            { "type": "function_result", "function_call_id": "c1",
              "content": [{ "type": "text", "text": "out" }], "is_error": false }
        ]))
        .unwrap();
        assert_eq!(blocks.len(), 5);
        let round = serde_json::to_value(&blocks).unwrap();
        assert_eq!(round[0], json!({ "type": "text", "text": "hi" }));
        assert_eq!(round[3]["function_id"], "f::g");
    }

    #[test]
    fn agent_message_roundtrip_all_roles() {
        let msgs: Vec<AgentMessage> = serde_json::from_value(json!([
            { "role": "user", "content": [{ "type": "text", "text": "q" }], "timestamp": 1 },
            { "role": "assistant", "content": [], "stop_reason": "end",
              "model": "m1", "provider": "p1", "timestamp": 2 },
            { "role": "function_result", "function_call_id": "c1", "function_id": "f::g",
              "content": [], "details": { "x": 1 }, "is_error": false, "timestamp": 3 },
            { "role": "custom", "custom_type": "notice", "content": [], "timestamp": 4 }
        ]))
        .unwrap();
        assert_eq!(msgs[0].role(), Role::User);
        assert_eq!(msgs[1].role(), Role::Assistant);
        assert_eq!(msgs[2].role(), Role::FunctionResult);
        assert_eq!(msgs[3].role(), Role::Custom);

        let round = serde_json::to_value(&msgs).unwrap();
        assert_eq!(round[1]["stop_reason"], "end");
        // Optional assistant fields absent from input stay absent.
        assert!(round[1].get("error_message").is_none());
        assert_eq!(round[2]["details"], json!({ "x": 1 }));
    }

    #[test]
    fn session_entry_kind_discrimination() {
        let entry: SessionEntry = serde_json::from_value(json!({
            "kind": "custom", "id": "e1", "parent_id": null, "timestamp": 5,
            "revision": 0, "custom_type": "compaction", "data": { "summary": "s" }
        }))
        .unwrap();
        assert!(!entry.is_message());
        assert_eq!(entry.id(), "e1");
        assert_eq!(entry.role(), None);

        let msg_entry: SessionEntry = serde_json::from_value(json!({
            "kind": "message", "id": "e2", "parent_id": "e1", "timestamp": 6, "revision": 1,
            "origin": { "turn_id": "t1" },
            "message": { "role": "user", "content": [], "timestamp": 6 }
        }))
        .unwrap();
        assert_eq!(msg_entry.parent_id(), Some("e1"));
        assert_eq!(msg_entry.revision(), 1);
        assert_eq!(msg_entry.role(), Some(Role::User));
    }

    #[test]
    fn session_meta_omits_absent_optionals() {
        let meta = SessionMeta {
            session_id: "s1".into(),
            title: String::new(),
            description: String::new(),
            status: SessionStatus::Idle,
            status_reason: None,
            metadata: None,
            forked_from: None,
            created_at: 1,
            updated_at: 1,
            message_count: 0,
        };
        let v = serde_json::to_value(&meta).unwrap();
        assert_eq!(v["status"], "idle");
        assert!(v.get("status_reason").is_none());
        assert!(v.get("metadata").is_none());
        assert!(v.get("forked_from").is_none());
    }
}
