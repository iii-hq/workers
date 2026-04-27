//! A2A v0.3 streaming surface.
//!
//! Implements `message/stream` and `tasks/resubscribe` over Server-Sent
//! Events. The iii engine's HTTP trigger ships a writable channel ref in
//! the input payload — writing the SSE preamble plus framed events to that
//! channel emits them downstream as `text/event-stream`.
//!
//! ## Cross-method propagation
//!
//! `StreamRegistry` is a process-local fan-out: each in-flight task has a
//! `TaskBus` of subscribers. `handle_send` (sync) and `handle_cancel` also
//! call into `broadcast` so concurrent stream subscribers see the live
//! transitions instead of needing to poll `tasks/get`.
//!
//! ## CR-validated invariants
//!
//! - Subscribers carry a stable `u64` id minted at subscribe-time. Pruning
//!   by positional `Vec` index would race with concurrent broadcasts that
//!   shift the indices.
//! - `JoinSet` results match all three arms (Ok(Ok), Ok(Err(sub_id)),
//!   Err(JoinError)) so a panicking writer task can't silently leak.
//! - Resubscribe replay reserves an id from the subscriber's own counter
//!   before the first broadcast lands; without that the manual replay
//!   frame and the next live frame would both ship `id: 1`.
//! - After subscribing to an in-flight task, we re-load the task. If the
//!   producer transitioned to terminal between `load_task` and `subscribe`,
//!   we'd never receive the final frame; the race guard emits one and
//!   closes.

use std::collections::HashMap;
use std::sync::Arc;

use iii_sdk::{
    ChannelDirection, ChannelWriter, III, StreamChannelRef, TriggerAction, TriggerRequest, Value,
    extract_channel_refs,
};
use serde_json::json;
use tokio::sync::{Mutex, watch};
use tokio::task::JoinSet;

use crate::handler::{
    is_protocol_loop, iso_now, load_task, msg_id, resolve_function, store_task, text_part,
};
use crate::types::*;

// SSE event names. The A2A spec doesn't pin an `event:` value but using
// the discriminator `kind` keeps the wire shape grep-friendly.
const EVENT_STATUS: &str = "status-update";
const EVENT_ARTIFACT: &str = "artifact-update";

/// Build a single SSE frame: `id: <n>\nevent: <kind>\ndata: <json>\n\n`.
///
/// We pre-serialize `payload` into a single line — the A2A spec uses
/// JSON-encoded payloads, which already escape newlines, so no per-line
/// `data:` splitting is needed.
pub fn build_sse_frame(id: u64, kind: &str, payload: &Value) -> Vec<u8> {
    let body = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string());
    let frame = format!("id: {id}\nevent: {kind}\ndata: {body}\n\n");
    frame.into_bytes()
}

/// Send the two SSE preamble control messages (`set_status` then
/// `set_headers`) so the engine's HTTP trigger flips the response into
/// `text/event-stream` mode before any data frames hit the wire.
async fn send_sse_preamble(writer: &ChannelWriter) -> Result<(), iii_sdk::IIIError> {
    writer
        .send_message(
            &serde_json::to_string(&json!({
                "type": "set_status", "status_code": 200
            }))
            .unwrap(),
        )
        .await?;
    writer
        .send_message(
            &serde_json::to_string(&json!({
                "type": "set_headers",
                "headers": {
                    "content-type": "text/event-stream",
                    "cache-control": "no-cache"
                }
            }))
            .unwrap(),
        )
        .await?;
    Ok(())
}

/// Pull the writable channel ref out of an HTTP trigger input. The iii-sdk
/// contract is one writable channel per HTTP trigger invocation — we take
/// the first match.
pub fn writer_ref_from_input(input: &Value) -> Option<StreamChannelRef> {
    extract_channel_refs(input)
        .into_iter()
        .find(|(_, r)| matches!(r.direction, ChannelDirection::Write))
        .map(|(_, r)| r)
}

struct Subscriber {
    id: u64,
    writer: Arc<ChannelWriter>,
    next_id: u64,
}

struct TaskBus {
    subscribers: Vec<Subscriber>,
    next_subscriber_id: u64,
    terminal_tx: watch::Sender<bool>,
    terminal_rx: watch::Receiver<bool>,
}

impl TaskBus {
    fn new() -> Self {
        let (tx, rx) = watch::channel(false);
        Self {
            subscribers: Vec::new(),
            next_subscriber_id: 1,
            terminal_tx: tx,
            terminal_rx: rx,
        }
    }
}

