use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Typed oracle vocabulary. Common send/completion/lifecycle/router checks
/// have defaults, leaving each authored scenario to state only
/// scenario-specific facts.
///
/// The expectation root is authored-only and never serialized. The leaf
/// structs that parameterize compiled invariants
/// ([`MessageCountsExpectationV1`], [`SendFlagsExpectationV1`],
/// [`TerminalExpectationV1`], [`LifecycleExpectationV1`]) keep wire derives.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpectationsV1 {
    pub message_counts: Option<MessageCountsExpectationV1>,
    pub assistant_text: Option<String>,
    pub function_results: Vec<FunctionResultExpectationV1>,
    pub calls_closed: bool,
    pub calls: Vec<TargetCallsExpectationV1>,
    pub send_flags: SendFlagsExpectationV1,
    pub no_duplicates: bool,
    pub terminal: TerminalExpectationV1,
    pub lifecycle: LifecycleExpectationV1,
}

impl Default for ExpectationsV1 {
    fn default() -> Self {
        Self {
            message_counts: None,
            assistant_text: None,
            function_results: Vec::new(),
            calls_closed: false,
            calls: Vec::new(),
            send_flags: SendFlagsExpectationV1::default(),
            no_duplicates: true,
            terminal: TerminalExpectationV1::default(),
            lifecycle: LifecycleExpectationV1::default(),
        }
    }
}

impl ExpectationsV1 {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MessageCountsExpectationV1 {
    pub user: u64,
    pub assistant: u64,
    pub function_result: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionResultExpectationV1 {
    pub function_call_id: String,
    /// Optional function alias; omitted when any result closing the call is
    /// acceptable (for example, a synthesized crash-recovery error).
    pub function: Option<String>,
    pub content: Option<Vec<Value>>,
    pub is_error: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TargetCallsExpectationV1 {
    pub function: String,
    pub count: u64,
    pub payload: Option<Value>,
    pub payload_subset: Option<Value>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SendFlagsExpectationV1 {
    #[serde(default)]
    pub merged: bool,
    #[serde(default)]
    pub queued: bool,
    #[serde(default)]
    pub deduplicated: bool,
}

impl SendFlagsExpectationV1 {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TerminalExpectationV1 {
    pub status: TerminalStatusV1,
    pub pending_calls: u64,
    pub queued_messages: u64,
}

impl Default for TerminalExpectationV1 {
    fn default() -> Self {
        Self {
            status: TerminalStatusV1::Completed,
            pending_calls: 0,
            queued_messages: 0,
        }
    }
}

impl TerminalExpectationV1 {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TerminalStatusV1 {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LifecycleExpectationV1 {
    #[serde(default = "default_true")]
    pub allow_identical_duplicates: bool,
}

impl Default for LifecycleExpectationV1 {
    fn default() -> Self {
        Self {
            allow_identical_duplicates: true,
        }
    }
}

impl LifecycleExpectationV1 {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

fn default_true() -> bool {
    true
}
