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

pub const FS_SCOPE_KEY: &str = "fs_scope";
pub const FS_SCOPE_ROOT_KEY: &str = "root";

/// Standalone `{ fs_scope: { root } }` metadata value — the shape
/// [`TurnOptions::filesystem_root`] reads back.
pub fn fs_scope_metadata(root: &str) -> Value {
    serde_json::json!({ FS_SCOPE_KEY: { FS_SCOPE_ROOT_KEY: root } })
}

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

/// New-format, names-only skill context. The filter remains live against the
/// harness catalog; the baseline is the immutable index admitted to the
/// session's system-prompt prefix.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillContext {
    /// `None` means all skills; a non-empty list is an exact-id curation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<String>,
}

/// The effective skill view this session most recently admitted. A
/// fingerprint of `None` is the explicit removed view; no `SkillAck` means
/// the source has not yet produced an authoritative observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillAck {
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

/// The agent profile a turn runs as, frozen at resolution time
/// (`options.agent` on `harness::send`, `agent` on `harness::spawn`). Only
/// what later turn machinery needs survives here: the id for display and
/// inheritance. Which agent profile a spawn may name is the prompt's decision — the
/// profile body steers it; nothing gates it. Directory edits after
/// resolution never reach a live session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentIdentity {
    pub id: String,
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
    /// Legacy-only attribution for skill bodies previously frozen from
    /// session metadata. New sessions never populate this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills_prompt: Option<String>,
    /// Names-and-descriptions-only skill context for new-format sessions.
    /// Absence identifies a durable legacy session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_context: Option<SkillContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    pub max_turns: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    /// Hard input-plus-output token budget shared by this root turn and every
    /// in-turn sub-agent it spawns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_tokens: Option<u64>,
    /// Hard USD budget shared by this root turn and every in-turn sub-agent it
    /// spawns. Requires catalog pricing for every model used by the tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
    /// Root session whose durable budget ledger this turn charges. Internal
    /// bookkeeping populated by `harness::send` and inherited by sub-agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_root_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    pub output: OutputContract,
    /// The dispatch policy; `None` denies every call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub functions: Option<FunctionPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    /// The agent profile this turn runs as, resolved once and frozen. Sticky
    /// like the system prompt: a send naming neither prompt field inherits it;
    /// naming an explicit prompt field sheds it (the escape hatch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentIdentity>,
    /// Cap on output-contract validation retries before finalising with a
    /// best-effort result (harness.md § Output contract).
    #[serde(default = "default_max_validation_retries")]
    pub max_validation_retries: u32,
    /// Mid-stream transient failures that may resume from the preserved
    /// transcript before this turn fails.
    #[serde(default = "default_max_transient_resumes")]
    pub max_transient_resumes: u32,
}

fn default_max_validation_retries() -> u32 {
    2
}

fn default_max_transient_resumes() -> u32 {
    // Keep in lockstep with `config::default_max_transient_resumes` —
    // overload bursts cluster; a budget of 1 dies on the second
    // mid-stream 529 in a turn.
    3
}

impl TurnOptions {
    /// The filesystem root this turn is scoped to:
    /// `metadata.fs_scope.root`.
    pub fn filesystem_root(&self) -> Option<&str> {
        self.metadata
            .as_ref()
            .and_then(|m| m.get(FS_SCOPE_KEY))
            .and_then(Value::as_object)
            .and_then(|scope| scope.get(FS_SCOPE_ROOT_KEY))
            .and_then(Value::as_str)
    }

    pub fn refresh_filesystem_root_from(&mut self, incoming: &TurnOptions) -> bool {
        let Some(root) = incoming.filesystem_root() else {
            return false;
        };
        if self.filesystem_root() == Some(root) {
            return false;
        }
        self.set_filesystem_root(root);
        true
    }