/// Process-local fan-out registry. `Arc<StreamRegistry>` is shared between
/// the JSON-RPC dispatch closure and the streaming module so a sync
/// `message/send` can broadcast through the same registry that streaming
/// subscribers are listening on.
pub struct StreamRegistry {
    buses: Mutex<HashMap<String, TaskBus>>,
}

impl StreamRegistry {
    pub fn new() -> Self {
        Self {
            buses: Mutex::new(HashMap::new()),
        }
    }

    /// Subscribe a writer to a task. Returns a stable subscriber id that
    /// callers use with `reserve_event_id` and that is logged on prune.
    pub async fn subscribe(&self, task_id: &str, writer: Arc<ChannelWriter>) -> u64 {
        let mut buses = self.buses.lock().await;
        let bus = buses
            .entry(task_id.to_string())
            .or_insert_with(TaskBus::new);
        let id = bus.next_subscriber_id;
        bus.next_subscriber_id += 1;
        bus.subscribers.push(Subscriber {
            id,
            writer,
            next_id: 1,
        });
        id
    }

    /// Reserve the next event id for a specific subscriber. Used by
    /// resubscribe replay so the manually-emitted frame and the first live
    /// broadcast frame don't both serialize as `id: 1`. Returns `None` if
    /// the bus or subscriber has gone away (e.g. after `close_task`).
    pub async fn reserve_event_id(&self, task_id: &str, sub_id: u64) -> Option<u64> {
        let mut buses = self.buses.lock().await;
        let bus = buses.get_mut(task_id)?;
        let sub = bus.subscribers.iter_mut().find(|s| s.id == sub_id)?;
        let id = sub.next_id;
        sub.next_id += 1;
        Some(id)
    }

    /// Fan a frame out to every subscriber of `task_id`. Snapshots the
    /// (sub_id, writer, frame) tuples under the lock, releases the lock,
    /// then dispatches via `JoinSet`. Failures prune by stable id, not
    /// positional index — concurrent broadcasts can mutate the Vec
    /// between snapshot and prune.
    pub async fn broadcast(&self, task_id: &str, kind: &str, payload: &Value) {
        let snapshots: Vec<(u64, Arc<ChannelWriter>, Vec<u8>)> = {
            let mut buses = self.buses.lock().await;
            let Some(bus) = buses.get_mut(task_id) else {
                return;
            };
            bus.subscribers
                .iter_mut()
                .map(|s| {
                    let id = s.next_id;
                    s.next_id += 1;
                    let frame = build_sse_frame(id, kind, payload);
                    (s.id, s.writer.clone(), frame)
                })
                .collect()
        };

        if snapshots.is_empty() {
            return;
        }

        let mut set: JoinSet<Result<(), u64>> = JoinSet::new();
        for (sub_id, writer, frame) in snapshots {
            set.spawn(async move {
                if writer.write(&frame).await.is_err() {
                    Err(sub_id)
                } else {
                    Ok(())
                }
            });
        }

        let mut to_prune: Vec<u64> = Vec::new();
        while let Some(res) = set.join_next().await {
            match res {
                Ok(Ok(())) => {}
                Ok(Err(sub_id)) => to_prune.push(sub_id),
                Err(join_err) => {
                    tracing::warn!(error = %join_err, task_id = %task_id, "broadcast task panicked");
                }
            }
        }

        if !to_prune.is_empty() {
            let mut buses = self.buses.lock().await;
            if let Some(bus) = buses.get_mut(task_id) {
                bus.subscribers.retain(|s| !to_prune.contains(&s.id));
            }
        }
    }

    /// Mark a task terminal: drop the bus and signal the terminal watch so
    /// resubscribe waiters wake up and tear down their writer.
    pub async fn close_task(&self, task_id: &str) {
        let mut buses = self.buses.lock().await;
        if let Some(bus) = buses.remove(task_id) {
            let _ = bus.terminal_tx.send(true);
        }
    }

    /// Hand out a `watch::Receiver` so callers can `.changed().await` for
    /// the terminal signal without holding the registry lock.
    pub async fn terminal_watch(&self, task_id: &str) -> Option<watch::Receiver<bool>> {
        let buses = self.buses.lock().await;
        buses.get(task_id).map(|b| b.terminal_rx.clone())
    }

    pub async fn subscriber_count(&self, task_id: &str) -> usize {
        let buses = self.buses.lock().await;
        buses.get(task_id).map(|b| b.subscribers.len()).unwrap_or(0)
    }

