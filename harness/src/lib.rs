//! Harness meta-worker. Composes the modular workers via `iii.worker.yaml`
//! runtime dependencies; this lib registers `harness::status` plus a
//! browser-facing `bridge::trigger` HTTP route that forwards arbitrary
//! `{function_id, payload}` calls onto the iii bus.
//!
//! ## SSE event-tail API discovery (T11)
//!
//! iii-sdk 0.11.3 exposes NO blocking/tailing read API for streams. The only
//! built-in stream functions invokable via `iii.trigger(...)` are:
//!
//! - `stream::set`     payload `{stream_name, group_id, item_id, data}`
//!   -> `{old_value, new_value}` (one-shot, returns immediately)
//! - `stream::update`  payload `{stream_name, group_id, item_id, ops: [UpdateOp]}`
//! - `stream::delete`  payload `{stream_name, group_id, item_id}`
//! - `stream::get`     payload `{stream_name, group_id, item_id}`
//!   -> the item's `data` value (or `null` if absent). Single item only.
//! - `stream::list`    payload `{stream_name, group_id}`
//!   -> JSON array of all items' `data` in that group.
//!   One-shot snapshot — does NOT block, does NOT return
//!   item_ids, has NO `since_id`/`max_items`/`block_ms` params,
//!   and has NO `next_cursor` in the response.
//!
//! There is no `stream::tail`, `stream::range`, `stream::read`, or `stream::since`,
//! and no `subscribe_stream` / `consume_stream` / `tail_stream` method on `III`.
//! Verified by inspection of `~/.cargo/registry/src/.../iii-sdk-0.11.3/src/`
//! (`builtin_triggers.rs`, `iii.rs`, `stream.rs`, `lib.rs`) plus
//! `tests/stream.rs`, which only exercises set/get/delete/list.
//!
//! Live-event consumption is pull-via-push: register a `stream` trigger
//! (`builtin_triggers::IIITrigger::Stream` with `StreamTriggerConfig` filtered
//! by `stream_name = "agent::events"` and optional `group_id = <session_id>`).
//! The engine then invokes the bound function with a `StreamCallRequest`
//! `{type: "stream", timestamp, streamName, groupId, id, event: {type, data}}`
//! every time a peer calls `stream::set`/`update`/`delete`. There is no
//! cursor field in the response from any read API; the only durable
//! sequence available is the `item_id` we mint ourselves in
//! `turn-orchestrator/src/events.rs::format_item_id` (`<sid>-<seq:08>`).
//!
//! T12's SSE pump must therefore:
//! 1. On `GET /bridge/events?session_id=<sid>`, register a `stream` trigger
//!    bound to a per-request handler that forwards `StreamCallRequest`
//!    frames into a per-session `tokio::sync::broadcast::Sender<AgentEvent>`
//!    (or build it once at boot and fan-out by `group_id`). Unregister the
//!    trigger when the response body is dropped.
//! 2. To honor `Last-Event-ID: <sid>-NNNNNNNN`, seed the SSE response by
//!    calling `stream::list` once, sorting by `item_id`, and replaying only
//!    items whose `item_id` sorts strictly greater than `Last-Event-ID`
//!    BEFORE attaching the broadcast receiver. (Items inserted between the
//!    list snapshot and the trigger registration must be deduped by
//!    `item_id` on the receiving end — keep a small "seen" set.)
//! 3. Use the `item_id` as the SSE `id:` field so browsers reconnect with
//!    the right `Last-Event-ID`. The stream trigger payload itself does not
//!    carry `item_id` under that key — it arrives as the top-level `id`
//!    field of `StreamCallRequest` (Option<String>).

pub mod fanout;
pub mod sse;

use std::sync::Arc;

use iii_sdk::{
    FunctionRef, IIIError, RegisterFunctionMessage, RegisterTriggerInput, TriggerRequest, Value,
    III,
};
use serde_json::json;

/// Upper bound for a single bridge::trigger call. Tool-calling turns roundtrip
/// the LLM 2+ times plus filesystem ops, so the engine's 30s default 504s
/// before turn-orchestrator finishes. 4 minutes covers the worst case we've
/// seen with Opus + a few tool calls.
const BRIDGE_TIMEOUT_MS: u64 = 240_000;