    /// Set `metadata.fs_scope.root`, preserving every other metadata key.
    pub fn set_filesystem_root(&mut self, root: &str) {
        let metadata = self
            .metadata
            .get_or_insert_with(|| Value::Object(Default::default()));
        if !metadata.is_object() {
            *metadata = Value::Object(Default::default());
        }
        if let Some(map) = metadata.as_object_mut() {
            let mut fs_scope = serde_json::Map::new();
            fs_scope.insert(
                FS_SCOPE_ROOT_KEY.to_string(),
                Value::String(root.to_string()),
            );
            map.insert(FS_SCOPE_KEY.to_string(), Value::Object(fs_scope));
        }
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
    /// Whether this spawn appended work to a session that already existed.
    /// Defaults to false so checkpoints written before this field existed are
    /// conservatively treated as session creations by the fan-out guard.
    #[serde(default)]
    pub child_session_reused: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub held_by: Option<String>,
    /// The call's arguments as mutated by the hook chain up to the hold, so a
    /// release executes the mutated call — not the model's original arguments
    /// re-read from the transcript. Absent on records written before this
    /// field existed; release falls back to transcript recovery then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_arguments: Option<Value>,
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

/// A model-visible function contract retained for reuse within one session.
/// Digests keep the durable turn record small; raw contracts remain only in
/// the session transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FunctionContractLedgerEntry {
    pub contract_digest: String,
    pub source_function_call_id: String,
    pub source_content_digest: String,
    /// True only after final context assembly confirmed the exact source is
    /// still model-visible. Newly appended and legacy rows start ineligible.
    #[serde(default)]
    pub eligible: bool,
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
    /// First ~30 chars of the user message that started the turn, stamped
    /// into OTel baggage as the `iii.tag.message` trace tag on every step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_preview: Option<String>,
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
    /// Display-only parent for parentless spawns (no live parent turn):
    /// lets worker-registered `turn-completed` / `turn-started`
    /// `parent_session_id` filters match those children too, and the console
    /// nest them. Never set alongside `parent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_parent_session_id: Option<String>,
    /// Function-registry generation this session last acknowledged; a mismatch
    /// at generate time appends a registry-change notice so session-cached
    /// contracts get re-fetched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub functions_generation: Option<u64>,
    /// Function contracts whose exact full source result was retained in the
    /// most recently assembled model context for this session.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub function_contract_ledger: BTreeMap<String, FunctionContractLedgerEntry>,
    /// Last effective names-only skill view admitted to the transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_ack: Option<SkillAck>,
    /// Whether this session has begun a model generation under the new skill
    /// context. Before then, first availability may still become the baseline.
    #[serde(default)]
    pub skills_started: bool,
    /// Latest generation's context accounting (also stored under
    /// `harness_context/<session_id>` once the generation completes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_snapshot: Option<crate::context_snapshot::ContextSnapshotV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_error: Option<String>,
    #[serde(default)]
    pub validation_retries: u32,
    #[serde(default)]
    pub transient_resumes: u32,
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

    /// Children targeted by this turn, regardless of checkpoint state or
    /// whether their session was created or reused (fire-and-forget spawns
    /// settle `Done` instantly). Feeds `harness::status` children and the stop
    /// cascade — stopping an already-finished child is a harmless no-op.
    pub fn spawned_children(&self) -> Vec<ParentLink> {
        self.calls
            .iter()
            .filter(|(_, c)| c.child_session_id.is_some() && c.child_turn_id.is_some())
            .map(|(call_id, c)| ParentLink {
                session_id: c.child_session_id.clone().unwrap_or_default(),
                turn_id: c.child_turn_id.clone().unwrap_or_default(),
                function_call_id: call_id.clone(),
            })
            .collect()
    }

    /// Number of child SESSIONS this turn actually created. Re-tasking an
    /// existing session still appears in [`Self::spawned_children`] for status
    /// and cancellation, but does not consume another fan-out slot.
    pub fn created_child_session_count(&self) -> usize {
        self.calls
            .values()
            .filter(|checkpoint| {
                checkpoint.child_session_id.is_some()
                    && checkpoint.child_turn_id.is_some()
                    && !checkpoint.child_session_reused
            })
            .count()
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
            message_preview: None,
            abort: false,
            watermark_entry_id: None,
            stream_request_id: None,
            options: TurnOptions {
                model: "m".into(),
                provider: None,
                system_prompt: None,
                skills_prompt: None,
                skill_context: None,
                mode: None,
                max_turns: 16,
                max_output_tokens: None,
                max_total_tokens: None,
                max_cost_usd: None,
                budget_root_session_id: None,
                thinking_level: None,
                provider_options: None,
                output: OutputContract::Text,
                functions: None,
                metadata: None,
                agent: None,
                max_validation_retries: 2,
                max_transient_resumes: 1,
            },
            calls: Default::default(),
            parent: None,
            display_parent_session_id: None,
            functions_generation: None,
            function_contract_ledger: Default::default(),
            skill_ack: None,
            skills_started: false,
            context_snapshot: None,
            result: None,
            result_error: None,
            validation_retries: 0,
            transient_resumes: 0,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn legacy_turn_records_default_to_an_empty_function_contract_ledger() {
        let mut value = serde_json::to_value(record()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("function_contract_ledger");

        let decoded: TurnRecord = serde_json::from_value(value).unwrap();
        assert!(decoded.function_contract_ledger.is_empty());
    }

    #[test]
    fn legacy_contract_ledger_entries_default_to_ineligible() {
        let decoded: FunctionContractLedgerEntry = serde_json::from_value(serde_json::json!({
            "contract_digest": "contract",
            "source_function_call_id": "call-1",
            "source_content_digest": "content"
        }))
        .unwrap();

        assert!(!decoded.eligible);
    }

    fn cp(state: CallState, child: Option<&str>, reused: bool) -> CallCheckpoint {
        CallCheckpoint {
            state,
            function_id: Some("harness::spawn".into()),
            entry_id: None,
            child_session_id: child.map(|s| s.to_string()),
            child_turn_id: child.map(|_| "t_child".to_string()),
            child_session_reused: reused,
            held_by: None,
            held_arguments: None,
            pending_timeout_ms: None,
            pending_at: None,
        }
    }

    #[test]
    fn filesystem_root_reads_the_metadata_key() {
        let mut r = record();
        r.options.metadata =
            Some(json!({ "fs_scope": { "root": "/work/p" }, "message_id": "m_1" }));
        assert_eq!(r.options.filesystem_root(), Some("/work/p"));
    }

    #[test]
    fn filesystem_root_is_none_without_metadata_or_key_or_string() {
        let mut r = record();
        assert_eq!(r.options.filesystem_root(), None);
        r.options.metadata = Some(json!({ "message_id": "m_1" }));
        assert_eq!(r.options.filesystem_root(), None);
        r.options.metadata = Some(json!({ "fs_scope": { "root": 7 } }));
        assert_eq!(r.options.filesystem_root(), None);
    }

    #[test]
    fn pending_call_ids_lists_triggered_and_pending() {
        let mut r = record();
        r.calls.insert("a".into(), cp(CallState::Done, None, false));
        r.calls
            .insert("b".into(), cp(CallState::Pending, None, false));
        r.calls
            .insert("c".into(), cp(CallState::Triggered, None, false));
        let mut ids = r.pending_call_ids();
        ids.sort();
        assert_eq!(ids, vec!["b", "c"]);
    }

    #[test]
    fn spawned_children_lists_every_checkpoint_with_child_ids() {
        let mut r = record();
        // Legacy parked spawn (pre-deploy record).
        r.calls
            .insert("a".into(), cp(CallState::Pending, Some("s_child"), false));
        r.calls
            .insert("b".into(), cp(CallState::Pending, None, false)); // hook hold, no child
                                                                      // Fire-and-forget spawn: Done instantly, still counts.
        r.calls
            .insert("c".into(), cp(CallState::Done, Some("s_done"), true));
        let children = r.spawned_children();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].session_id, "s_child");
        assert_eq!(children[0].function_call_id, "a");
        assert_eq!(children[1].session_id, "s_done");
        assert_eq!(children[1].function_call_id, "c");
        assert_eq!(r.created_child_session_count(), 1);
    }

    #[test]
    fn reused_sessions_do_not_consume_fanout_slots() {
        let mut r = record();
        r.calls
            .insert("fresh".into(), cp(CallState::Done, Some("s_child"), false));
        for n in 0..8 {
            r.calls.insert(
                format!("reuse-{n}"),
                cp(CallState::Done, Some("s_child"), true),
            );
        }
        assert_eq!(r.spawned_children().len(), 9);
        assert_eq!(r.created_child_session_count(), 1);
    }

    #[test]
    fn legacy_spawn_checkpoint_counts_as_a_session_creation() {
        let checkpoint: CallCheckpoint = serde_json::from_value(json!({
            "state": "done",
            "function_id": "harness::spawn",
            "child_session_id": "s_child",
            "child_turn_id": "t_child"
        }))
        .unwrap();
        assert!(!checkpoint.child_session_reused);

        let mut r = record();
        r.calls.insert("legacy".into(), checkpoint);
        assert_eq!(r.created_child_session_count(), 1);
    }

    #[test]
    fn turn_record_round_trips_through_json() {
        let mut r = record();
        r.calls
            .insert("a".into(), cp(CallState::Pending, Some("s_child"), false));
        // Populate the trigger-spawn fields so the round trip exercises them
        // with values, not just their skip-if-none defaults.
        r.display_parent_session_id = Some("s_display_parent".into());
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
        assert_eq!(r.options.max_transient_resumes, 3);
        assert_eq!(r.transient_resumes, 0);
        assert_eq!(r.options.output, OutputContract::Text);
        assert_eq!(r.options.skill_context, None);
        assert_eq!(r.skill_ack, None);
        assert!(!r.skills_started);
        assert_eq!(r.options.agent, None);
    }

    #[test]
    fn agent_identity_round_trips_and_stays_off_the_wire_when_absent() {
        let mut r = record();
        assert!(!serde_json::to_value(&r.options)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("agent"));
        r.options.agent = Some(AgentIdentity {
            id: "tech-leader".into(),
        });
        let back: TurnRecord = serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn new_skill_context_and_ack_round_trip() {
        let mut r = record();
        r.options.skill_context = Some(SkillContext {
            filter: Some(vec!["review".into()]),
            baseline: Some("<available_skills>review</available_skills>".into()),
        });
        r.skill_ack = Some(SkillAck {
            generation: 3,
            fingerprint: Some("sha256:abc".into()),
        });
        r.skills_started = true;

        let value = serde_json::to_value(&r).unwrap();
        assert_eq!(
            value["options"]["skill_context"]["filter"],
            json!(["review"])
        );
        assert_eq!(value["skill_ack"]["generation"], 3);
        let back: TurnRecord = serde_json::from_value(value).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn refresh_filesystem_root_updates_only_when_incoming_has_scope() {
        let mut existing = record().options;
        existing.metadata = Some(json!({ "trace": "keep", "fs_scope": { "root": "/old" } }));

        let mut incoming = existing.clone();
        incoming.metadata = Some(json!({ "fs_scope": { "root": "/new" }, "ignored": true }));

        assert!(existing.refresh_filesystem_root_from(&incoming));
        assert_eq!(existing.filesystem_root(), Some("/new"));
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
        assert!(!existing.refresh_filesystem_root_from(&no_scope));
        assert_eq!(existing.filesystem_root(), Some("/new"));
    }
}