    pub async fn has_task(&self, task_id: &str) -> bool {
        let buses = self.buses.lock().await;
        buses.contains_key(task_id)
    }
}

impl Default for StreamRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a `TaskStatusUpdateEvent` payload as a `serde_json::Value`. We
/// emit it as Value rather than the typed struct so the broadcast path
/// stays uniform across status and artifact frames.
fn status_event_payload(task: &Task, state: TaskState, final_event: bool) -> Value {
    let event = TaskStatusUpdateEvent {
        task_id: task.id.clone(),
        context_id: task.context_id.clone(),
        kind: EVENT_STATUS.to_string(),
        status: TaskStatus {
            state,
            message: None,
            timestamp: Some(iso_now()),
        },
        final_event,
    };
    serde_json::to_value(event).unwrap_or(Value::Null)
}

fn artifact_event_payload(task_id: &str, context_id: Option<&str>, artifact: Artifact) -> Value {
    let event = TaskArtifactUpdateEvent {
        task_id: task_id.to_string(),
        context_id: context_id.map(|s| s.to_string()),
        kind: EVENT_ARTIFACT.to_string(),
        artifact,
        append: None,
        last_chunk: Some(true),
    };
    serde_json::to_value(event).unwrap_or(Value::Null)
}

/// Send the SSE preamble + a single SSE frame via this writer (used by
/// resubscribe replay where there's exactly one subscriber: the caller).
async fn write_one_frame(
    writer: &ChannelWriter,
    id: u64,
    kind: &str,
    payload: &Value,
) -> Result<(), iii_sdk::IIIError> {
    let frame = build_sse_frame(id, kind, payload);
    writer.write(&frame).await
}

