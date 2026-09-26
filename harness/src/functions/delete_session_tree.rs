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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
// Like other function inputs, tolerate metadata injected by the engine
// (notably _caller_worker_id). Domain identifiers remain required.
pub struct DeleteRequest {
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
    serde_json::from_value(value)
        .map(Some)
        .map_err(|e| HarnessError::State(e.to_string()))
}

async fn save(deps: &Deps, operation: &Operation) -> Result<(), HarnessError> {
    state::state_set(
        &deps.iii,
        OPERATIONS,
        &operation.snapshot.operation_id,
        serde_json::to_value(operation).map_err(|e| HarnessError::State(e.to_string()))?,
        deps.cfg().await.session_timeout_ms,
    )
    .await
}

pub async fn status(deps: &Deps, req: StatusRequest) -> Result<Option<Snapshot>, HarnessError> {
    Ok(load(deps, &req.operation_id).await?.map(|op| op.snapshot))
}

/// Check the durable ancestry, so a not-yet-enumerated descendant is already
/// barred by its root's tombstone. Metadata errors fail closed.
pub(crate) async fn guard_owner(
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
                error: None,
            },
            deadline: AgentMessage::now_ms() + DEADLINE_MS,
            members: Vec::new(),
            parent: None,
            name: req.session_id.clone(),
            planned: false,
            notified: false,
        },
    };
    // Save before admission is closed: recovery can finish a partial guard write.
    save(deps, &op).await?;
    let admission = deps.topology.lock().await;
    let reserve = reserve_root(deps, &op).await;
    drop(admission);
    let accepted = match reserve {
        Ok(()) => enqueue(deps, &id).await,
        Err(e) => Err(e),
    };
    if let Err(error) = accepted {
        op.snapshot.status = DeletionStatus::Failed;
        op.snapshot.error = Some(error.to_string());
        save(deps, &op).await?;
        deps.deletion_events.emit(&op.snapshot).await?;
    }
    Ok(op.snapshot)
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
    .await?;
    match current {
        None => Ok(()),
        Some(value) if value.as_str() == Some(owner) => Ok(()),
        Some(value) => Err(failure(format!(
            "overlapping deletion of {session_id}: {value}"
        ))),
    }
}

async fn release_guard(deps: &Deps, session_id: &str, owner: &str) -> Result<(), HarnessError> {
    // A stale rollback cannot erase a replacement owner's reservation.
    state::cas_value(
        &deps.iii,
        GUARDS,
        session_id,
        Some(json!(owner)),
        Value::Null,
        deps.cfg().await.session_timeout_ms,
    )
    .await?;
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
async fn reserve_root(deps: &Deps, op: &Operation) -> Result<(), HarnessError> {
    release_unplanned_ancestors(deps, op).await?;
    let mut conflict = guard_owner(deps, &op.snapshot.session_id)
        .await?
        .filter(|owner| owner != &op.snapshot.operation_id);
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
                conflict = Some(format!("{id}: {guard}"));
                break;
            }
        }
    }
    if let Some(conflict) = conflict {
        // Before planning, our only possible write is the root reservation.
        // A rejected ancestor must not stop its own work or surviving siblings.
        if !op.planned {
            release_guard(deps, &op.snapshot.session_id, &op.snapshot.operation_id).await?;
        }
        return Err(failure(format!("overlapping deletion owned by {conflict}")));
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
            "deletion deadline expired; unconfirmed sessions and data retained",
        )),
    };
    match result {
        Ok(()) => {
            op.snapshot.status = DeletionStatus::Completed;
            op.snapshot.error = None;
        }
        Err(e) => {
            op.snapshot.status = DeletionStatus::Failed;
            op.snapshot.error = Some(e.to_string());
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
            .or(session.title(&op.snapshot.session_id).await)
            .unwrap_or_else(|| op.snapshot.session_id.clone());
        // Validate the whole set before claiming descendants: overlapping
        // operations fail observably rather than erase or notify twice.
        for id in &op.members {
            let guard = state::state_get(&deps.iii, GUARDS, id, timeout).await?;
            if !guard.is_null() && guard.as_str() != Some(&op.snapshot.operation_id) {
                return Err(failure(format!("overlapping deletion of {id}: {guard}")));
            }
        }
        op.planned = true;
        save(deps, op).await?;
    }
    for id in &op.members {
        claim_guard(deps, id, &op.snapshot.operation_id).await?;
    }
    drop(topology);

    // Signal EVERY member first, even when the selected root is terminal.
    // Never interpret a stop acknowledgement as terminality.
    let mut records = Vec::new();
    for id in &op.members {
        if let Some(record) = state::get_turn(&deps.iii, id, timeout).await? {
            if !record.status.is_terminal() {
                deps.cancels.fire(&record.turn_id);
            }
            records.push(record);
        }
    }
    let mut cancellation_errors = Vec::new();
    for record in &records {
        if !record.status.is_terminal() {
            // Stricter than `harness::stop`: an unconfirmed stop (router abort
            // failure, unknown external pending work) fails the deletion.
            match super::stop::stop_one(
                deps,
                super::stop::StopRequest {
                    session_id: record.session_id.clone(),
                    turn_id: Some(record.turn_id.clone()),
                },
            )
            .await
            {
                Ok(outcome) => cancellation_errors.extend(
                    outcome
                        .unconfirmed
                        .into_iter()
                        .map(|reason| format!("{}: {reason}", record.session_id)),
                ),
                Err(error) => cancellation_errors.push(error.to_string()),
            }
        }
    }
    if !cancellation_errors.is_empty() {
        return Err(failure(cancellation_errors.join("; ")));
    }
    loop {
        // Arm before reading terminal state: covers completion racing the read.
        let notified = deps.deletion_changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let mut terminal = true;
        for id in &op.members {
            let _activity = deps.turn_activity.guard(id).await;
            let _lock = deps.locks.guard(id).await;
            if let Some(record) = state::get_turn(&deps.iii, id, timeout).await? {
                if !record.status.is_terminal() {
                    terminal = false;
                }
                if record
                    .calls
                    .values()
                    .any(|c| c.state == CallState::Triggered)
                {
                    return Err(failure(format!(
                        "{id} has a tool with unknown completion; data retained"
                    )));
                }
            }
        }
        let witnesses =
            state::list_values::<DispatchWitness>(&deps.iii, DISPATCHES, timeout).await?;
        if witnesses.iter().any(|w| op.members.contains(&w.session_id)) {
            terminal = false;
        }
        if terminal {
            break;
        }
        notified.await;
    }
    Ok(())
}