// Note: `iii-worker-manager` is intentionally NOT listed here. It is provided
// by the iii engine itself (built-in default in the engine's
// `iii-worker/src/cli/builtin_defaults.rs`), not a discrete worker crate the
// harness can depend on. The browser SDK README's `iii worker add
// iii-worker-manager` instruction toggles that built-in feature, not a
// separate dependency. So Phase B step A keeps EXPECTED_WORKERS unchanged.
pub const EXPECTED_WORKERS: &[&str] = &[
    "turn-orchestrator",
    "provider-router",
    "session-tree",
    "session-inbox",
    "models-catalog",
    "hook-fanout",
    "policy-denylist",
    "shell-bash",
    "shell-filesystem",
    "subagent",
    "provider-anthropic",
    "provider-openai",
    "auth-credentials",
    "llm-budget",
    "skills",
    "approval-gate",
];

/// Build the payload sent to skills::register at boot. Pure helper so the
/// shape is testable without a live engine.
pub fn build_skills_register_payload() -> serde_json::Value {
    serde_json::json!({
        "id": "harness",
        "skill_version": env!("CARGO_PKG_VERSION"),
        "min_console_version": "0.1.0",
        "body": "Harness meta-worker. Composes the modular workers that back the iii chat surface.",
        "expected_workers": EXPECTED_WORKERS,
    })
}

pub struct HarnessFunctionRefs {
    pub status: FunctionRef,
    pub bridge: FunctionRef,
    pub events: FunctionRef,
    pub bridge_info: FunctionRef,
    pub subscribe_fn: FunctionRef,
    pub unsubscribe_fn: FunctionRef,
    pub fanout_pumps: Option<fanout::FanoutPumps>,
}

impl HarnessFunctionRefs {
    pub fn unregister_all(self) {
        self.status.unregister();
        self.bridge.unregister();
        self.events.unregister();
        self.bridge_info.unregister();
        self.subscribe_fn.unregister();
        self.unsubscribe_fn.unregister();
        if let Some(p) = self.fanout_pumps {
            p.shutdown();
        }
    }
}

/// Default engine URL used by `bridge::info` when no override is provided.
/// Matches `harness/src/config.rs::default_engine_url`.
const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

/// Register harness functions with iii.
///
/// Uses [`DEFAULT_ENGINE_URL`] for the `engine_url` field of `bridge::info`.
/// Production callers should prefer [`register_with_iii_with_engine_url`] so
/// the URL reflects the actual engine the harness is connected to.
pub async fn register_with_iii(iii: &III) -> anyhow::Result<HarnessFunctionRefs> {
    register_with_iii_with_engine_url(iii, DEFAULT_ENGINE_URL).await
}

