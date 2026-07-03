//! Harness-internal loop bookkeeping: the durable turn record persisted to
//! the `harness_turn` state scope, plus the per-send options and per-call
//! checkpoints it carries (harness.md § State / § Durability & idempotency).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::prompt::Mode;
use crate::types::model::ThinkingLevel;
use crate::types::output::OutputContract;

/// The options-metadata key that carries the session working directory. The
/// read side (`TurnOptions::working_dir`) and the sub-agent write side
/// (`subagent::inherit_workspace`) share this constant so a rename can't
/// silently desync the two and drop child scope. Mirrors
/// `workspace_inject::BASE_DIR_FIELD`.
pub const WORKING_DIR_KEY: &str = "working_dir";

/// The coarse, harness-internal turn lifecycle (harness.md § API Reference).
/// Finer-grained than the session's `status`, which the loop derives from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Running,
    AwaitingFunctions,
    Completed,
    Cancelled,
    Failed,
}

impl TurnStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TurnStatus::Completed | TurnStatus::Cancelled | TurnStatus::Failed
        )
    }
}

/// How allowed functions reach the model (harness.md § Exposure modes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExposeMode {
    #[default]
    AgentTrigger,
    Native,
}

/// The fail-closed dispatch policy (harness.md § Functions). Absent on the
/// send => every call denied (a plain chat loop).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FunctionPolicy {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub expose: ExposeMode,
}

/// Per-send options frozen onto the turn record when it is created; they
/// apply unchanged until the turn ends (a merged send never changes them).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TurnOptions {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    pub max_turns: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    #[serde(default)]
    pub output: OutputContract,
    /// The dispatch policy; `None` denies every call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub functions: Option<FunctionPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    /// Cap on output-contract validation retries before finalising with a
    /// best-effort result (harness.md § Output contract).
    #[serde(default = "default_max_validation_retries")]
    pub max_validation_retries: u32,
}

fn default_max_validation_retries() -> u32 {
    2
}

impl TurnOptions {
    /// The session working directory this turn is scoped to: the
    /// `working_dir` key of the frozen options metadata. Every path that
    /// invokes a scoped `shell::*` / `coder::*` call (turn loop, deferred
    /// release, `function::trigger`) must stamp this via
    /// `workspace_inject::inject` so the console-picked directory stays
    /// reachable.
    pub fn working_dir(&self) -> Option<&str> {
        self.metadata
            .as_ref()
            .and_then(|m| m.get(WORKING_DIR_KEY))
            .and_then(Value::as_str)
    }

    pub fn refresh_working_dir_from(&mut self, incoming: &TurnOptions) -> bool {
        let Some(working_dir) = incoming.working_dir() else {
            return false;
        };
        if self.working_dir() == Some(working_dir) {
            return false;
        }

        let metadata = self
            .metadata
            .get_or_insert_with(|| Value::Object(Default::default()));
        if !metadata.is_object() {
            *metadata = Value::Object(Default::default());
        }
        if let Some(map) = metadata.as_object_mut() {
            map.insert(
                WORKING_DIR_KEY.to_string(),
                Value::String(working_dir.to_string()),
            );
        }
        true
    }
}

/// Lifecycle of one function call within a turn (harness.md § Per-call
/// checkpoints).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CallState {
    #[serde(alias = "dispatched")]
    Triggered,
    Pending,
    Done,
}

/// One call's checkpoint on the turn record. `held_by` marks a `pre_trigger`
/// hook hold; `child_*` marks a `harness::spawn` pending trigger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CallCheckpoint {
    pub state: CallState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub held_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_at: Option<i64>,
}

/// Sub-agent linkage recorded on a child turn (harness.md § Sub-agents).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ParentLink {
    pub session_id: String,
    pub turn_id: String,
    pub function_call_id: String,
}

/// The durable loop record (`harness_turn/<session_id>`). Seeded by CAS from
/// `harness::send` / `spawn`, advanced one step per `harness::turn`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TurnRecord {
    pub turn_id: String,
    pub session_id: String,
    pub status: TurnStatus,
    /// Monotonic step counter; guards against stale/duplicate dequeues.
    pub step: u64,
    /// Generate steps completed in this turn (the `max_turns` guard).
    pub turn_count: u32,
    /// Sub-agent depth; 0 for top-level turns.
    pub depth: u32,
    #[serde(default)]
    pub abort: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watermark_entry_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_request_id: Option<String>,
    pub options: TurnOptions,
    #[serde(default)]
    pub calls: BTreeMap<String, CallCheckpoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<ParentLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_error: Option<String>,
    #[serde(default)]
    pub validation_retries: u32,
    pub created_at: i64,
    pub updated_at: i64,
}

