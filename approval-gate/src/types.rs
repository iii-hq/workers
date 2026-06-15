//! Wire types, transliterated from
//! tech-specs/2026-06-agentic/approval-gate.md § API Reference and
//! harness.md § Hooks › Contract. Every type that crosses the bus derives
//! serde + `JsonSchema` so the SDK publishes request/response schemas.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ApprovalError;

pub type JsonMap = serde_json::Map<String, Value>;

pub const DENIAL_SCHEMA_VERSION: u32 = 1;

/// Subset-equality metadata match: every key in `want` must equal the
/// stored value (the tenancy filter convention shared with
/// session-manager's trigger configs).
pub fn metadata_matches(want: &JsonMap, have: Option<&JsonMap>) -> bool {
    want.iter()
        .all(|(key, value)| have.and_then(|m| m.get(key)) == Some(value))
}

/// `session_id` / `function_call_id` boundary validation: non-empty and
/// no `/` — the reserved key separator in the `approval_pending` scope.
pub fn validate_id(label: &str, value: &str) -> Result<(), ApprovalError> {
    if value.is_empty() {
        return Err(ApprovalError::InvalidPayload(format!(
            "{label} must be a non-empty string"
        )));
    }
    if value.contains('/') {
        return Err(ApprovalError::InvalidPayload(format!(
            "{label} must not contain \"/\""
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Permission model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    #[default]
    Manual,
    Auto,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GrantedBy {
    UserClick,
    /// Copied from the deployment's `always_allow_seed` on first mutation.
    Seed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AlwaysAllowEntry {
    pub function_id: String,
    /// ms epoch
    pub granted_at: i64,
    pub granted_by: GrantedBy,
}

/// The stored per-session record (scope `approval_settings`). Reads
/// compute the effective settings from configuration defaults when no
/// record exists — see settings.rs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalSettings {
    #[serde(default)]
    pub mode: PermissionMode,
    /// Consulted only in auto mode.
    #[serde(default)]
    pub always_allow: Vec<AlwaysAllowEntry>,
    /// Consulted in every mode — remembered human decisions.
    #[serde(default)]
    pub approved_always: Vec<AlwaysAllowEntry>,
    /// ms epoch
    #[serde(default)]
    pub mode_set_at: i64,
}

// ---------------------------------------------------------------------------
// Denial envelope
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeniedBy {
    Permissions,
    User,
    GateUnavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MatchedConstraint {
    pub field: String,
    pub operator: String,
    pub value: Value,
}

/// The structured denial shape, rendered into `is_error` function_results.
/// `reason` strings are written for the model as much as the human — the
/// denial is information the agent can adapt to, not just a wall.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DenialEnvelope {
    pub schema_version: u32,
    pub status: String,
    pub denied_by: DeniedBy,
    pub function_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_constraint: Option<MatchedConstraint>,
    /// Redacted (see redact.rs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args_excerpt: Option<Value>,
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Pending inbox
// ---------------------------------------------------------------------------

/// The inbox payload — shared by both triggers, `list_pending`, and
/// `get_pending`. Self-describing (all ids inside the value) and
/// notification-safe (arguments pass through redaction).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PendingApprovalRecord {
    pub session_id: String,
    pub turn_id: String,
    pub function_call_id: String,
    pub function_id: String,
    /// Redacted — safe to forward to notification channels.
    #[serde(default)]
    pub arguments_excerpt: Value,
    /// ms epoch
    pub pending_at: i64,
    /// `pending_at + pending_timeout_ms`
    pub expires_at: i64,

    // Denormalized session context — soft-fetched via session::get at
    // hold time; omitted when the fetch fails or session-manager is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_description: Option<String>,
    /// Tenancy + routing (trigger config filter target).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_metadata: Option<JsonMap>,

    /// Sub-agent depth (0 = top-level), from `HookInput`.
    #[serde(default)]
    pub depth: i64,

    /// First text block of the assistant message that contained this
    /// function_call. Best-effort; not derivable from `pre_dispatch`
    /// `HookInput` in v1, so always omitted today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_excerpt: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ResolvedOutcome {
    Allow,
    Deny,
    Timeout,
    Aborted,
}

/// Payload of `approval::pending-resolved` — a pending call left the
/// inbox. Emitted exactly once per record (gated on the record delete).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PendingResolvedEvent {
    pub session_id: String,
    pub turn_id: String,
    pub function_call_id: String,
    pub function_id: String,
    pub outcome: ResolvedOutcome,
    /// Operator-supplied on deny.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Tenancy routing, from the record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_metadata: Option<JsonMap>,
    /// ms epoch
    pub resolved_at: i64,
}

// ---------------------------------------------------------------------------
// Harness hook contract (harness.md § Hooks › Contract) — only the fields
// the gate consumes. The harness owns these types; unknown fields are
// ignored on input.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct HookCall {
    pub id: String,
    pub function_id: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct HookInput {
    #[serde(default)]
    pub point: Option<String>,
    pub session_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub step: Option<i64>,
    /// Sub-agent depth (hooks run for child turns too).
    #[serde(default)]
    pub depth: i64,
    /// The per-send tracing metadata.
    #[serde(default)]
    pub metadata: Option<JsonMap>,
    /// pre_dispatch payload.
    #[serde(default)]
    pub call: Option<HookCall>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "decision", rename_all = "lowercase")]
pub enum HookOutput {
    Continue,
    Deny { reason: String },
    Hold { pending_timeout_ms: i64 },
}

// ---------------------------------------------------------------------------
// ContentBlock (README.md § Cross-cutting contracts) — only the text
// variant is needed (denial rendering into function_result content).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub text: String,
}

pub fn text_block(text: impl Into<String>) -> TextBlock {
    TextBlock {
        block_type: "text".to_string(),
        text: text.into(),
    }
}

// ---------------------------------------------------------------------------
// RPC request/response shapes (approval-gate.md § API Reference)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ResolveDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ResolveRequest {
    pub session_id: String,
    pub function_call_id: String,
    pub decision: ResolveDecision,
    /// Surfaced to the model on deny.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ResolveResponse {
    /// false: unknown/already-resolved pending call.
    pub resolved: bool,
    /// Passthrough from harness::function::resolve.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_resumed: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ListPendingRequest {
    #[serde(default)]
    pub session_id: Option<String>,
    /// Equality match against `session_metadata` (tenancy).
    #[serde(default)]
    pub metadata: Option<JsonMap>,
    /// Default 50.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Opaque.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListPendingResponse {
    /// Ordered by pending_at ascending.
    pub pending: Vec<PendingApprovalRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetPendingRequest {
    /// The session the pending record belongs to.
    pub session_id: String,
    /// The function call id of the held call.
    pub function_call_id: String,
}

/// `null` (handler returns `Option<GetPendingResponse>`) when resolved or
/// unknown.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GetPendingResponse {
    pub pending: PendingApprovalRecord,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SetModeRequest {
    pub session_id: String,
    pub mode: PermissionMode,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AlwaysAllowMutationRequest {
    pub session_id: String,
    pub function_id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ApproveAlwaysRequest {
    pub session_id: String,
    pub function_id: String,
}

/// Shared by every settings mutation RPC: the post-mutation record.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SettingsResponse {
    pub settings: ApprovalSettings,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetSettingsRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SettingsSource {
    Stored,
    Defaults,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GetSettingsResponse {
    pub settings: ApprovalSettings,
    /// Whether a per-session record exists.
    pub source: SettingsSource,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ClearSettingsRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ClearSettingsResponse {
    pub cleared: bool,
}

/// ms epoch now.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hook_output_serializes_to_spec_shapes() {
        assert_eq!(
            serde_json::to_value(HookOutput::Continue).unwrap(),
            json!({ "decision": "continue" })
        );
        assert_eq!(
            serde_json::to_value(HookOutput::Deny {
                reason: "nope".into()
            })
            .unwrap(),
            json!({ "decision": "deny", "reason": "nope" })
        );
        assert_eq!(
            serde_json::to_value(HookOutput::Hold {
                pending_timeout_ms: 1_800_000
            })
            .unwrap(),
            json!({ "decision": "hold", "pending_timeout_ms": 1_800_000 })
        );
    }

    #[test]
    fn hook_input_tolerates_unknown_and_missing_optional_fields() {
        let input: HookInput = serde_json::from_value(json!({
            "point": "pre_dispatch",
            "session_id": "s_1",
            "turn_id": "t_1",
            "step": 3,
            "depth": 1,
            "unknown_future_field": { "x": 1 },
            "call": { "id": "c_1", "function_id": "shell::run", "arguments": { "cmd": "ls" } }
        }))
        .unwrap();
        assert_eq!(input.call.unwrap().function_id, "shell::run");

        let bare: HookInput = serde_json::from_value(json!({
            "session_id": "s_1",
            "turn_id": "t_1"
        }))
        .unwrap();
        assert!(bare.call.is_none());
        assert_eq!(bare.depth, 0);
    }

    #[test]
    fn permission_mode_wire_values() {
        assert_eq!(
            serde_json::to_value(PermissionMode::Manual).unwrap(),
            json!("manual")
        );
        assert_eq!(
            serde_json::to_value(PermissionMode::Auto).unwrap(),
            json!("auto")
        );
        assert_eq!(
            serde_json::to_value(PermissionMode::Full).unwrap(),
            json!("full")
        );
    }

    #[test]
    fn settings_parse_tolerates_missing_fields() {
        let s: ApprovalSettings = serde_json::from_value(json!({ "mode": "auto" })).unwrap();
        assert_eq!(s.mode, PermissionMode::Auto);
        assert!(s.always_allow.is_empty());
        assert!(s.approved_always.is_empty());
        assert_eq!(s.mode_set_at, 0);
    }

    #[test]
    fn granted_by_wire_values() {
        assert_eq!(
            serde_json::to_value(GrantedBy::UserClick).unwrap(),
            json!("user_click")
        );
        assert_eq!(
            serde_json::to_value(GrantedBy::Seed).unwrap(),
            json!("seed")
        );
    }

    #[test]
    fn pending_record_roundtrip_omits_absent_optionals() {
        let record = PendingApprovalRecord {
            session_id: "s_1".into(),
            turn_id: "t_1".into(),
            function_call_id: "c_1".into(),
            function_id: "shell::run".into(),
            arguments_excerpt: json!({ "cmd": "ls" }),
            pending_at: 100,
            expires_at: 200,
            session_title: None,
            session_description: None,
            session_metadata: None,
            depth: 0,
            assistant_excerpt: None,
        };
        let v = serde_json::to_value(&record).unwrap();
        assert!(v.get("session_title").is_none());
        assert!(v.get("assistant_excerpt").is_none());
        let back: PendingApprovalRecord = serde_json::from_value(v).unwrap();
        assert_eq!(back, record);
    }

    #[test]
    fn validate_id_rejects_slash_and_empty() {
        assert!(validate_id("session_id", "s_1").is_ok());
        assert!(validate_id("session_id", "").is_err());
        assert!(validate_id("session_id", "a/b").is_err());
        let err = validate_id("function_call_id", "x/y").unwrap_err();
        assert_eq!(err.code(), "approval/invalid_payload");
    }

    #[test]
    fn metadata_matches_is_subset_equality() {
        let want: JsonMap = serde_json::from_value(json!({ "owner": "u_1" })).unwrap();
        let have: JsonMap =
            serde_json::from_value(json!({ "owner": "u_1", "extra": true })).unwrap();
        assert!(metadata_matches(&want, Some(&have)));
        let other: JsonMap = serde_json::from_value(json!({ "owner": "u_2" })).unwrap();
        assert!(!metadata_matches(&want, Some(&other)));
        assert!(!metadata_matches(&want, None));
        let empty: JsonMap = JsonMap::new();
        assert!(metadata_matches(&empty, None));
    }

    #[test]
    fn denial_envelope_omits_absent_optionals() {
        let envelope = DenialEnvelope {
            schema_version: DENIAL_SCHEMA_VERSION,
            status: "denied".into(),
            denied_by: DeniedBy::GateUnavailable,
            function_id: "shell::run".into(),
            rule_id: None,
            rule_action: None,
            matched_constraint: None,
            args_excerpt: None,
            reason: "policy unreachable".into(),
        };
        let v = serde_json::to_value(&envelope).unwrap();
        assert_eq!(v["denied_by"], json!("gate_unavailable"));
        assert!(v.get("rule_id").is_none());
        assert!(v.get("args_excerpt").is_none());
    }
}
