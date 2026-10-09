//! Durable, fail-closed subtree deletion. The selected session is always the root.
//! Queue redelivery resumes the persisted plan; terminal turn notifications, not
//! polling, drive the wait. Tombstones deliberately survive successful deletion.

use std::collections::BTreeSet;
use std::time::Duration;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::TriggerAction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::state;
use crate::types::message::AgentMessage;
use crate::types::turn::CallState;

pub const DELETE_ID: &str = "harness::delete-session-tree";
pub const STATUS_ID: &str = "harness::delete-session-tree-status";
pub const RUN_ID: &str = "harness::delete-session-tree-run";
pub const QUEUE: &str = "harness-session-deletion";
pub const OPERATIONS: &str = "harness_deletion";
pub const GUARDS: &str = "harness_deletion_guard";
pub const DISPATCHES: &str = "harness_deletion_dispatch";
const DEADLINE_MS: i64 = 120_000;
// Healthy local cancellation gets a substantial grace period, bounded by the
// durable operation deadline. Force only waits briefly for a finishing step.
const LOCAL_CANCEL_WAIT_MS: u64 = 30_000;
const FORCE_WRITER_WAIT_MS: u64 = 500;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
// Like other function inputs, tolerate metadata injected by the engine
// (notably _caller_worker_id). Domain identifiers remain required.
pub struct DeleteRequest {
    pub session_id: String,
    #[serde(default)]
    pub mode: DeletionMode,
    /// Required together for force; identifies the normal result confirmed by the operator.
    pub operation_id: Option<String>,
    pub attempt: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeletionMode {
    #[default]
    Normal,
    Force,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlockerKind {
    ActiveProcessing,
    UnconfirmedCancellation,
    UnknownCompletion,
}

/// Safe metadata only. Never include arguments, remote error messages or URLs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeletionBlocker {
    pub kind: BlockerKind,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeletionFailureCode {
    Blocked,
    OverlappingDeletion,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExistingDeletion {
    pub operation_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
// Shared by the status RPC and the queue runner, both engine-dispatched.
pub struct StatusRequest {
    pub operation_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeletionStatus {
    Deleting,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub operation_id: String,
    /// Initial attempt is 1; only an explicit failed-operation retry increments it.
    #[schemars(range(min = 1))]
    #[serde(deserialize_with = "deserialize_attempt")]
    pub attempt: u32,
    pub session_id: String,
    pub status: DeletionStatus,
    pub deleted_session_ids: Vec<String>,
    /// Planned members not confirmed complete; includes unconfirmed outcomes.
    #[serde(default)]
    pub remaining_session_ids: Vec<String>,
    /// Subset of remaining: erase was attempted, but completion is not confirmed.
    #[serde(default)]
    pub unconfirmed_session_ids: Vec<String>,
    #[serde(default)]
    pub mode: DeletionMode,
    /// True only when this attempt blocked before notification/cleanup/erase.
    /// Missing legacy metadata never proves retention.
    #[serde(default)]
    pub data_retained: bool,
    #[serde(default)]
    pub blockers: Vec<DeletionBlocker>,
    #[serde(default)]
    pub force_eligible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<DeletionFailureCode>,
    /// Navigation identity only; never authorization to retry or force.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_deletion: Option<ExistingDeletion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Operation {
    #[serde(deserialize_with = "stored_snapshot")]
    snapshot: Snapshot,
    deadline: i64,
    /// Persisted before any destructive action, including parent identity.
    members: Vec<String>,
    parent: Option<String>,
    name: String,
    planned: bool,
    notified: bool,
    /// Durable intent before notification or any cleanup; ambiguity cannot prove retention.
    #[serde(default)]
    cleanup_started: bool,
    /// Allows duplicate delivery of the exact confirmation without starting a new attempt.
    #[serde(default)]
    force_confirmed_attempt: Option<u32>,
    #[serde(default)]
    links: Vec<(String, Option<String>)>,
    #[serde(skip)]
    legacy_links_missing: bool,
    /// Only HEAD rows missing all three fields may recover its implicit erase cursor.
    #[serde(skip)]
    legacy_head_checkpoint: bool,
    /// The next child-first erase RPC may have succeeded without acknowledgement.
    #[serde(default)]
    erasing: Option<String>,
}

// Storage-only migration: pre-review operations are attempt 1. The public
// Snapshot schema/deserializer still REQUIRES attempt on all wire responses.
fn stored_snapshot<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Snapshot, D::Error> {
    let mut value = Value::deserialize(deserializer)?;
    if let Some(object) = value.as_object_mut() {
        object.entry("attempt").or_insert(json!(1));
    }
    serde_json::from_value(value).map_err(serde::de::Error::custom)
}

fn deserialize_attempt<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let attempt = u32::deserialize(deserializer)?;
    if attempt == 0 {
        return Err(serde::de::Error::custom(
            "deletion attempt must be at least 1",
        ));
    }
    Ok(attempt)
}

fn failure(message: impl Into<String>) -> HarnessError {
    HarnessError::InvalidRequest(message.into())
}

fn public_error(error: &HarnessError) -> String {
    match error {
        HarnessError::InvalidRequest(_) => error.to_string(),
        HarnessError::State(_) => {
            "Deletion state could not be verified; completion is not confirmed.".into()
        }
        _ => "Deletion dependency failed; completion is not confirmed.".into(),
    }
}

fn operation_id(session_id: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("delete_{:x}", Sha256::digest(session_id.as_bytes()))
}

async fn load(deps: &Deps, id: &str) -> Result<Option<Operation>, HarnessError> {
    let value = state::state_get(
        &deps.iii,
        OPERATIONS,
        id,
        deps.cfg().await.session_timeout_ms,
    )
    .await?;
    if value.is_null() {
        return Ok(None);
    }
    let legacy_outcomes = value
        .get("snapshot")
        .and_then(Value::as_object)
        .is_some_and(|snapshot| !snapshot.contains_key("unconfirmed_session_ids"));
    let legacy_cleanup_unknown = value.get("cleanup_started").is_none();
    let legacy_links_missing = !value
        .as_object()
        .is_some_and(|row| row.contains_key("links"));
    let legacy_head_checkpoint =
        legacy_links_missing && value.get("erasing").is_none() && legacy_cleanup_unknown;
    let mut op: Operation =
        serde_json::from_value(value).map_err(|e| HarnessError::State(e.to_string()))?;
    op.legacy_links_missing = legacy_links_missing;
    op.legacy_head_checkpoint = legacy_head_checkpoint;
    if legacy_cleanup_unknown {
        // Old checkpoints cannot exclude an unacknowledged notification/cleanup.
        op.cleanup_started = true;
        op.snapshot.data_retained = false;
    }
    // Legacy checkpoints also carry intent. Absence of the additive field
    // must never turn an interrupted delete into a retention assertion.
    if let Some(id) = &op.erasing {
        if legacy_outcomes
            && !op.snapshot.deleted_session_ids.contains(id)
            && !op.snapshot.unconfirmed_session_ids.contains(id)
        {
            if !op.snapshot.remaining_session_ids.contains(id) {
                op.snapshot.remaining_session_ids.push(id.clone());
            }
            op.snapshot.unconfirmed_session_ids.push(id.clone());
        }
    }
    Ok(Some(op))
}

async fn save(deps: &Deps, operation: &Operation) -> Result<(), HarnessError> {
    let mut value =
        serde_json::to_value(operation).map_err(|e| HarnessError::State(e.to_string()))?;
    // An acceptance/recovery write must not turn missing legacy links into
    // an explicitly empty (corrupt) plan before the locked migration runs.
    if operation.legacy_links_missing {
        if let Some(row) = value.as_object_mut() {
            row.remove("links");
        }
    }
    if operation.legacy_head_checkpoint {
        if let Some(row) = value.as_object_mut() {
            row.remove("erasing");
            row.remove("cleanup_started");
        }
    }
    state::state_set(
        &deps.iii,
        OPERATIONS,
        &operation.snapshot.operation_id,
        value,
        deps.cfg().await.session_timeout_ms,
    )
    .await
}

pub async fn status(deps: &Deps, req: StatusRequest) -> Result<Option<Snapshot>, HarnessError> {
    let Some(mut op) = load(deps, &req.operation_id).await? else {
        return Ok(None);
    };
    if op.snapshot.force_eligible {
        // Reads must not queue behind a runner. A busy runner cannot offer
        // actionable eligibility; acceptance still revalidates under locks.
        let Some(_command) = deps.deletion_commands.try_guard(&req.operation_id) else {
            op.snapshot.force_eligible = false;
            return Ok(Some(op.snapshot));
        };
        let Ok(_topology) = deps.topology.try_lock() else {
            op.snapshot.force_eligible = false;
            return Ok(Some(op.snapshot));
        };
        let refresh = async {
            validate_plan(deps, &op).await?;
            validate_owners(deps, &op).await?;
            drop(_topology);
            // A status refresh is a new read-only diagnosis, not a RUN of the
            // expired attempt. Its complete scan still has one fixed deadline.
            current_blockers(deps, &op, AgentMessage::now_ms() + DEADLINE_MS).await
        }
        .await;
        match refresh {
            Ok(blockers) => {
                op.snapshot.force_eligible = !blockers.is_empty();
                op.snapshot.blockers = blockers;
            }
            Err(error) => {
                op.snapshot.force_eligible = false;
                op.snapshot.failure_code = Some(DeletionFailureCode::Failed);
                op.snapshot.blockers.clear();
                op.snapshot.error = Some(public_error(&error));
            }
        }
    }
    Ok(Some(op.snapshot))
}

/// The deletion owner tombstoning `session_id` or one of its durable
/// ancestors, for regular session work (steps, dispatch, send, spawn, wake).
/// A "live" answer is memoized per session until this process writes or
/// releases any tombstone (see [`crate::liveness`]); an owner is always read
/// from state.
pub(crate) async fn guard_owner(
    deps: &Deps,
    session_id: &str,
) -> Result<Option<String>, HarnessError> {
    if deps.liveness.is_live(session_id) {
        return Ok(None);
    }
    let stamp = deps.liveness.stamp();
    let owner = guard_owner_uncached(deps, session_id).await?;
    if owner.is_none() {
        deps.liveness.remember_live(session_id, stamp);
    }
    Ok(owner)
}

/// Check the durable ancestry, so a not-yet-enumerated descendant is already
/// barred by its root's tombstone. Metadata errors fail closed. Deletion's own
/// conflict and parent checks call this authoritative walk directly.
async fn guard_owner_uncached(
    deps: &Deps,
    session_id: &str,
) -> Result<Option<String>, HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    let session = deps.session().await;
    let mut current = Some(session_id.to_string());
    let mut seen = BTreeSet::new();
    while let Some(id) = current {
        if !seen.insert(id.clone()) {
            return Err(failure(
                "cyclic session ancestry; refusing lifecycle mutation",
            ));
        }
        let guard = state::state_get(&deps.iii, GUARDS, &id, timeout).await?;
        if !guard.is_null() {
            return guard
                .as_str()
                .map(|id| Some(id.to_string()))
                .ok_or_else(|| failure("malformed deletion tombstone"));
        }
        current = session.metadata_of(&id).await?.and_then(|m| {
            m.get("parent_session_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    }
    Ok(None)
}

pub(crate) async fn ensure_live(deps: &Deps, session_id: &str) -> Result<(), HarnessError> {
    if let Some(owner) = guard_owner(deps, session_id).await? {
        return Err(failure(format!(
            "session {session_id} is tombstoned by {owner}"
        )));
    }
    Ok(())
}

async fn enqueue(deps: &Deps, id: &str) -> Result<(), HarnessError> {
    deps.iii
        .trigger(TriggerRequest {
            function_id: RUN_ID.into(),
            payload: json!({"operation_id": id}),
            action: Some(TriggerAction::Enqueue {
                queue: QUEUE.into(),
            }),
            timeout_ms: Some(deps.cfg().await.session_timeout_ms),
        })
        .await
        .map(|_| ())
        .map_err(|e| HarnessError::Dependency(format!("enqueue deletion: {e}")))
}

pub async fn handle(deps: &Deps, req: DeleteRequest) -> Result<Snapshot, HarnessError> {
    if req.session_id.trim().is_empty() {
        return Err(failure("session_id must not be empty"));
    }
    let id = operation_id(&req.session_id);
    if req.mode == DeletionMode::Force {
        return accept_force(deps, req, &id).await;
    }
    // A pending/completed read must not wait behind the cancellation lifetime.
    // It never writes state. A retry/first acceptance re-reads under the SAME
    // lock as run, so an old runner cannot overwrite a newer attempt.
    if let Some(op) = load(deps, &id).await? {
        if op.snapshot.status != DeletionStatus::Failed {
            if op.snapshot.status == DeletionStatus::Deleting {
                enqueue(deps, &id).await?;
            }
            return Ok(op.snapshot);
        }
    }
    let _lock = deps.deletion_commands.guard(&id).await;
    if let Some(op) = load(deps, &id).await? {
        // A normal command can check an escalated operation, never silently retry force.
        if op.snapshot.mode == DeletionMode::Force {
            return Ok(op.snapshot);
        }
    }
    let mut op = match load(deps, &id).await? {
        Some(op) if op.snapshot.status != DeletionStatus::Failed => {
            // An acceptance lost between state and enqueue is repaired by a
            // repeated command (and by startup recovery), with the same id.
            if op.snapshot.status == DeletionStatus::Deleting {
                enqueue(deps, &id).await?;
            }
            return Ok(op.snapshot);
        }
        Some(mut op) => {
            op.snapshot.attempt = op
                .snapshot
                .attempt
                .checked_add(1)
                .ok_or_else(|| failure("deletion attempt counter exhausted"))?;
            op.snapshot.status = DeletionStatus::Deleting;
            op.snapshot.error = None;
            op.snapshot.force_eligible = false;
            op.snapshot.failure_code = None;
            op.snapshot.existing_deletion = None;
            op.snapshot.blockers.clear();
            op.snapshot.data_retained = false;
            op.deadline = AgentMessage::now_ms() + DEADLINE_MS;
            op
        }
        None => Operation {
            snapshot: Snapshot {
                operation_id: id.clone(),
                attempt: 1,
                session_id: req.session_id.clone(),
                status: DeletionStatus::Deleting,
                deleted_session_ids: Vec::new(),
                remaining_session_ids: Vec::new(),
                unconfirmed_session_ids: Vec::new(),
                mode: DeletionMode::Normal,
                data_retained: false,
                blockers: Vec::new(),
                force_eligible: false,
                failure_code: None,
                existing_deletion: None,
                error: None,
            },
            deadline: AgentMessage::now_ms() + DEADLINE_MS,
            members: Vec::new(),
            parent: None,
            name: req.session_id.clone(),
            planned: false,
            notified: false,
            cleanup_started: false,
            force_confirmed_attempt: None,
            links: Vec::new(),
            legacy_links_missing: false,
            legacy_head_checkpoint: false,
            erasing: None,
        },
    };
    // Save before admission is closed: recovery can finish a partial guard write.
    save(deps, &op).await?;
    let admission = deps.topology.lock().await;
    let reserve = reserve_root(deps, &mut op).await;
    drop(admission);
    let accepted = match reserve {
        Ok(()) => enqueue(deps, &id).await,
        Err(e) => Err(e),
    };
    if let Err(error) = accepted {
        op.snapshot.status = DeletionStatus::Failed;
        op.snapshot.error = Some(public_error(&error));
        if op.snapshot.failure_code.is_none() {
            op.snapshot.failure_code = Some(DeletionFailureCode::Failed);
        }
        save(deps, &op).await?;
        deps.deletion_events.emit(&op.snapshot).await?;
    }
    Ok(op.snapshot)
}

async fn accept_force(deps: &Deps, req: DeleteRequest, id: &str) -> Result<Snapshot, HarnessError> {
    let _command = deps.deletion_commands.guard(id).await;
    let mut op = load(deps, id)
        .await?
        .ok_or_else(|| failure("force requires a prior normal deletion"))?;
    if req.operation_id.as_deref() != Some(id) || op.snapshot.session_id != req.session_id {
        return Err(failure("force confirmation does not match selected root"));
    }
    if op.snapshot.mode == DeletionMode::Force && req.attempt == op.force_confirmed_attempt {
        // Replay, not a retry. A retry must confirm the current failed attempt.
        if op.snapshot.status == DeletionStatus::Deleting {
            enqueue(deps, id).await?;
        }
        return Ok(op.snapshot);
    }
    if req.attempt != Some(op.snapshot.attempt)
        || op.snapshot.status != DeletionStatus::Failed
        || !(op.snapshot.force_eligible || op.snapshot.mode == DeletionMode::Force)
    {
        return Err(failure(
            "force confirmation is stale or deletion is not eligible",
        ));
    }
    let _topology = deps.topology.lock().await;
    validate_or_migrate_plan(deps, &mut op).await?;
    validate_owners(deps, &op).await?;
    reserve_root(deps, &mut op).await?;
    // Refresh every authoritative record/witness; a malformed store or a new
    // overlap fails before accepting force, not through an error-string guess.
    // Confirmation may follow an expired normal attempt. Diagnose under a
    // separate fixed read deadline; do not change durable RUN state until accepted.
    let blockers = current_blockers(deps, &op, AgentMessage::now_ms() + DEADLINE_MS).await?;
    if op.snapshot.mode == DeletionMode::Normal && blockers.is_empty() {
        return Err(failure("deletion blockers resolved; retry normal deletion"));
    }
    op.force_confirmed_attempt = req.attempt;
    op.snapshot.attempt = op
        .snapshot
        .attempt
        .checked_add(1)
        .ok_or_else(|| failure("deletion attempt counter exhausted"))?;
    op.snapshot.mode = DeletionMode::Force;
    op.snapshot.status = DeletionStatus::Deleting;
    op.snapshot.error = None;
    op.snapshot.failure_code = None;
    op.snapshot.existing_deletion = None;
    op.snapshot.force_eligible = false;
    op.snapshot.data_retained = false;
    op.snapshot.blockers = blockers;
    op.deadline = AgentMessage::now_ms() + DEADLINE_MS;
    save(deps, &op).await?;
    drop(_topology);
    enqueue(deps, id).await?;
    Ok(op.snapshot)
}

/// Only a MISSING legacy link field is migratable. Derive current links under
/// topology/command locks, then validate exact order, scope, root parent,
/// intent and existing guard ownership before persisting the migration.
async fn validate_or_migrate_plan(deps: &Deps, op: &mut Operation) -> Result<(), HarnessError> {
    if !op.legacy_links_missing {
        return validate_plan(deps, op).await;
    }
    let mut migrated = op.clone();
    if op.legacy_head_checkpoint {
        // HEAD saved no erase intent. Only its exact child-first cursor, with
        // our pre-existing guard, can explain a lost delete acknowledgement.
        if let Some(next) = op
            .members
            .iter()
            .rev()
            .find(|id| !op.snapshot.deleted_session_ids.contains(id))
        {
            if !deps.session().await.exists(next).await? {
                if state::state_get(&deps.iii, GUARDS, next, deps.cfg().await.session_timeout_ms)
                    .await?
                    .as_str()
                    != Some(&op.snapshot.operation_id)
                {
                    return Err(failure("legacy erase cursor lost guard ownership"));
                }
                migrated.erasing = Some(next.clone());
                migrated.snapshot.unconfirmed_session_ids = vec![next.clone()];
                migrated.snapshot.remaining_session_ids = op
                    .members
                    .iter()
                    .filter(|id| !op.snapshot.deleted_session_ids.contains(id))
                    .cloned()
                    .collect();
            }
        }
        // Preflight exact scope/order/parent BEFORE reserving any missing guard.
        // The topology lock serializes reservations; no foreign owner is replaced.
        migrated.links = super::session_tree::collect(deps, &op.snapshot.session_id)
            .await?
            .sessions
            .into_iter()
            .map(|n| (n.session_id, n.parent_session_id))
            .collect();
        validate_plan(deps, &migrated).await?;
        let timeout = deps.cfg().await.session_timeout_ms;
        for id in &op.members {
            let guard = state::state_get(&deps.iii, GUARDS, id, timeout).await?;
            if !guard.is_null() && guard.as_str() != Some(&op.snapshot.operation_id) {
                return Err(failure("deletion guard ownership changed"));
            }
        }
        if let Some(parent) = &op.parent {
            if guard_owner_uncached(deps, parent).await?.is_some() {
                return Err(failure("overlapping ancestor deletion"));
            }
        }
        for id in &op.members {
            claim_guard(deps, id, &op.snapshot.operation_id).await?;
        }
        // An interrupted claim leaves the original legacy provenance intact.
        // Once all reservations exist, persist intent/unknown BEFORE final validation.
        migrated.legacy_head_checkpoint = false;
        *op = migrated.clone();
        save(deps, &migrated).await?;
    }
    validate_owners(deps, &migrated).await?;
    migrated.links = super::session_tree::collect(deps, &op.snapshot.session_id)
        .await?
        .sessions
        .into_iter()
        .map(|n| (n.session_id, n.parent_session_id))
        .collect();
    migrated.legacy_links_missing = false;
    validate_plan(deps, &migrated).await?;
    save(deps, &migrated).await?;
    *op = migrated;
    Ok(())
}

/// A force confirmation cannot silently expand a previously confirmed scope.
/// Also checked by every runner/redelivery and partial-delete retry.
async fn validate_plan(deps: &Deps, op: &Operation) -> Result<(), HarnessError> {
    if op.snapshot.operation_id != operation_id(&op.snapshot.session_id) {
        return Err(failure("corrupt deletion identity"));
    }
    if !op.planned {
        return Err(failure("deletion has no validated plan"));
    }
    if op.members.iter().collect::<BTreeSet<_>>().len() != op.members.len()
        || !op
            .snapshot
            .deleted_session_ids
            .iter()
            .all(|id| op.members.contains(id))
        || (!op.members.is_empty() && !op.members.contains(&op.snapshot.session_id))
    {
        return Err(failure("corrupt deletion scope"));
    }
    let next = op
        .members
        .iter()
        .rev()
        .find(|id| !op.snapshot.deleted_session_ids.contains(id));
    if op.erasing.as_ref().is_some_and(|id| Some(id) != next) {
        return Err(failure("corrupt deletion erase intent"));
    }
    let absent_intent = if let Some(id) = &op.erasing {
        if !deps.session().await.exists(id).await? {
            let owner =
                state::state_get(&deps.iii, GUARDS, id, deps.cfg().await.session_timeout_ms)
                    .await?;
            if owner.as_str() != Some(&op.snapshot.operation_id) {
                return Err(failure("deletion erase intent lost guard ownership"));
            }
            Some(id)
        } else {
            None
        }
    } else {
        None
    };
    let tree = super::session_tree::collect(deps, &op.snapshot.session_id).await?;
    let links: BTreeSet<_> = tree
        .sessions
        .iter()
        .map(|n| (n.session_id.clone(), n.parent_session_id.clone()))
        .collect();
    let expected_links: BTreeSet<_> = op
        .links
        .iter()
        .filter(|(id, _)| {
            !op.snapshot.deleted_session_ids.contains(id) && absent_intent != Some(id)
        })
        .cloned()
        .collect();
    if links != expected_links {
        return Err(failure("deletion topology links changed; refusing erase"));
    }
    let ordered: Vec<_> = tree.sessions.iter().map(|n| n.session_id.clone()).collect();
    let expected_order: Vec<_> = op
        .members
        .iter()
        .filter(|id| !op.snapshot.deleted_session_ids.contains(id) && absent_intent != Some(*id))
        .cloned()
        .collect();
    if ordered != expected_order {
        return Err(failure("deletion plan order changed; refusing erase"));
    }
    let actual: BTreeSet<_> = tree.sessions.into_iter().map(|n| n.session_id).collect();
    let expected: BTreeSet<_> = op
        .members
        .iter()
        .filter(|id| !op.snapshot.deleted_session_ids.contains(id) && absent_intent != Some(*id))
        .cloned()
        .collect();
    if (!tree.complete && !expected.is_empty()) || actual != expected {
        return Err(failure(
            "deletion topology changed or is incomplete; refusing erase",
        ));
    }
    let parent = deps
        .session()
        .await
        .metadata_of(&op.snapshot.session_id)
        .await?
        .and_then(|m| {
            m.get("parent_session_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    if !op
        .snapshot
        .deleted_session_ids
        .contains(&op.snapshot.session_id)
        && absent_intent != Some(&op.snapshot.session_id)
        && parent != op.parent
    {
        return Err(failure("deletion root parent changed; refusing erase"));
    }
    Ok(())
}

async fn validate_owners(deps: &Deps, op: &Operation) -> Result<(), HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    for id in &op.members {
        if state::state_get(&deps.iii, GUARDS, id, timeout)
            .await?
            .as_str()
            != Some(&op.snapshot.operation_id)
        {
            return Err(failure("deletion guard ownership changed"));
        }
    }
    if let Some(parent) = &op.parent {
        if guard_owner_uncached(deps, parent).await?.is_some() {
            return Err(failure("overlapping ancestor deletion"));
        }
    }
    Ok(())
}

/// Claim only an absent guard (or our own). Never overwrite another owner,
/// even when recovering a plan whose descendant guards were partly written.
async fn claim_guard(deps: &Deps, session_id: &str, owner: &str) -> Result<(), HarnessError> {
    let current = state::cas_value(
        &deps.iii,
        GUARDS,
        session_id,
        None,
        json!(owner),
        deps.cfg().await.session_timeout_ms,
    )
    .await;
    // Swapped, refused or unknown, a tombstone may now exist: no memoized
    // "live" answer survives the write (see `crate::liveness`).
    deps.liveness.invalidate();
    match current? {
        None => Ok(()),
        Some(value) if value.as_str() == Some(owner) => Ok(()),
        Some(_) => Err(failure(format!(
            "overlapping deletion of {session_id}: foreign guard"
        ))),
    }
}

async fn release_guard(deps: &Deps, session_id: &str, owner: &str) -> Result<(), HarnessError> {
    // A stale rollback cannot erase a replacement owner's reservation.
    let released = state::cas_value(
        &deps.iii,
        GUARDS,
        session_id,
        Some(json!(owner)),
        Value::Null,
        deps.cfg().await.session_timeout_ms,
    )
    .await;
    // Like every guard mutation, invalidate whether or not it applied.
    deps.liveness.invalidate();
    released?;
    Ok(())
}

/// Repair the old inverse-overlap bug only when this operation ALREADY owns
/// its root and the ancestor has not planned/cancelled/deleted anything. The
/// ancestor will revalidate before claiming again; do not mutate its snapshot
/// under another operation's lock. Called only while holding topology.
async fn release_unplanned_ancestors(deps: &Deps, op: &Operation) -> Result<(), HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    let root_guard = state::state_get(&deps.iii, GUARDS, &op.snapshot.session_id, timeout).await?;
    if root_guard.as_str() != Some(&op.snapshot.operation_id) {
        return Ok(());
    }
    let session = deps.session().await;
    let mut current = op.snapshot.session_id.clone();
    let mut seen = BTreeSet::from([current.clone()]);
    while let Some(parent) = session.metadata_of(&current).await?.and_then(|m| {
        m.get("parent_session_id")
            .and_then(Value::as_str)
            .map(str::to_string)
    }) {
        if !seen.insert(parent.clone()) {
            return Err(failure("cyclic session ancestry"));
        }
        let guard = state::state_get(&deps.iii, GUARDS, &parent, timeout).await?;
        if let Some(owner) = guard
            .as_str()
            .filter(|owner| *owner != op.snapshot.operation_id)
        {
            if let Some(ancestor) = load(deps, owner).await? {
                if !ancestor.planned && ancestor.snapshot.session_id == parent {
                    release_guard(deps, &parent, owner).await?;
                }
            }
        }
        current = parent;
    }
    Ok(())
}

/// Bidirectional conflict check BEFORE a root tombstone. The lock order is
/// operation -> topology; regular session work never acquires an operation lock.
/// Planned members supplement the live tree on partial-delete recovery.
async fn reserve_root(deps: &Deps, op: &mut Operation) -> Result<(), HarnessError> {
    release_unplanned_ancestors(deps, op).await?;
    let mut conflict = guard_owner_uncached(deps, &op.snapshot.session_id)
        .await?
        .filter(|owner| owner != &op.snapshot.operation_id)
        .map(Some);
    if conflict.is_none() {
        let tree = super::session_tree::collect(deps, &op.snapshot.session_id).await?;
        if !tree.complete && !tree.sessions.is_empty() {
            return Err(failure("incomplete durable session tree"));
        }
        let ids: BTreeSet<_> = tree
            .sessions
            .into_iter()
            .map(|node| node.session_id)
            .chain(op.members.iter().cloned())
            .collect();
        let timeout = deps.cfg().await.session_timeout_ms;
        for id in ids {
            let guard = state::state_get(&deps.iii, GUARDS, &id, timeout).await?;
            if !guard.is_null() && guard.as_str() != Some(&op.snapshot.operation_id) {
                conflict = Some(guard.as_str().map(str::to_owned));
                break;
            }
        }
    }
    if let Some(owner) = conflict {
        // Roll back only our unplanned root BEFORE optional diagnostic reads.
        // A foreign/planned reservation is never released, even on read errors.
        if !op.planned {
            release_guard(deps, &op.snapshot.session_id, &op.snapshot.operation_id).await?;
        }
        let identity = async {
            let Some(owner) = owner else {
                return Ok(None);
            };
            let Some(existing) = load(deps, &owner).await? else {
                return Ok(None);
            };
            let root = &existing.snapshot.session_id;
            let timeout = deps.cfg().await.session_timeout_ms;
            let root_guard = state::state_get(&deps.iii, GUARDS, root, timeout).await?;
            Ok::<_, HarnessError>(
                (existing.snapshot.operation_id == owner
                    && operation_id(root) == owner
                    && root_guard.as_str() == Some(owner.as_str()))
                .then(|| ExistingDeletion {
                    operation_id: owner,
                    session_id: root.clone(),
                }),
            )
        }
        .await;
        // Identity is optional presentation metadata, not admission authority.
        // Missing/malformed/unavailable backing state stays a generic failure.
        if let Ok(Some(identity)) = identity {
            op.snapshot.failure_code = Some(DeletionFailureCode::OverlappingDeletion);
            op.snapshot.existing_deletion = Some(identity);
            op.snapshot.force_eligible = false;
        }
        return Err(failure("overlapping deletion owned by another operation"));
    }
    claim_guard(deps, &op.snapshot.session_id, &op.snapshot.operation_id).await
}

/// Startup recovery is a single durable scan, never a polling loop. The queue
/// also redelivers a job interrupted by a worker/engine restart.
///
/// Bad data must not become a boot outage: each row is decoded on its own and
/// a malformed one (including a stored `attempt: 0`) is logged and skipped.
/// Its guards stay in place, so the affected subtree remains fail-closed. An
/// enqueue failure is logged too; a repeated delete command re-enqueues.
pub async fn recover(deps: &Deps) -> Result<(), HarnessError> {
    let rows =
        state::list_values::<Value>(&deps.iii, OPERATIONS, deps.cfg().await.session_timeout_ms)
            .await?;
    for row in rows {
        // The storage key is the operation id; `state::list` returns values only.
        let key = row
            .pointer("/snapshot/operation_id")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>")
            .to_string();
        let op = match serde_json::from_value::<Operation>(row) {
            Ok(op) => op,
            Err(error) => {
                tracing::error!(
                    operation_id = %key,
                    %error,
                    "skipping malformed session deletion during recovery; its guards remain"
                );
                continue;
            }
        };
        if op.snapshot.status == DeletionStatus::Deleting {
            if let Err(error) = enqueue(deps, &op.snapshot.operation_id).await {
                tracing::warn!(
                    operation_id = %op.snapshot.operation_id,
                    %error,
                    "could not re-enqueue session deletion during recovery"
                );
            }
        }
    }
    Ok(())
}

pub async fn run(deps: &Deps, req: StatusRequest) -> Result<Option<Snapshot>, HarnessError> {
    let _lock = deps.deletion_commands.guard(&req.operation_id).await;
    let Some(mut op) = load(deps, &req.operation_id).await? else {
        return Ok(None);
    };
    if op.snapshot.status != DeletionStatus::Deleting {
        deps.deletion_events.emit(&op.snapshot).await?;
        return Ok(Some(op.snapshot));
    }
    let remaining = op.deadline.saturating_sub(AgentMessage::now_ms()).max(0) as u64;
    let prepared =
        tokio::time::timeout(Duration::from_millis(remaining), prepare(deps, &mut op)).await;
    let result = match prepared {
        Ok(Ok(())) => erase(deps, &mut op).await,
        Ok(Err(e)) => Err(e),
        Err(_) => Err(failure(
            "deletion deadline expired; completion is not confirmed",
        )),
    };
    match result {
        Ok(()) => {
            op.snapshot.status = DeletionStatus::Completed;
            op.snapshot.error = None;
            op.snapshot.force_eligible = false;
            op.snapshot.failure_code = None;
            op.snapshot.existing_deletion = None;
            op.snapshot.blockers.clear();
        }
        Err(e) => {
            op.snapshot.status = DeletionStatus::Failed;
            tracing::warn!(error = %e, "session tree deletion failed");
            op.snapshot.error = Some(public_error(&e));
            if op.snapshot.failure_code.is_none() {
                op.snapshot.failure_code = Some(DeletionFailureCode::Failed);
                op.snapshot.force_eligible = false;
            }
        }
    }
    save(deps, &op).await?;
    deps.deletion_events.emit(&op.snapshot).await?;
    Ok(Some(op.snapshot))
}

async fn prepare(deps: &Deps, op: &mut Operation) -> Result<(), HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    // Only creation/append admission holds topology, never a running tool or
    // a wait on the session lock. Existing spawns finish their metadata write
    // before this enumeration; subsequent spawns see the ancestor guard.
    let topology = deps.topology.lock().await;
    reserve_root(deps, op).await?;
    if !op.planned {
        let session = deps.session().await;
        let metadata = session.metadata_of(&op.snapshot.session_id).await?;
        let tree = super::session_tree::collect(deps, &op.snapshot.session_id).await?;
        if metadata.is_some() && !tree.complete {
            return Err(failure("incomplete durable session tree"));
        }
        op.links = tree
            .sessions
            .iter()
            .map(|n| (n.session_id.clone(), n.parent_session_id.clone()))
            .collect();
        op.legacy_links_missing = false;
        op.members = tree.sessions.into_iter().map(|n| n.session_id).collect();
        op.parent = metadata
            .as_ref()
            .and_then(|m| m.get("parent_session_id"))
            .and_then(Value::as_str)
            .map(str::to_string);
        op.name = metadata
            .as_ref()
            .and_then(|m| m.get("display"))
            .and_then(|d| d.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or(session.turn_hints(&op.snapshot.session_id).await.title)
            .unwrap_or_else(|| op.snapshot.session_id.clone());
        // Validate the whole set before claiming descendants: overlapping
        // operations fail observably rather than erase or notify twice.
        for id in &op.members {
            let guard = state::state_get(&deps.iii, GUARDS, id, timeout).await?;
            if !guard.is_null() && guard.as_str() != Some(&op.snapshot.operation_id) {
                return Err(failure(format!(
                    "overlapping deletion of {id}: foreign guard"
                )));
            }
        }
        op.planned = true;
        save(deps, op).await?;
    }
    validate_or_migrate_plan(deps, op).await?;
    op.snapshot.remaining_session_ids = op
        .members
        .iter()
        .filter(|id| !op.snapshot.deleted_session_ids.contains(id))
        .cloned()
        .collect();
    for id in &op.members {
        claim_guard(deps, id, &op.snapshot.operation_id).await?;
    }
    drop(topology);

    // Signal EVERY member first, even when the selected root is terminal.
    // Never interpret a stop acknowledgement as terminality.
    let mut records = Vec::new();
    for id in &op.members {
        if let Some(record) = state::get_turn_unhydrated(&deps.iii, id, timeout).await? {
            if !record.status.is_terminal() {
                deps.cancels.fire(&record.turn_id);
            }
            records.push(record);
        }
    }
    let mut hard_errors = Vec::new();
    for record in &records {
        if !record.status.is_terminal() {
            // Bounded best effort. Never wait forever behind an in-flight tool.
            // The lock-free cancel signal has already been fired for EVERY member.
            match super::stop::stop_for_deletion(
                deps,
                super::stop::StopRequest {
                    session_id: record.session_id.clone(),
                    turn_id: Some(record.turn_id.clone()),
                },
            )
            .await
            {
                Ok(outcome) if outcome.hard_failure => hard_errors.extend(outcome.unconfirmed),
                Ok(_) => {}
                Err(error) => hard_errors.push(error.to_string()),
            }
        }
    }
    // A generic dependency failure is never a force-eligible result.
    if !hard_errors.is_empty() {
        tracing::warn!(?hard_errors, "deletion cancellation dependency failed");
        return Err(failure(
            "cancellation dependency failed (router::abort); completion is not confirmed",
        ));
    }
    let grace = if op.snapshot.mode == DeletionMode::Normal {
        LOCAL_CANCEL_WAIT_MS
    } else {
        FORCE_WRITER_WAIT_MS
    };
    let wait_until = AgentMessage::now_ms()
        .saturating_add(grace as i64)
        .min(op.deadline - 10);
    loop {
        let notified = deps.deletion_changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let busy = if op.snapshot.mode == DeletionMode::Normal {
            let blockers = current_blockers(deps, op, op.deadline).await?;
            // Only durable uncertainty fails promptly. Healthy local dispatches
            // and Triggered calls with a local writer share the bounded cancel grace.
            if blockers
                .iter()
                .any(|b| b.kind != BlockerKind::ActiveProcessing)
            {
                return Err(blocked(op, blockers));
            }
            blockers
        } else {
            let mut busy = Vec::new();
            for id in &op.members {
                let activity = deps.turn_activity.try_guard(id);
                let lock = deps.locks.try_guard(id);
                if activity.is_none() || lock.is_none() {
                    busy.push(active_blocker(id));
                }
            }
            busy
        };
        if busy.is_empty() {
            break;
        }
        let remaining = wait_until.saturating_sub(AgentMessage::now_ms()).max(0) as u64;
        if remaining == 0 {
            return Err(blocked(op, busy));
        }
        // Writer-release has no notification channel; bounded rechecks also
        // cover a lost turn notification without treating 1 s latency as failure.
        let _ = tokio::time::timeout(Duration::from_millis(remaining.min(100)), notified).await;
    }
    Ok(())
}

async fn current_blockers(
    deps: &Deps,
    op: &Operation,
    scan_deadline: i64,
) -> Result<Vec<DeletionBlocker>, HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    let mut blockers = Vec::new();
    // Freeze witness admission while diagnosing, without waiting behind it.
    // A busy admission is active even if its witness is not yet visible.
    let admissions: Vec<_> = op
        .members
        .iter()
        .map(|id| (id, deps.dispatch_admission.try_guard(id)))
        .collect();
    for (id, guard) in &admissions {
        if guard.is_none() {
            blockers.push(active_blocker(id));
        }
    }
    // Snapshot BEFORE reading witnesses: a reply can clear its witness and
    // drop the ticket while the RPC is in flight. Recheck on the next pass,
    // rather than falsely calling that stale row an orphan.
    let live_at_read = deps
        .live_dispatches
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    for id in &op.members {
        let activity = deps.turn_activity.try_guard(id);
        let lock = deps.locks.try_guard(id);
        if activity.is_none() || lock.is_none() {
            blockers.push(active_blocker(id));
        }
        if let Some(record) = state::get_turn_unhydrated(&deps.iii, id, timeout).await? {
            // Resolve/approved spawn holds only the session lock; detached or
            // direct dispatch can hold neither. All local owners share grace.
            let locally_active = activity.is_none()
                || lock.is_none()
                || live_at_read.values().any(|session| session == id)
                || deps
                    .live_dispatches
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .values()
                    .any(|session| session == id);
            for (call_id, call) in record.calls {
                if call.state == CallState::Triggered
                    || (call.state == CallState::Pending
                        && call.held_by.is_none()
                        && call.child_session_id.is_none())
                {
                    blockers.push(DeletionBlocker {
                        kind: if call.state == CallState::Triggered && locally_active {
                            BlockerKind::ActiveProcessing
                        } else if call.state == CallState::Triggered {
                            BlockerKind::UnknownCompletion
                        } else {
                            BlockerKind::UnconfirmedCancellation
                        },
                        session_id: id.clone(),
                        function_id: call.function_id,
                        call_id: Some(call_id),
                        started_at: Some(record.updated_at),
                    });
                }
            }
            if !record.status.is_terminal()
                && !blockers
                    .iter()
                    .any(|b| b.session_id == *id && b.kind != BlockerKind::ActiveProcessing)
            {
                blockers.push(DeletionBlocker {
                    kind: BlockerKind::ActiveProcessing,
                    session_id: id.clone(),
                    function_id: None,
                    call_id: None,
                    started_at: Some(record.updated_at),
                });
            }
        }
    }
    for (key, _, w) in dispatch_entries(deps, op, timeout, scan_deadline).await? {
        let admission_busy = admissions
            .iter()
            .any(|(id, guard)| **id == w.session_id && guard.is_none());
        let live = live_at_read.contains_key(&key)
            || deps
                .live_dispatches
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .contains_key(&key);
        blockers.push(DeletionBlocker {
            kind: if live || admission_busy {
                BlockerKind::ActiveProcessing
            } else {
                BlockerKind::UnknownCompletion
            },
            session_id: w.session_id,
            function_id: Some(w.function_id),
            call_id: w.call_id,
            started_at: w.started_at,
        });
    }
    deduplicate_blockers(&mut blockers);
    Ok(blockers)
}

fn deduplicate_blockers(blockers: &mut Vec<DeletionBlocker>) {
    let mut seen = BTreeSet::new();
    blockers.retain(|b| seen.insert((b.kind as u8, b.session_id.clone(), b.call_id.clone())));
}

fn active_blocker(id: &str) -> DeletionBlocker {
    DeletionBlocker {
        kind: BlockerKind::ActiveProcessing,
        session_id: id.into(),
        function_id: None,
        call_id: None,
        started_at: None,
    }
}

fn blocked(op: &mut Operation, blockers: Vec<DeletionBlocker>) -> HarnessError {
    op.snapshot.blockers = blockers;
    deduplicate_blockers(&mut op.snapshot.blockers);
    op.snapshot.failure_code = Some(DeletionFailureCode::Blocked);
    op.snapshot.force_eligible = op.snapshot.mode == DeletionMode::Normal;
    op.snapshot.data_retained = !op.cleanup_started
        && !op.notified
        && op.snapshot.deleted_session_ids.is_empty()
        && op.snapshot.unconfirmed_session_ids.is_empty()
        && op.erasing.is_none();
    if op.snapshot.data_retained {
        failure("deletion blocked; unconfirmed work and data retained")
    } else {
        failure("deletion blocked; remaining sessions are not deleted or unconfirmed")
    }
}

async fn erase(deps: &Deps, op: &mut Operation) -> Result<(), HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    // Capture once per RUN, after closing every target's in-flight admission.
    // Guard ownership rejects future admissions; these barriers drain prior ones.
    let mut admissions = Vec::new();
    let entries = if op.snapshot.mode == DeletionMode::Force {
        for id in &op.members {
            let Some(guard) = deps.dispatch_admission.try_guard(id) else {
                return Err(blocked(op, vec![active_blocker(id)]));
            };
            admissions.push(guard);
        }
        let remaining = deadline_remaining(op)?;
        let entries = tokio::time::timeout(
            Duration::from_millis(remaining),
            dispatch_entries(deps, op, timeout.min(remaining), op.deadline),
        )
        .await
        .map_err(|_| {
            failure("dispatch snapshot deadline expired; completion is not confirmed")
        })??;
        deadline_remaining(op)?;
        entries
    } else {
        Vec::new()
    };
    op.snapshot.data_retained = false;
    if !op.cleanup_started {
        op.cleanup_started = true;
        save(deps, op).await?;
    }
    if !op.notified {
        if let Some(parent) = op
            .parent
            .as_deref()
            .filter(|p| !op.members.iter().any(|id| id == p))
        {
            notify_parent(deps, parent, op).await?;
        }
        op.notified = true;
        save(deps, op).await?;
    }
    // Children first: a partial failure leaves the surviving root metadata
    // available, and the saved plan retains already-deleted members for retry.
    for id in op.members.clone().into_iter().rev() {
        if AgentMessage::now_ms() >= op.deadline {
            return Err(failure(
                "deletion cleanup deadline expired; remaining sessions are not deleted or unconfirmed",
            ));
        }
        if op.snapshot.deleted_session_ids.contains(&id) {
            continue;
        }
        let Some(_activity) = deps.turn_activity.try_guard(&id) else {
            return Err(blocked(op, vec![active_blocker(&id)]));
        };
        let Some(_lock) = deps.locks.try_guard(&id) else {
            return Err(blocked(op, vec![active_blocker(&id)]));
        };
        // Admission barrier only: the subtree's guards are already claimed, so
        // any send, spawn or binding that takes the topology lock from here on
        // is refused. Holding it across cleanup and session::delete would
        // stall every topology writer in the process for those RPCs.
        drop(deps.topology.lock().await);
        if let Some(mut record) = state::get_turn_unhydrated(&deps.iii, &id, timeout).await? {
            if op.snapshot.mode == DeletionMode::Force && !record.status.is_terminal() {
                // Both writer barriers are held. This is local abandonment, NOT
                // a claim that an arbitrary remote invocation was terminated.
                record.abort = true;
                record.status = crate::types::turn::TurnStatus::Cancelled;
                record.updated_at = AgentMessage::now_ms();
                record.slim_finished();
                state::put_turn(&deps.iii, &record, timeout).await?;
            }
            if op.snapshot.mode == DeletionMode::Force {
                deps.cancels.clear(&record.turn_id);
            }
            if !record.status.is_terminal() && op.snapshot.mode == DeletionMode::Normal {
                return Err(failure(format!("{id} became nonterminal; refusing erase")));
            }
        }
        if op.snapshot.mode == DeletionMode::Force {
            purge_dispatches(deps, op, &id, &entries).await?;
        }
        cleanup(deps, &id).await?;
        op.erasing = Some(id.clone());
        if !op.snapshot.unconfirmed_session_ids.contains(&id) {
            op.snapshot.unconfirmed_session_ids.push(id.clone());
        }
        save(deps, op).await?;
        let response = deps
            .iii
            .trigger(TriggerRequest {
                function_id: "session::delete".into(),
                payload: json!({"session_id": id}),
                action: None,
                timeout_ms: Some(timeout),
            })
            .await
            .map_err(|e| HarnessError::Dependency(format!("session::delete {id}: {e}")))?;
        #[derive(Deserialize)]
        struct DeleteAck {
            deleted: bool,
        }
        let ack: DeleteAck = serde_json::from_value(response).map_err(|e| {
            HarnessError::Dependency(format!(
                "session::delete {id}: malformed acknowledgement: {e}"
            ))
        })?;
        // false is a legitimate replay after a lost delete acknowledgement,
        // but only absence proves it. Also reject a contradictory true reply.
        if deps.session().await.exists(&id).await? {
            op.snapshot
                .unconfirmed_session_ids
                .retain(|unknown| unknown != &id);
            return Err(failure(format!(
                "session::delete {id} returned deleted={} but the session still exists",
                ack.deleted
            )));
        }
        state::delete_turn(&deps.iii, &id, timeout).await?;
        op.snapshot
            .remaining_session_ids
            .retain(|remaining| remaining != &id);
        op.snapshot
            .unconfirmed_session_ids
            .retain(|unknown| unknown != &id);
        op.snapshot.deleted_session_ids.push(id);
        op.erasing = None;
        save(deps, op).await?;
    }
    Ok(())
}

/// Ignore malformed foreign rows only when a typed session id disproves
/// membership. Missing/invalid identity or malformed target values fail closed.
async fn dispatch_entries(
    deps: &Deps,
    op: &Operation,
    timeout: u64,
    scan_deadline: i64,
) -> Result<Vec<(String, Value, DispatchWitness)>, HarnessError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Page {
        entries: Vec<(String, Value)>,
        next_cursor: Value,
        done: bool,
        offset: usize,
        total: usize,
    }
    let mut cursor: Option<String> = None;
    let mut cursors = BTreeSet::new();
    let mut keys = BTreeSet::new();
    let mut entries = Vec::new();
    let mut total = None;
    loop {
        // RUN uses its original durable deadline; read-only revalidation uses
        // a fixed diagnosis deadline. Neither is renewed between pages.
        let remaining = scan_remaining(scan_deadline)?;
        let response = tokio::time::timeout(
            Duration::from_millis(remaining),
            state::state_list_entries(
                &deps.iii,
                DISPATCHES,
                cursor.as_deref(),
                timeout.min(remaining),
            ),
        )
        .await
        .map_err(|_| failure("dispatch scan deadline expired; completion is not confirmed"))??;
        scan_remaining(scan_deadline)?;
        if !page_within_budget(&response) {
            return Err(failure("oversized dispatch page"));
        }
        if response.get("next_cursor").is_none() {
            return Err(failure("missing dispatch terminal indication"));
        }
        let page: Page = serde_json::from_value(response)
            .map_err(|_| failure("malformed dispatch page metadata or entries"))?;
        if page.offset != entries.len()
            || page.total > 100_000
            || total.is_some_and(|total| total != page.total)
            || page.entries.len() > 100
            || page.offset + page.entries.len() > page.total
        {
            return Err(failure("inconsistent dispatch page metadata"));
        }
        total = Some(page.total);
        let end = page.offset + page.entries.len();
        let next = if page.done {
            if !page.next_cursor.is_null() || end != page.total {
                return Err(failure("missing dispatch terminal indication"));
            }
            None
        } else {
            let token = page
                .next_cursor
                .as_str()
                .filter(|token| !token.is_empty() && token.len() <= 256)
                .ok_or_else(|| failure("missing dispatch continuation"))?;
            if page.entries.is_empty() || end >= page.total || !cursors.insert(token.to_owned()) {
                return Err(failure("looping or non-progressing dispatch cursor"));
            }
            Some(token.to_owned())
        };
        for (key, value) in page.entries {
            if !keys.insert(key.clone()) {
                return Err(failure("duplicate dispatch snapshot key"));
            }
            entries.push((key, value));
        }
        cursor = next;
        if page.done {
            break;
        }
    }
    scan_remaining(scan_deadline)?;
    let mut target = Vec::new();
    for (key, value) in entries {
        scan_remaining(scan_deadline)?;
        if value.is_null() {
            continue;
        }
        let sid = value
            .get("session_id")
            .and_then(Value::as_str)
            .ok_or_else(|| failure("dispatch membership cannot be verified"))?;
        if !op.members.iter().any(|id| id == sid) {
            continue;
        }
        let witness = serde_json::from_value(value.clone())
            .map_err(|_| failure("malformed target dispatch witness"))?;
        target.push((key, value, witness));
    }
    scan_remaining(scan_deadline)?;
    Ok(target)
}

/// Reject oversized replies without allocating a second serialized copy.
fn page_within_budget(value: &Value) -> bool {
    struct Budget(usize);
    impl std::io::Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.0 {
                return Err(std::io::Error::other("page too large"));
            }
            self.0 -= bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(&mut Budget(1_000_000), value).is_ok()
}

fn deadline_remaining(op: &Operation) -> Result<u64, HarnessError> {
    scan_remaining(op.deadline)
}

fn scan_remaining(deadline: i64) -> Result<u64, HarnessError> {
    let remaining = deadline.saturating_sub(AgentMessage::now_ms());
    if remaining <= 0 {
        return Err(failure(
            "deletion cleanup deadline expired; completion is not confirmed",
        ));
    }
    Ok(remaining as u64)
}

/// The whole target's admission barriers are held before the complete paginated snapshot.
/// CAS preserves replacement/foreign witnesses; a late reply can see null.
async fn purge_dispatches(
    deps: &Deps,
    op: &Operation,
    session_id: &str,
    entries: &[(String, Value, DispatchWitness)],
) -> Result<(), HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    for (key, value, _) in entries
        .iter()
        .filter(|(_, _, w)| w.session_id == session_id)
    {
        let remaining = deadline_remaining(op)?;
        let current = tokio::time::timeout(
            Duration::from_millis(remaining),
            state::cas_value(
                &deps.iii,
                DISPATCHES,
                key,
                Some(value.clone()),
                Value::Null,
                timeout.min(remaining),
            ),
        )
        .await
        .map_err(|_| failure("dispatch cleanup deadline expired; completion is not confirmed"))??;
        if current.is_some_and(|current| !current.is_null()) {
            return Err(failure("dispatch changed during cleanup"));
        }
        deadline_remaining(op)?;
    }
    Ok(())
}

async fn cleanup(deps: &Deps, id: &str) -> Result<(), HarnessError> {
    deps.hooks.unregister_session(id);
    let timeout = deps.cfg().await.session_timeout_ms;
    let store = deps.bindings().await;
    for binding in
        state::list_values::<crate::bindings::Binding>(&deps.iii, state::BINDING_SCOPE, timeout)
            .await?
            .into_iter()
            .filter(|binding| binding.owner.session_id == id)
    {
        if let Some(trigger) = &binding.trigger_id {
            if trigger.starts_with(super::subscribe::SDK_TRIGGER_ID_PREFIX) {
                if !super::subscribe::unregister_engine_trigger(deps, trigger).await {
                    return Err(failure(format!("failed to unregister {trigger}")));
                }
            } else {
                deps.iii
                    .trigger(TriggerRequest {
                        function_id: "engine::unregister_trigger".into(),
                        payload: json!({"id":trigger}),
                        action: None,
                        timeout_ms: Some(timeout),
                    })
                    .await
                    .map_err(|e| HarnessError::Dependency(format!("unregister {trigger}: {e}")))?;
            }
        }
        store.delete(&binding.id).await?;
    }
    // Retiring a binding can leave its capacity index after an interrupted
    // acknowledgement. Clear it explicitly rather than silently keeping it.
    state::cas_value(
        &deps.iii,
        state::BINDING_OWNER_SCOPE,
        id,
        Some(state::state_get(&deps.iii, state::BINDING_OWNER_SCOPE, id, timeout).await?),
        Value::Null,
        timeout,
    )
    .await?
    .map_or(Ok(()), |_| {
        Err(failure("binding owner index changed during deletion"))
    })?;
    for row in state::list_queued(&deps.iii, id, timeout).await? {
        state::delete_queued(&deps.iii, id, &row.id, timeout).await?;
    }
    crate::filesystem_grants::purge(&deps.iii, id, timeout).await?;
    crate::budget::purge(deps, id, timeout).await?;
    crate::context_snapshot::delete(&deps.iii, id, timeout).await?;
    // Approval owns its data. Its existing purge is best effort; explicitly
    // read the inbox afterwards so a failed purge cannot be reported completed.
    let purge = deps
        .iii
        .trigger(TriggerRequest {
            function_id: "approval::on-session-deleted".into(),
            payload: json!({"session_id": id}),
            action: None,
            timeout_ms: Some(timeout),
        })
        .await;
    match purge {
        Err(iii_sdk::Error::Remote { code, .. })
            if code.eq_ignore_ascii_case("function_not_found") => {}
        Err(e) => {
            return Err(HarnessError::Dependency(format!(
                "approval cleanup {id}: {e}"
            )))
        }
        Ok(_) => {
            let pending = deps
                .iii
                .trigger(TriggerRequest {
                    function_id: "approval::list-pending".into(),
                    payload: json!({"session_id": id, "limit": 1}),
                    action: None,
                    timeout_ms: Some(timeout),
                })
                .await
                .map_err(|e| HarnessError::Dependency(format!("approval verification: {e}")))?;
            if !pending
                .get("pending")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
            {
                return Err(failure(format!("approval cleanup not confirmed for {id}")));
            }
            let settings = deps
                .iii
                .trigger(TriggerRequest {
                    function_id: "approval::get-settings".into(),
                    payload: json!({"session_id":id}),
                    action: None,
                    timeout_ms: Some(timeout),
                })
                .await
                .map_err(|e| {
                    HarnessError::Dependency(format!("approval settings verification: {e}"))
                })?;
            if settings.get("source").and_then(Value::as_str) != Some("defaults") {
                return Err(failure(format!(
                    "approval settings cleanup not confirmed for {id}"
                )));
            }
        }
    }
    Ok(())
}

fn notification(op: &Operation) -> String {
    format!("User-requested cancellation and deletion of session {} ({}) and its subtree: {}. Best-effort cancellation was requested; these sessions are being deleted. External operations may continue and prior side effects are not undone. Do not await their results and do not recreate them automatically. Your history, execution and other children are preserved. Operation: {}.",
        op.name, op.snapshot.session_id, op.members.join(", "), op.snapshot.operation_id)
}

async fn notify_parent(deps: &Deps, parent: &str, op: &Operation) -> Result<(), HarnessError> {
    let cfg = deps.cfg().await;
    let topology = deps.topology.lock().await;
    if guard_owner_uncached(deps, parent).await?.is_some()
        || !deps.session().await.exists(parent).await?
    {
        return Ok(());
    }
    let Some(record) = state::get_turn(&deps.iii, parent, cfg.session_timeout_ms).await? else {
        return Err(failure(format!(
            "surviving parent {parent} has no model/options for notification"
        )));
    };
    let entry_id = format!("e_{}_parent", op.snapshot.operation_id);
    if deps
        .session()
        .await
        .messages_strict(parent)
        .await?
        .iter()
        .any(|entry| entry.entry_id == entry_id)
    {
        return Ok(());
    }
    // Always queue first. Only a running/seeded turn drains the deterministic
    // entry into history, so a crash cannot append a notice with no consumer.
    let row = state::QueuedMessage {
        id: entry_id.clone(),
        session_id: parent.into(),
        entry_id: entry_id.clone(),
        message: AgentMessage::user_text(notification(op)),
        origin: Some(json!({"deletion_operation_id": op.snapshot.operation_id})),
        queued_at: AgentMessage::now_ms(),
    };
    state::enqueue_message(&deps.iii, &row, cfg.session_timeout_ms).await?;
    drop(topology);
    let latest = state::get_turn(&deps.iii, parent, cfg.session_timeout_ms).await?;
    if latest.as_ref().is_some_and(|r| !r.status.is_terminal()) {
        // The active parent keeps its execution and consumes the durable queue.
        // Do not wait behind its in-flight tools merely to park a notification.
        if let Some(latest) =
            latest.filter(|r| r.status != crate::types::turn::TurnStatus::AwaitingFunctions)
        {
            crate::turn_loop::enqueue_step(
                &deps.iii,
                parent,
                &latest.turn_id,
                latest.step,
                latest.message_preview.as_deref(),
                latest.depth,
            )
            .await?;
        }
    } else {
        let _lock = deps.locks.guard(parent).await;
        let _topology = deps.topology.lock().await;
        if guard_owner_uncached(deps, parent).await?.is_some()
            || !deps.session().await.exists(parent).await?
        {
            return Ok(());
        }
        // The finalizer might already have drained and reseeded the parent.
        let latest = state::get_turn(&deps.iii, parent, cfg.session_timeout_ms).await?;
        if latest.as_ref().is_some_and(|r| r.status.is_terminal())
            && !deps
                .session()
                .await
                .messages_strict(parent)
                .await?
                .iter()
                .any(|entry| entry.entry_id == entry_id)
        {
            let prior = latest.as_ref().unwrap_or(&record);
            super::send::seed_new(
                deps,
                &cfg,
                parent,
                prior.options.clone(),
                Some(prior),
                Some("User cancelled a child subtree".into()),
                &super::send::TurnLineage::continuing(prior),
            )
            .await?;
        }
    }
    deps.events
        .emit_queued(parent, &entry_id, row.queued_at)
        .await;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DispatchWitness {
    pub session_id: String,
    pub function_id: String,
    #[serde(default)]
    pub call_id: Option<String>,
    #[serde(default)]
    pub started_at: Option<i64>,
}

/// Must be persisted before dispatch; cleared only on a confirmed reply.
///
/// The short per-session admission barrier also covers Force's keyed purge.
/// Check the durable tombstone BEFORE writing, then recheck after persistence
/// in case guard claiming raced this admission. If admission wins the barrier,
/// its witness is visible to the later purge; if purge wins, no witness is
/// written afterwards. Never hold this barrier across the remote invocation
/// or acquire the process-wide topology/session-writer locks here.
pub(crate) async fn begin_dispatch(
    deps: &Deps,
    session_id: &str,
    function_id: &str,
) -> Result<DispatchTicket, HarnessError> {
    let _admission = deps.dispatch_admission.guard(session_id).await;
    ensure_live(deps, session_id).await?;
    let mut ticket = DispatchTicket {
        id: uuid::Uuid::new_v4().to_string(),
        local: None,
        witness: json!(DispatchWitness {
            session_id: session_id.into(),
            function_id: function_id.into(),
            call_id: None,
            started_at: Some(AgentMessage::now_ms()),
        }),
    };
    // The key is fresh, so one CAS from absent writes it: no read-then-swap.
    if let Some(current) = state::cas_value(
        &deps.iii,
        DISPATCHES,
        &ticket.id,
        None,
        ticket.witness.clone(),
        deps.cfg().await.session_timeout_ms,
    )
    .await?
    {
        return Err(failure(format!(
            "dispatch witness {} already present: {current}",
            ticket.id
        )));
    }
    if let Err(refused) = ensure_live(deps, session_id).await {
        // Nothing was dispatched. A failed withdrawal stays visible: Force's
        // purge cannot take the admission barrier until this path returns.
        if let Err(error) = end_dispatch(deps, &ticket).await {
            tracing::warn!(
                session_id,
                function_id,
                %error,
                "could not withdraw the witness of a refused dispatch"
            );
        }
        return Err(refused);
    }
    deps.live_dispatches
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(ticket.id.clone(), session_id.into());
    ticket.local = Some((deps.live_dispatches.clone(), deps.deletion_changed.clone()));
    Ok(ticket)
}

/// A persisted dispatch witness: its key and the exact value written, so it
/// is cleared by one CAS from that value.
type LocalDispatch = (
    std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<String, String>>>,
    std::sync::Arc<tokio::sync::Notify>,
);

pub(crate) struct DispatchTicket {
    id: String,
    witness: Value,
    local: Option<LocalDispatch>,
}

impl Drop for DispatchTicket {
    fn drop(&mut self) {
        // Owned by subscribe::invoke itself, including the detached task after
        // dispatch_unless_stopped returns None. Errors/unwind also retire local
        // liveness, but never erase an uncertain durable witness.
        if let Some((live, changed)) = self.local.take() {
            live.lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&self.id);
            changed.notify_waiters();
        }
    }
}

pub(crate) async fn end_dispatch(deps: &Deps, ticket: &DispatchTicket) -> Result<(), HarnessError> {
    match state::cas_value(
        &deps.iii,
        DISPATCHES,
        &ticket.id,
        Some(ticket.witness.clone()),
        Value::Null,
        deps.cfg().await.session_timeout_ms,
    )
    .await?
    {
        // Cleared now, or by an earlier attempt whose reply was lost.
        None => {}
        Some(current) if current.is_null() => {}
        Some(current) => {
            return Err(failure(format!(
                "dispatch witness {} changed under its dispatch: {current}",
                ticket.id
            )))
        }
    }
    deps.deletion_changed.notify_waiters();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ticket belongs to the invocation task, not the cancelled waiter.
    #[tokio::test]
    async fn detached_dispatch_ticket_stays_live_until_task_returns_or_unwinds() {
        for panic in [false, true] {
            let live = std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::BTreeMap::from([("ticket".into(), "session".into())]),
            ));
            let changed = std::sync::Arc::new(tokio::sync::Notify::new());
            let ticket = DispatchTicket {
                id: "ticket".into(),
                witness: Value::Null,
                local: Some((live.clone(), changed.clone())),
            };
            let (started_tx, started) = tokio::sync::oneshot::channel();
            let (reply, replied) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                let _ticket = ticket;
                started_tx.send(()).unwrap();
                replied.await.unwrap();
                assert!(!panic, "simulated invocation unwind");
            });
            started.await.unwrap();
            // Dropping the cancellation wrapper's JoinHandle detaches, not aborts.
            drop(task);
            assert!(live.lock().unwrap().contains_key("ticket"));
            let retired = changed.notified();
            tokio::pin!(retired);
            retired.as_mut().enable();
            reply.send(()).unwrap();
            tokio::time::timeout(Duration::from_secs(1), retired)
                .await
                .unwrap();
            assert!(live.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn only_stored_legacy_operations_default_attempt_to_one() {
        let snapshot = json!({"operation_id":"op", "session_id":"child2", "status":"failed", "deleted_session_ids":[]});
        assert!(serde_json::from_value::<Snapshot>(snapshot.clone()).is_err());
        let stored = json!({"snapshot":snapshot, "deadline":0, "members":[], "parent":null,
            "name":"child2", "planned":false, "notified":false});
        assert_eq!(
            serde_json::from_value::<Operation>(stored)
                .unwrap()
                .snapshot
                .attempt,
            1
        );
    }

    #[test]
    fn operation_identity_is_stable_and_scoped_to_selected_root() {
        assert_eq!(operation_id("child2"), operation_id("child2"));
        assert_ne!(operation_id("parent"), operation_id("child2"));
    }

    #[test]
    fn notification_names_cancellation_not_a_successful_child_result() {
        let op = Operation {
            snapshot: Snapshot {
                operation_id: "op".into(),
                attempt: 1,
                session_id: "child2".into(),
                status: DeletionStatus::Deleting,
                deleted_session_ids: vec![],
                remaining_session_ids: vec![],
                unconfirmed_session_ids: vec![],
                mode: DeletionMode::Normal,
                data_retained: false,
                blockers: vec![],
                force_eligible: false,
                failure_code: None,
                existing_deletion: None,
                error: None,
            },
            deadline: 0,
            members: vec!["child2".into(), "grandchild1".into()],
            parent: Some("parent".into()),
            name: "Research".into(),
            planned: true,
            notified: false,
            cleanup_started: false,
            force_confirmed_attempt: None,
            links: vec![],
            legacy_links_missing: false,
            legacy_head_checkpoint: false,
            erasing: None,
        };
        let text = notification(&op);
        for required in [
            "Research",
            "child2",
            "grandchild1",
            "grandchild1",
            "User-requested cancellation",
            "Do not await",
            "do not recreate",
        ] {
            assert!(text.contains(required), "missing {required}");
        }
        assert!(!text.contains(", child1"));
    }

    #[test]
    fn public_snapshot_rejects_zero_attempt() {
        let snapshot = json!({"operation_id":"op", "attempt":0, "session_id":"s", "status":"deleting", "deleted_session_ids":[]});
        assert!(serde_json::from_value::<Snapshot>(snapshot).is_err());
    }

    #[test]
    fn snapshot_schema_rejects_zero_attempt() {
        let schema = serde_json::to_value(crate::surface::schema_value::<Snapshot>()).unwrap();
        let validator = jsonschema::JSONSchema::compile(&schema).unwrap();
        let valid = json!({"operation_id":"op", "attempt":1, "session_id":"s", "status":"deleting", "deleted_session_ids":[]});
        let invalid = json!({"operation_id":"op", "attempt":0, "session_id":"s", "status":"deleting", "deleted_session_ids":[]});
        assert!(validator.is_valid(&valid));
        assert!(!validator.is_valid(&invalid));
    }
}