/// `message/stream`: kick off a task and stream every state transition +
/// the final artifact frame to this caller's writer.
pub async fn handle_stream(
    iii: &III,
    params: Option<Value>,
    writer_ref: StreamChannelRef,
    registry: Arc<StreamRegistry>,
) {
    let writer = Arc::new(ChannelWriter::new(iii.address(), &writer_ref));

    if let Err(e) = send_sse_preamble(&writer).await {
        tracing::warn!(error = %e, "failed to send SSE preamble");
        let _ = writer.close().await;
        return;
    }

    let params: SendMessageParams = match params {
        Some(p) => match serde_json::from_value(p) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "message/stream: invalid params");
                let _ = writer.close().await;
                return;
            }
        },
        None => {
            tracing::warn!("message/stream: missing params");
            let _ = writer.close().await;
            return;
        }
    };

    let task_id = params
        .message
        .task_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let context_id = params.message.context_id.clone();

    let mut task = if let Some(existing) = load_task(iii, &task_id).await {
        if matches!(
            existing.status.state,
            TaskState::Completed | TaskState::Canceled | TaskState::Failed | TaskState::Rejected
        ) {
            // Already terminal — emit one final frame and exit. Reuse the
            // stored state so callers see the historical outcome.
            let payload = status_event_payload(&existing, existing.status.state.clone(), true);
            let _ = write_one_frame(&writer, 1, EVENT_STATUS, &payload).await;
            let _ = writer.close().await;
            return;
        }
        let mut t = existing;
        if let Some(ref mut history) = t.history {
            history.push(params.message.clone());
        }
        t.status = TaskStatus {
            state: TaskState::Working,
            message: Some(Message {
                message_id: msg_id(),
                role: MessageRole::Agent,
                parts: vec![text_part("Processing...")],
                task_id: None,
                context_id: None,
                metadata: None,
            }),
            timestamp: Some(iso_now()),
        };
        t
    } else {
        Task {
            id: task_id.clone(),
            context_id: context_id.clone(),
            status: TaskStatus {
                state: TaskState::Working,
                message: Some(Message {
                    message_id: msg_id(),
                    role: MessageRole::Agent,
                    parts: vec![text_part("Processing...")],
                    task_id: None,
                    context_id: None,
                    metadata: None,
                }),
                timestamp: Some(iso_now()),
            },
            artifacts: None,
            history: Some(vec![params.message.clone()]),
            metadata: params.metadata.clone(),
        }
    };
    store_task(iii, &task).await;

    // Subscribe BEFORE the first broadcast so this writer receives every
    // frame we emit below.
    let _sub_id = registry.subscribe(&task_id, writer.clone()).await;

    // Wire-only `Submitted` frame for spec compliance. We never persist
    // the Submitted state because the engine has already moved through
    // it (store_task above wrote Working) — emitting it here just gives
    // A2A clients the canonical state sequence they expect.
    let submitted_task = Task {
        status: TaskStatus {
            state: TaskState::Submitted,
            message: None,
            timestamp: Some(iso_now()),
        },
        ..task.clone()
    };
    let submitted_payload = status_event_payload(&submitted_task, TaskState::Submitted, false);
    registry
        .broadcast(&task_id, EVENT_STATUS, &submitted_payload)
        .await;

    // Working frame mirrors the persisted state.
    let working_payload = status_event_payload(&task, TaskState::Working, false);
    registry
        .broadcast(&task_id, EVENT_STATUS, &working_payload)
        .await;

    let (function_id, payload) = resolve_function(&params.message);
    if function_id.is_empty() {
        task.status = TaskStatus {
            state: TaskState::Failed,
            message: Some(Message {
                message_id: msg_id(),
                role: MessageRole::Agent,
                parts: vec![text_part(
                    "No function_id found. Send a data part with {\"function_id\": \"...\", \"payload\": {...}} or use :: notation in text.",
                )],
                task_id: None,
                context_id: None,
                metadata: None,
            }),
            timestamp: Some(iso_now()),
        };
        store_task(iii, &task).await;
        let payload = status_event_payload(&task, TaskState::Failed, true);
        registry.broadcast(&task_id, EVENT_STATUS, &payload).await;
        registry.close_task(&task_id).await;
        let _ = writer.close().await;
        return;
    }
    let fn_name = function_id.clone();

    if is_protocol_loop(&function_id) {
        let reason = format!(
            "Function '{}' is a protocol entry point, not a callable tool",
            function_id
        );
        task.status = TaskStatus {
            state: TaskState::Failed,
            message: Some(Message {
                message_id: msg_id(),
                role: MessageRole::Agent,
                parts: vec![text_part(reason)],
                task_id: None,
                context_id: None,
                metadata: None,
            }),
            timestamp: Some(iso_now()),
        };
        store_task(iii, &task).await;
        let payload = status_event_payload(&task, TaskState::Failed, true);
        registry.broadcast(&task_id, EVENT_STATUS, &payload).await;
        registry.close_task(&task_id).await;
        let _ = writer.close().await;
        return;
    }

    // Spawn the actual function call. Runs concurrently with any further
    // broadcasts (e.g. from a sibling `tasks/cancel`).
    let iii_run = iii.clone();
    let registry_run = registry.clone();
    let task_id_run = task_id.clone();
    let context_id_run = task.context_id.clone();
    let metadata_run = task.metadata.clone();
    let history_run = task.history.clone();
    // Drop-guard so a panic anywhere inside the spawned future still
    // unwinds through close_task. Without this, a panic leaves the bus
    // alive in `buses` with `terminal_tx` un-fired, and the parent loop
    // below blocks indefinitely on `rx.changed()` — SSE socket stays open
    // forever. Same coverage as a `defer`/RAII pattern: explicit early
    // returns (e.g. the cancel branch below) also fire it on the way out.
    struct CloseGuard {
        registry: Arc<StreamRegistry>,
        task_id: String,
        armed: bool,
    }
    impl CloseGuard {
        fn disarm(&mut self) {
            self.armed = false;
        }
    }
    impl Drop for CloseGuard {
        fn drop(&mut self) {
            if self.armed {
                let registry = self.registry.clone();
                let task_id = self.task_id.clone();
                tokio::spawn(async move {
                    registry.close_task(&task_id).await;
                });
            }
        }
    }

    tokio::spawn(async move {
        let mut guard = CloseGuard {
            registry: registry_run.clone(),
            task_id: task_id_run.clone(),
            armed: true,
        };

        let result = iii_run
            .trigger(TriggerRequest {
                function_id,
                payload,
                action: None,
                timeout_ms: Some(30000),
            })
            .await;

        // Cancel-while-running: if the task is now Canceled, don't
        // overwrite that with Completed. The cancel path has already
        // broadcast its own final frame and called close_task; nothing more
        // to do here. NOTE: this is a best-effort TOCTOU check — iii engine
        // state has no atomic CAS, so a cancel landing AFTER this load but
        // BEFORE the store_task below would still get clobbered. Accepted
        // race per repo convention; consider it eventual consistency.
        let fresh = load_task(&iii_run, &task_id_run).await;
        if let Some(ref t) = fresh
            && matches!(t.status.state, TaskState::Canceled)
        {
            // Cancel path already called close_task — disarm so we don't
            // double-close.
            guard.disarm();
            return;
        }

        match result {
            Ok(value) => {
                let result_text =
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
                let artifact = Artifact {
                    artifact_id: uuid::Uuid::new_v4().to_string(),
                    parts: vec![Part {
                        text: Some(result_text),
                        data: None,
                        url: None,
                        raw: None,
                        media_type: Some("application/json".to_string()),
                    }],
                    name: Some(fn_name),
                    metadata: None,
                };

                let artifact_payload = artifact_event_payload(
                    &task_id_run,
                    context_id_run.as_deref(),
                    artifact.clone(),
                );
                registry_run
                    .broadcast(&task_id_run, EVENT_ARTIFACT, &artifact_payload)
                    .await;

                let completed = Task {
                    id: task_id_run.clone(),
                    context_id: context_id_run.clone(),
                    status: TaskStatus {
                        state: TaskState::Completed,
                        message: None,
                        timestamp: Some(iso_now()),
                    },
                    artifacts: Some(vec![artifact]),
                    history: history_run.clone(),
                    metadata: metadata_run.clone(),
                };
                store_task(&iii_run, &completed).await;
                let final_payload = status_event_payload(&completed, TaskState::Completed, true);
                registry_run
                    .broadcast(&task_id_run, EVENT_STATUS, &final_payload)
                    .await;
            }
            Err(err) => {
                let failed = Task {
                    id: task_id_run.clone(),
                    context_id: context_id_run.clone(),
                    status: TaskStatus {
                        state: TaskState::Failed,
                        message: Some(Message {
                            message_id: msg_id(),
                            role: MessageRole::Agent,
                            parts: vec![text_part(format!("Error: {}", err))],
                            task_id: None,
                            context_id: None,
                            metadata: None,
                        }),
                        timestamp: Some(iso_now()),
                    },
                    artifacts: None,
                    history: history_run.clone(),
                    metadata: metadata_run.clone(),
                };
                store_task(&iii_run, &failed).await;
                let final_payload = status_event_payload(&failed, TaskState::Failed, true);
                registry_run
                    .broadcast(&task_id_run, EVENT_STATUS, &final_payload)
                    .await;
            }
        }

        // Happy path completed normally — disarm the guard before letting
        // it drop, otherwise we'd close twice. Drop fires the guard's
        // `close_task` only when `armed == true`.
        guard.disarm();
        registry_run.close_task(&task_id_run).await;
    });

    // Wait for the producer to signal terminal, then close our writer so
    // the SSE response completes cleanly.
    if let Some(mut rx) = registry.terminal_watch(&task_id).await {
        loop {
            if *rx.borrow() {
                break;
            }
            if rx.changed().await.is_err() {
                break;
            }
        }
    }
    let _ = writer.close().await;
}