async fn erase(deps: &Deps, op: &mut Operation) -> Result<(), HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
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
                "deletion cleanup deadline expired; remaining data retained",
            ));
        }
        if op.snapshot.deleted_session_ids.contains(&id) {
            continue;
        }
        let _activity = deps.turn_activity.guard(&id).await;
        let _lock = deps.locks.guard(&id).await;
        let _topology = deps.topology.lock().await;
        if let Some(record) = state::get_turn(&deps.iii, &id, timeout).await? {
            if !record.status.is_terminal() {
                return Err(failure(format!("{id} became nonterminal; refusing erase")));
            }
        }
        cleanup(deps, &id).await?;
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
            return Err(failure(format!(
                "session::delete {id} returned deleted={} but the session still exists",
                ack.deleted
            )));
        }
        state::delete_turn(&deps.iii, &id, timeout).await?;
        op.snapshot.deleted_session_ids.push(id);
        save(deps, op).await?;
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
    format!("User-requested cancellation and deletion of session {} ({}) and its subtree: {}. All turns have stopped; these sessions are being deleted. Do not await their results and do not recreate them automatically. Your history, execution and other children are preserved. Operation: {}.",
        op.name, op.snapshot.session_id, op.members.join(", "), op.snapshot.operation_id)
}

async fn notify_parent(deps: &Deps, parent: &str, op: &Operation) -> Result<(), HarnessError> {
    let cfg = deps.cfg().await;
    let topology = deps.topology.lock().await;
    if guard_owner(deps, parent).await?.is_some() || !deps.session().await.exists(parent).await? {
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
        if guard_owner(deps, parent).await?.is_some()
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
}

/// Must be persisted before dispatch; cleared only on a confirmed reply.
pub(crate) async fn begin_dispatch(
    deps: &Deps,
    session_id: &str,
    function_id: &str,
) -> Result<String, HarnessError> {
    let _topology = deps.topology.lock().await;
    ensure_live(deps, session_id).await?;
    let id = uuid::Uuid::new_v4().to_string();
    state::state_set(
        &deps.iii,
        DISPATCHES,
        &id,
        json!(DispatchWitness {
            session_id: session_id.into(),
            function_id: function_id.into(),
        }),
        deps.cfg().await.session_timeout_ms,
    )
    .await?;
    Ok(id)
}

pub(crate) async fn end_dispatch(deps: &Deps, id: &str) -> Result<(), HarnessError> {
    state::state_delete(
        &deps.iii,
        DISPATCHES,
        id,
        deps.cfg().await.session_timeout_ms,
    )
    .await?;
    deps.deletion_changed.notify_waiters();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
                error: None,
            },
            deadline: 0,
            members: vec!["child2".into(), "grandchild1".into()],
            parent: Some("parent".into()),
            name: "Research".into(),
            planned: true,
            notified: false,
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