#[allow(clippy::too_many_lines)]
pub async fn register_with_iii_with_engine_url(
    iii: &III,
    engine_url: &str,
) -> anyhow::Result<HarnessFunctionRefs> {
    let status = iii.register_function((
        RegisterFunctionMessage::with_id("harness::status".into()).with_description(
            "Returns the harness bundle name, version, and the list of expected runtime workers."
                .into(),
        ),
        |_payload: Value| async move {
            Ok::<_, IIIError>(json!({
                "ok": true,
                "name": env!("CARGO_PKG_NAME"),
                "version": env!("CARGO_PKG_VERSION"),
                "expected_workers": EXPECTED_WORKERS,
            }))
        },
    ));

    let iii_for_bridge = iii.clone();
    let bridge = iii.register_function((
        RegisterFunctionMessage::with_id("bridge::trigger".into()).with_description(
            "Forward {function_id, payload} to iii.trigger and return the result. \
             Used by harness/web/ to reach the bus over HTTP."
                .into(),
        ),
        move |input: Value| {
            let iii = iii_for_bridge.clone();
            async move {
                // HTTP trigger wraps the request body as { body, query_params, headers, ... }.
                // Direct bus callers send { function_id, payload } at the top level.
                let body = input.get("body").cloned().unwrap_or(input);
                let function_id = body
                    .get("function_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| IIIError::Handler("missing function_id".into()))?
                    .to_string();
                let inner = body.get("payload").cloned().unwrap_or_else(|| json!({}));
                let result = iii
                    .trigger(TriggerRequest {
                        function_id,
                        payload: inner,
                        action: None,
                        timeout_ms: Some(BRIDGE_TIMEOUT_MS),
                    })
                    .await
                    .map_err(|e| IIIError::Handler(e.to_string()))?;
                Ok::<_, IIIError>(json!({
                    "status_code": 200,
                    "headers": { "content-type": "application/json" },
                    "body": result,
                }))
            }
        },
    ));

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".into(),
        function_id: "bridge::trigger".into(),
        config: json!({
            "api_path": "bridge/trigger",
            "http_method": "POST",
            "timeout_ms": BRIDGE_TIMEOUT_MS,
        }),
        metadata: None,
    })
    .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let iii_for_events = iii.clone();
    let events_fn = iii.register_function((
        RegisterFunctionMessage::with_id("bridge::events".into()).with_description(
            "Tail agent::events/<session_id> as Server-Sent Events. Used by harness/web/.".into(),
        ),
        move |input: Value| {
            let iii = iii_for_events.clone();
            async move {
                let body = input.get("body").cloned().unwrap_or_else(|| input.clone());
                let query = input
                    .get("query_params")
                    .cloned()
                    .or_else(|| body.get("query_params").cloned())
                    .unwrap_or_else(|| json!({}));
                let session_id = query
                    .get("session_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| IIIError::Handler("missing session_id".into()))?
                    .to_string();
                let last_event_id = input
                    .get("headers")
                    .and_then(|h| h.get("Last-Event-ID").or_else(|| h.get("last-event-id")))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        query
                            .get("since")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    });

                let body = pump_events(&iii, &session_id, last_event_id).await;
                Ok::<_, IIIError>(json!({
                    "status_code": 200,
                    "headers": {
                        "content-type": "text/event-stream",
                        "cache-control": "no-cache",
                        "connection": "keep-alive",
                    },
                    "body": body,
                }))
            }
        },
    ));

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".into(),
        function_id: "bridge::events".into(),
        config: json!({
            "api_path": "bridge/events",
            "http_method": "GET",
            "timeout_ms": 0u64,
            "stream_response": true,
        }),
        metadata: None,
    })
    .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    // bridge::info — relative WS path so reverse-proxy / HTTPS deployments
    // compose the full URL from `window.location`. `engine_url` is the direct
    // ws:// URL for callers (tests, native clients) that bypass the reverse
    // proxy.
    let engine_url_owned = engine_url.to_string();
    let bridge_info = iii.register_function((
        RegisterFunctionMessage::with_id("bridge::info".into()).with_description(
            "Returns the relative WebSocket path and engine URL for browser clients.".into(),
        ),
        move |_payload: Value| {
            let engine_url = engine_url_owned.clone();
            async move {
                Ok::<_, IIIError>(json!({
                    "ws_path": "/iii/ws",
                    "protocol": "ws",
                    "engine_url": engine_url,
                }))
            }
        },
    ));

    // Per-browser subscription registry. Skeleton: future steps will use this
    // to drive WS push of agent::events / state diffs / cost / approvals.
    let fanout = fanout::new_shared();

    let fanout_for_subscribe = Arc::clone(&fanout);
    let subscribe_fn = iii.register_function((
        RegisterFunctionMessage::with_id("ui::subscribe".into()).with_description(
            "Register a browser's interest in a session (or all sessions if session_id is null)."
                .into(),
        ),
        move |input: Value| {
            let fanout = Arc::clone(&fanout_for_subscribe);
            async move {
                let body = input.get("body").cloned().unwrap_or(input);
                let browser_id = body
                    .get("browser_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| IIIError::Handler("missing browser_id".into()))?
                    .to_string();
                let session_id = body
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let total = {
                    let mut state = fanout.write().await;
                    state.subscribe(browser_id, session_id);
                    state.browser_count()
                };
                Ok::<_, IIIError>(json!({
                    "ok": true,
                    "total_browsers": total,
                }))
            }
        },
    ));

    let fanout_for_unsubscribe = Arc::clone(&fanout);
    let unsubscribe_fn = iii.register_function((
        RegisterFunctionMessage::with_id("ui::unsubscribe".into()).with_description(
            "Remove a browser's subscription to a session (or its all-sessions sub if session_id is null)."
                .into(),
        ),
        move |input: Value| {
            let fanout = Arc::clone(&fanout_for_unsubscribe);
            async move {
                let body = input.get("body").cloned().unwrap_or(input);
                let browser_id = body
                    .get("browser_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| IIIError::Handler("missing browser_id".into()))?
                    .to_string();
                let session_id = body
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let total = {
                    let mut state = fanout.write().await;
                    state.unsubscribe(&browser_id, session_id);
                    state.browser_count()
                };
                Ok::<_, IIIError>(json!({
                    "ok": true,
                    "total_browsers": total,
                }))
            }
        },
    ));

    // Best-effort: a missing `skills` worker shouldn't stop harness from booting.
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "skills::register".into(),
            payload: build_skills_register_payload(),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await;

    // Wire the upstream fanout pumps:
    //   - agent::events stream subscriber → ui::session::event::<browser_id>
    //   - state::list poll                → ui::sessions::changed::<browser_id>
    //
    // These spawn long-lived tasks; the returned handle ends them on
    // `unregister_all`. The `iii` clone here is fine — `III` is internally
    // ref-counted (it's already an `Arc<...>` inside the SDK).
    let fanout_pumps = fanout::spawn_subscribers(&Arc::new(iii.clone()), Arc::clone(&fanout));

    Ok(HarnessFunctionRefs {
        status,
        bridge,
        events: events_fn,
        bridge_info,
        subscribe_fn,
        unsubscribe_fn,
        fanout_pumps: Some(fanout_pumps),
    })
}

/// Build the SSE response body for a `GET /bridge/events?session_id=<sid>`
/// request. Seeds from `stream::list` (sorted by item_id, filtered by `since`)
/// and appends a single keepalive comment.
///
/// TODO(T12-followup): wire live tail. Register a `stream` trigger filtered by
/// `{stream_name: "agent::events", group_id: session_id}` whose handler pushes
/// each `StreamCallRequest` into a per-session
/// `tokio::sync::broadcast::Sender<Value>`. Forward broadcast frames after the
/// seed snapshot, deduping by id against `seen`. Unregister the trigger when
/// the response body is dropped.
async fn pump_events(iii: &III, session_id: &str, since: Option<String>) -> String {
    use crate::sse::{format_frame, heartbeat};
    use std::collections::HashSet;

    let mut out = String::new();
    let mut seen: HashSet<String> = HashSet::new();

    // Seed from stream::list, sorted, filtered by `since` cursor.
    let snapshot = iii
        .trigger(TriggerRequest {
            function_id: "stream::list".into(),
            payload: json!({
                "stream_name": "agent::events",
                "group_id": session_id,
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .ok();
    // `stream::list` returns the raw `data` values in emit order with NO
    // `item_id` per item (T11 discovery). Synthesise positional ids so the
    // browser's `Last-Event-ID` cursor still works — zero-padded so the
    // lexicographic compare matches numeric order.
    if let Some(items) = snapshot.as_ref().and_then(|v| v.as_array()) {
        for (idx, item) in items.iter().enumerate() {
            let id = format!("{session_id}-{idx:08}");
            if let Some(s) = since.as_deref() {
                if id.as_str() <= s {
                    seen.insert(id);
                    continue;
                }
            }
            let data = item.get("data").cloned().unwrap_or_else(|| item.clone());
            out.push_str(&format_frame(&id, &data));
            seen.insert(id);
        }
    }

    // For now, return the seed snapshot plus one heartbeat. Live broadcast
    // wiring is the T12-followup TODO documented above this function.
    let _ = seen;
    out.push_str(heartbeat());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_workers_is_unique_and_non_empty() {
        assert!(!EXPECTED_WORKERS.is_empty());
        let mut seen = std::collections::HashSet::new();
        for w in EXPECTED_WORKERS {
            assert!(seen.insert(*w), "duplicate worker in EXPECTED_WORKERS: {w}");
        }
    }

    #[test]
    fn expected_workers_includes_approval_gate() {
        assert!(
            EXPECTED_WORKERS.contains(&"approval-gate"),
            "approval-gate is required for approval_required to actually block (Phase A item #3)"
        );
    }
}