impl TurnRecord {
    /// Function_call ids still awaiting a result (`triggered` or `pending`).
    pub fn pending_call_ids(&self) -> Vec<String> {
        self.calls
            .iter()
            .filter(|(_, c)| matches!(c.state, CallState::Pending | CallState::Triggered))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Non-terminal spawned children of this turn (for the stop cascade).
    pub fn live_children(&self) -> Vec<ParentLink> {
        self.calls
            .iter()
            .filter(|(_, c)| {
                c.state == CallState::Pending
                    && c.child_session_id.is_some()
                    && c.child_turn_id.is_some()
            })
            .map(|(call_id, c)| ParentLink {
                session_id: c.child_session_id.clone().unwrap_or_default(),
                turn_id: c.child_turn_id.clone().unwrap_or_default(),
                function_call_id: call_id.clone(),
            })
            .collect()
    }
}

/// `harness::send` webhook dedupe record (`harness_idem/<idempotency_key>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IdemRecord {
    pub session_id: String,
    pub turn_id: String,
    pub entry_id: String,
    pub ts: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record() -> TurnRecord {
        TurnRecord {
            turn_id: "t_1".into(),
            session_id: "s_1".into(),
            status: TurnStatus::AwaitingFunctions,
            step: 2,
            turn_count: 1,
            depth: 0,
            abort: false,
            watermark_entry_id: None,
            stream_request_id: None,
            options: TurnOptions {
                model: "m".into(),
                provider: None,
                system_prompt: None,
                mode: None,
                max_turns: 16,
                thinking_level: None,
                output: OutputContract::Text,
                functions: None,
                metadata: None,
                max_validation_retries: 2,
            },
            calls: Default::default(),
            parent: None,
            result: None,
            result_error: None,
            validation_retries: 0,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn cp(state: CallState, child: Option<&str>) -> CallCheckpoint {
        CallCheckpoint {
            state,
            function_id: Some("harness::spawn".into()),
            entry_id: None,
            child_session_id: child.map(|s| s.to_string()),
            child_turn_id: child.map(|_| "t_child".to_string()),
            held_by: None,
            pending_timeout_ms: None,
            pending_at: None,
        }
    }

    #[test]
    fn working_dir_reads_the_metadata_key() {
        let mut r = record();
        r.options.metadata = Some(json!({ "working_dir": "/work/p", "message_id": "m_1" }));
        assert_eq!(r.options.working_dir(), Some("/work/p"));
    }

    #[test]
    fn working_dir_is_none_without_metadata_or_key_or_string() {
        let mut r = record();
        assert_eq!(r.options.working_dir(), None);
        r.options.metadata = Some(json!({ "message_id": "m_1" }));
        assert_eq!(r.options.working_dir(), None);
        r.options.metadata = Some(json!({ "working_dir": 7 }));
        assert_eq!(r.options.working_dir(), None);
    }

    #[test]
    fn pending_call_ids_lists_triggered_and_pending() {
        let mut r = record();
        r.calls.insert("a".into(), cp(CallState::Done, None));
        r.calls.insert("b".into(), cp(CallState::Pending, None));
        r.calls.insert("c".into(), cp(CallState::Triggered, None));
        let mut ids = r.pending_call_ids();
        ids.sort();
        assert_eq!(ids, vec!["b", "c"]);
    }

    #[test]
    fn live_children_lists_only_pending_children() {
        let mut r = record();
        r.calls
            .insert("a".into(), cp(CallState::Pending, Some("s_child")));
        r.calls.insert("b".into(), cp(CallState::Pending, None)); // hook hold, no child
        r.calls
            .insert("c".into(), cp(CallState::Done, Some("s_done")));
        let children = r.live_children();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].session_id, "s_child");
        assert_eq!(children[0].function_call_id, "a");
    }

    #[test]
    fn turn_record_round_trips_through_json() {
        let mut r = record();
        r.calls
            .insert("a".into(), cp(CallState::Pending, Some("s_child")));
        let back: TurnRecord = serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn defaults_fill_in_for_a_minimal_record() {
        // A record persisted before optional fields existed still parses.
        let minimal = json!({
            "turn_id": "t_1", "session_id": "s_1", "status": "running",
            "step": 0, "turn_count": 0, "depth": 0,
            "options": { "model": "m", "max_turns": 16 },
            "created_at": 1, "updated_at": 1
        });
        let r: TurnRecord = serde_json::from_value(minimal).unwrap();
        assert!(!r.abort);
        assert!(r.calls.is_empty());
        assert_eq!(r.options.max_validation_retries, 2);
        assert_eq!(r.options.output, OutputContract::Text);
    }

    #[test]
    fn refresh_working_dir_updates_only_when_incoming_has_scope() {
        let mut existing = record().options;
        existing.metadata = Some(json!({ "trace": "keep", "working_dir": "/old" }));

        let mut incoming = existing.clone();
        incoming.metadata = Some(json!({ "working_dir": "/new", "ignored": true }));

        assert!(existing.refresh_working_dir_from(&incoming));
        assert_eq!(existing.working_dir(), Some("/new"));
        assert_eq!(
            existing
                .metadata
                .as_ref()
                .and_then(|m| m.get("trace"))
                .and_then(Value::as_str),
            Some("keep")
        );

        let mut no_scope = incoming;
        no_scope.metadata = Some(json!({ "other": "/ignored" }));
        assert!(!existing.refresh_working_dir_from(&no_scope));
        assert_eq!(existing.working_dir(), Some("/new"));
    }
}
