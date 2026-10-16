//! Wire copies of the harness transcript and of the `context::assemble`
//! request/response. Every struct keeps the fields this worker does not read
//! in a flattened `rest` map, so a message or request passes through
//! `context-clm` and on to `context-manager` without losing anything
//! (signatures, usage, details, `parts`, future fields).
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One content block (`text`, `thinking`, `function_call`, `image`, ...).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Block {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

impl Block {
    /// The text of a `text` block; `None` for every other kind.
    pub fn text(&self) -> Option<&str> {
        if self.kind != "text" {
            return None;
        }
        self.rest.get("text").and_then(Value::as_str)
    }

    pub fn new_text(s: impl Into<String>) -> Block {
        let mut rest = Map::new();
        rest.insert("text".into(), Value::String(s.into()));
        Block {
            kind: "text".into(),
            rest,
        }
    }
}

/// One transcript message (`user`, `assistant`, `function_result`, `custom`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Msg {
    pub role: String,
    #[serde(default)]
    pub content: Vec<Block>,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

impl Msg {
    /// An assistant message that asks for at least one function call.
    pub fn is_assistant_call(&self) -> bool {
        self.role == "assistant" && self.content.iter().any(|b| b.kind == "function_call")
    }

    /// Ids of this message's `function_call` blocks, in order.
    pub fn call_ids(&self) -> Vec<String> {
        self.content
            .iter()
            .filter(|b| b.kind == "function_call")
            .filter_map(|b| b.rest.get("id").and_then(Value::as_str))
            .map(str::to_owned)
            .collect()
    }

    /// The call a `function_result` message answers.
    pub fn function_call_id(&self) -> Option<&str> {
        self.rest.get("function_call_id").and_then(Value::as_str)
    }

    /// The concatenated text blocks (a function result's output).
    pub fn result_text(&self) -> String {
        self.content.iter().filter_map(Block::text).collect()
    }
}

/// The target model, as the harness names it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelRef {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// `options` of an assemble request. The harness-side keys this worker reads
/// are typed; every other key (`allow_compaction`, `reserved_tokens`, ...)
/// rides along in `rest`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AssembleOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lease_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_overhead_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_prune: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesystem_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u64>,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

/// `context-clm::assemble` request: the `context::assemble` contract.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AssembleRequest {
    pub messages: Vec<Msg>,
    pub model: ModelRef,
    #[serde(default)]
    pub options: AssembleOptions,
    #[serde(default)]
    pub tools: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

/// `context-clm::assemble` response: `context::assemble`'s response, with
/// `applied.strategy_stop` set when the strategy ends the run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AssembleResponse {
    pub system_prompt: String,
    pub messages: Vec<Msg>,
    pub token_count: u64,
    pub usable: u64,
    pub effective_max_output_tokens: u64,
    /// Never sent as `null`: the harness reads it into a struct.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub applied: Value,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}