/// `tasks/resubscribe`: latch onto an in-flight task and replay the
/// current snapshot, then forward subsequent broadcasts until terminal.
pub async fn handle_resubscribe(
    iii: &III,
    params: Option<Value>,
    writer_ref: StreamChannelRef,
    registry: Arc<StreamRegistry>,
) {
    let writer = Arc::new(ChannelWriter::new(iii.address(), &writer_ref));

    if let Err(e) = send_sse_preamble(&writer).await {
        tracing::warn!(error = %e, "failed to send SSE preamble");
        let _ = writer.close().await;
        return;
    }

    let params: ResubscribeParams = match params {
        Some(p) => match serde_json::from_value(p) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "tasks/resubscribe: invalid params");
                let _ = writer.close().await;
                return;
            }
        },
        None => {
            tracing::warn!("tasks/resubscribe: missing params");
            let _ = writer.close().await;
            return;
        }
    };

    let task_id = params.id.clone();

    let task = match load_task(iii, &task_id).await {
        Some(t) => t,
        None => {
            // Synthesize a Failed final frame so the client sees a clean
            // termination instead of a hung connection.
            let synthetic = Task {
                id: task_id.clone(),
                context_id: None,
                status: TaskStatus {
                    state: TaskState::Failed,
                    message: Some(Message {
                        message_id: msg_id(),
                        role: MessageRole::Agent,
                        parts: vec![text_part(format!("Task not found: {}", task_id))],
                        task_id: None,
                        context_id: None,
                        metadata: None,
                    }),
                    timestamp: Some(iso_now()),
                },
                artifacts: None,
                history: None,
                metadata: None,
            };
            let payload = status_event_payload(&synthetic, TaskState::Failed, true);
            let _ = write_one_frame(&writer, 1, EVENT_STATUS, &payload).await;
            let _ = writer.close().await;
            return;
        }
    };

    // Already terminal — emit the stored state as a final frame and exit.
    if matches!(
        task.status.state,
        TaskState::Completed | TaskState::Canceled | TaskState::Failed | TaskState::Rejected
    ) {
        let payload = status_event_payload(&task, task.status.state.clone(), true);
        let _ = write_one_frame(&writer, 1, EVENT_STATUS, &payload).await;
        let _ = writer.close().await;
        return;
    }

    // Subscribe first so any frame the producer emits next reaches us.
    let sub_id = registry.subscribe(&task_id, writer.clone()).await;

    // Reserve our replay frame's id from this subscriber's own counter so
    // the next broadcast that lands doesn't collide.
    let replay_id = registry
        .reserve_event_id(&task_id, sub_id)
        .await
        .unwrap_or(1);
    let replay_payload = status_event_payload(&task, task.status.state.clone(), false);
    if let Err(e) = write_one_frame(&writer, replay_id, EVENT_STATUS, &replay_payload).await {
        tracing::warn!(error = %e, "tasks/resubscribe: replay frame write failed");
        let _ = writer.close().await;
        return;
    }

    // Race guard: the producer may have transitioned to terminal between
    // our `load_task` above and our `subscribe`. If so, the close_task
    // signal already fired and our subscriber will never receive a final
    // frame. Re-load and synthesize one if needed.
    if let Some(refreshed) = load_task(iii, &task_id).await
        && matches!(
            refreshed.status.state,
            TaskState::Completed | TaskState::Canceled | TaskState::Failed | TaskState::Rejected
        )
    {
        if let Some(final_id) = registry.reserve_event_id(&task_id, sub_id).await {
            let payload = status_event_payload(&refreshed, refreshed.status.state.clone(), true);
            let _ = write_one_frame(&writer, final_id, EVENT_STATUS, &payload).await;
        }
        registry.close_task(&task_id).await;
        let _ = writer.close().await;
        return;
    }

    // Wait for terminal.
    if let Some(mut rx) = registry.terminal_watch(&task_id).await {
        loop {
            if *rx.borrow() {
                break;
            }
            if rx.changed().await.is_err() {
                break;
            }
        }
    }
    let _ = writer.close().await;
}

/// Public entry called by the JSON-RPC dispatch closure when it sees
/// `message/stream` (alias `SendStreamingMessage`).
pub async fn dispatch_stream(
    iii: &III,
    params: Option<Value>,
    writer_ref: StreamChannelRef,
    registry: Arc<StreamRegistry>,
) {
    handle_stream(iii, params, writer_ref, registry).await;
}

/// Public entry called by the JSON-RPC dispatch closure when it sees
/// `tasks/resubscribe` (alias `SubscribeToTask`).
pub async fn dispatch_resubscribe(
    iii: &III,
    params: Option<Value>,
    writer_ref: StreamChannelRef,
    registry: Arc<StreamRegistry>,
) {
    handle_resubscribe(iii, params, writer_ref, registry).await;
}

/// Cross-method propagation helper: broadcast a `TaskStatusUpdateEvent`
/// for the given task. Used by sync `message/send` and `tasks/cancel` to
/// keep concurrent stream subscribers in sync.
pub async fn broadcast_status(registry: &StreamRegistry, task: &Task, final_event: bool) {
    let payload = status_event_payload(task, task.status.state.clone(), final_event);
    registry.broadcast(&task.id, EVENT_STATUS, &payload).await;
}

/// Cross-method propagation helper: broadcast a `TaskArtifactUpdateEvent`.
pub async fn broadcast_artifact(registry: &StreamRegistry, task: &Task, artifact: &Artifact) {
    let payload = artifact_event_payload(&task.id, task.context_id.as_deref(), artifact.clone());
    registry.broadcast(&task.id, EVENT_ARTIFACT, &payload).await;
}

// Unused-warning silencer for the trigger helper; kept for symmetry with
// future store-side uses.
#[allow(dead_code)]
async fn touch_task(iii: &III, task: &Task) {
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "state::set".to_string(),
            payload: json!({ "scope": "a2a:tasks", "key": task.id, "data": task }),
            action: Some(TriggerAction::Void),
            timeout_ms: None,
        })
        .await;
}
