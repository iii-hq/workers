//! Harness-runtime registers four agent::* function ids on the iii bus:
//!
//! | Function                | Purpose                                      |
//! |-------------------------|----------------------------------------------|
//! | `agent::stream_assistant`| Provider router. Calls `provider::<name>::complete` (with optional `router::decide` indirection when llm-router is on the bus). |
//! | `agent::abort`           | Set the abort flag for a session via `flag::set`. |
//! | `agent::push_steering`   | Push messages onto the session's steering queue via `queue::push`. |
//! | `agent::push_followup`   | Push messages onto the session's follow-up queue via `queue::push`. |
//!
//! Plus the `hook-fanout`, `durable-queue`, `state-flag` primitives and the
//! `shell-filesystem`, `shell-bash`, `shell-subagent` shell crates.
//!
//! `agent::run_loop` and the seven helper ids that backed the in-process
//! state machine were removed in P7. Migrate callers to `run::start` /
//! `run::start_and_wait` (owned by `turn-orchestrator`).

use std::sync::Arc;

use harness_types::{AgentMessage, AssistantMessage, ErrorKind, StopReason};
use iii_sdk::{
    IIIError, RegisterFunctionMessage, RegisterTriggerInput, TriggerRequest, Value, III,
};
use serde_json::json;

/// Stream name for agent events.
pub const EVENTS_STREAM: &str = "agent::events";

/// State scope shared by all session keys.
pub const STATE_SCOPE: &str = "agent";

/// Hook topic ids.
pub const TOPIC_BEFORE: &str = "agent::before_tool_call";
pub const TOPIC_AFTER: &str = "agent::after_tool_call";

/// Build the payload accepted by `flag::is_set` / `flag::set`.
fn build_flag_payload(name: &str, session_id: &str) -> Value {
    json!({ "name": name, "session_id": session_id })
}

async fn list_function_infos(iii: &III) -> Result<Vec<iii_sdk::FunctionInfo>, String> {
    let value = iii
        .trigger(TriggerRequest {
            function_id: "engine::functions::list".to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: None,
        })
        .await
        .map_err(|e| e.to_string())?;
    serde_json::from_value(
        value
            .get("functions")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    )
    .map_err(|e| e.to_string())
}

/// Register the harness functions on `iii`.
///
/// iii has three primitives: Worker, Function, Trigger. Every entry is a
/// Function. The `agent::` and `provider::` prefixes are naming
/// conventions for grep, not categories; the engine treats every id the
/// same. The eight LLM-callable builtins (`read`, `write`, `edit`, `ls`,
/// `grep`, `find`, `bash`, `run_subagent`) register under the same name
/// the LLM emits in `ContentBlock::ToolCall { name }`. The agent loop
/// dispatches via `iii.trigger(name, payload)` directly; no prefix
/// mapping, no wrapper.
///
/// Provider crates register `provider::<name>::complete` (canonical;
/// `provider::<name>::stream_assistant` remains as a deprecated alias
/// for one release) separately so `agent::stream_assistant` can route
/// to them via `iii.trigger`.
pub async fn register_with_iii(iii: &III) -> anyhow::Result<()> {
    register_stream_assistant(iii);
    register_abort(iii);
    register_push_steering(iii);
    register_push_followup(iii);

    register_http(iii, "agent/{session_id}/steer", "agent::push_steering")?;
    register_http(iii, "agent/{session_id}/abort", "agent::abort")?;
    register_http(iii, "agent/{session_id}/follow_up", "agent::push_followup")?;

    Ok(())
}

/// Stream name where hook subscribers write their replies.
///
/// Group_id is the per-event uuid. Any custom hook subscriber MUST
/// `stream::set` its reply value here with `group_id = event_id`.
pub const HOOK_REPLY_STREAM: &str = "agent::hook_reply";

fn error_assistant(reason: &str) -> AssistantMessage {
    AssistantMessage {
        content: Vec::new(),
        stop_reason: StopReason::Error,
        error_message: Some(reason.to_string()),
        error_kind: Some(ErrorKind::Transient),
        usage: None,
        model: "harness-runtime".into(),
        provider: "harness-runtime".into(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    }
}

fn register_stream_assistant(iii: &III) {
    let iii_for_handler = iii.clone();
    // One-shot cache for `router::decide` presence. Resolved on the first
    // `agent::stream_assistant` invocation — after that, every turn skips the
    // bus list_functions call and reads an atomic. Topology is assumed fixed
    // for the lifetime of the registered handler. If a user adds llm-router
    // mid-session (rare; harnessd / CLI register-then-invoke patterns set
    // topology before the loop runs), restart to pick it up.
    let router_cache: Arc<RouterPresenceCache> = Arc::new(RouterPresenceCache::default());
    iii.register_function((
        RegisterFunctionMessage::with_id("agent::stream_assistant".to_string())
            .with_description("Route a stream call to the configured provider worker, optionally going through router::decide (llm-router) when present.".to_string()),
        move |payload: Value| {
            let iii = iii_for_handler.clone();
            let cache = router_cache.clone();
            async move {
                let original_provider = payload
                    .get("provider")
                    .and_then(Value::as_str)
                    .or_else(|| payload.get("provider_name").and_then(Value::as_str))
                    .ok_or_else(|| {
                        IIIError::Handler("missing required field: provider".to_string())
                    })?
                    .to_string();

                // llm-router integration. When `router::decide` is registered
                // on the bus (i.e. user ran `iii worker add llm-router`),
                // call it first. The router takes a `RoutingRequest` (prompt
                // is required) and returns a `RoutingDecision` whose `model`
                // field is the chosen model. We extract the last user
                // message as the prompt and forward routing hints. When
                // absent, the request is dispatched directly to the
                // configured provider.
                let mut routed_payload = payload.clone();
                let mut provider = original_provider.clone();
                if cache.has_router(&iii).await {
                    if let Some(routing_request) = build_routing_request(&payload) {
                        let router_resp = iii
                            .trigger(TriggerRequest {
                                function_id: "router::decide".to_string(),
                                payload: routing_request,
                                action: None,
                                timeout_ms: None,
                            })
                            .await;
                        if let Err(ref e) = router_resp {
                            // Surface the router failure instead of silently
                            // falling back to direct dispatch. Operators
                            // expect their router to be load-bearing; a
                            // silent skip masks deny_unknown_fields-style
                            // schema rejects (see iii-hq/workers PR #58).
                            tracing::warn!(
                                error = %e,
                                "router::decide call failed; falling back to direct provider dispatch"
                            );
                        }
                        if let Ok(decision) = router_resp {
                            // RoutingDecision: { model, reason, policy_id?, ab_test_id?,
                            // fallback?, confidence, provider? }. The `provider` field shipped
                            // upstream in llm-router via iii-hq/workers PR #57 (merged
                            // 2026-04-29; commit 1117e2eceeb0d8feb9c8157f82b05bf722345d2c).
                            // Resolution rules:
                            //  1. explicit `provider` on the decision wins, with or without
                            //     a `model` (router may swap provider only — e.g. failover);
                            //  2. otherwise, derive provider from a namespaced model id
                            //     (`anthropic/claude-...`) when present;
                            //  3. otherwise, keep the caller's provider.
                            // Model is lifted independently when present.
                            let model_field = decision.get("model").and_then(Value::as_str);
                            let resolved_provider = decision
                                .get("provider")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                                .or_else(|| {
                                    model_field
                                        .and_then(|m| m.split_once('/'))
                                        .map(|(p, _)| p.to_string())
                                });
                            if let Some(p) = resolved_provider {
                                provider.clone_from(&p);
                                routed_payload["provider"] = Value::String(p);
                            }
                            if let Some(m) = model_field {
                                let model_id = m
                                    .split_once('/')
                                    .map_or_else(|| m.to_string(), |(_, rest)| rest.to_string());
                                routed_payload["model"] = Value::String(model_id);
                            }
                        }
                    }
                }

                let budget_id = payload
                    .get("budget_id")
                    .and_then(Value::as_str)
                    .map(String::from);
                if let Some(bid) = budget_id.as_ref() {
                    let est = payload
                        .get("estimated_cost_usd")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0);
                    let resp = iii
                        .trigger(TriggerRequest {
                            function_id: "budget::check".into(),
                            payload: json!({ "budget_id": bid, "estimated_cost_usd": est }),
                            action: None,
                            timeout_ms: None,
                        })
                        .await;
                    match resp {
                        Ok(v) => {
                            if v.get("allowed").and_then(Value::as_bool) == Some(false) {
                                let reason = v
                                    .get("reason")
                                    .and_then(Value::as_str)
                                    .unwrap_or("budget exceeded");
                                return serde_json::to_value(error_assistant(&format!(
                                    "budget exceeded: {reason}"
                                )))
                                .map_err(|e| IIIError::Handler(e.to_string()));
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "budget::check failed; proceeding without enforcement"
                            );
                        }
                    }
                }

                let target = format!("provider::{provider}::complete");
                let assistant = match iii
                    .trigger(TriggerRequest {
                        function_id: target.clone(),
                        payload: routed_payload,
                        action: None,
                        // Provider streams routinely exceed iii-sdk's 30 s
                        // default for extended-thinking models or large
                        // contexts. 5 minutes covers all observed real-world
                        // cases without unbounding a wedged provider.
                        timeout_ms: Some(300_000),
                    })
                    .await
                {
                    Ok(v) => v,
                    Err(err) => serde_json::to_value(error_assistant(&format!(
                        "{target} not registered or failed: {err}"
                    )))
                    .map_err(|e| IIIError::Handler(e.to_string()))?,
                };
                if let Some(bid) = budget_id {
                    let cost = assistant
                        .get("usage")
                        .and_then(|u| u.get("cost_usd"))
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0);
                    if cost > 0.0 {
                        let _ = iii
                            .trigger(TriggerRequest {
                                function_id: "budget::record".into(),
                                payload: json!({ "budget_id": bid, "cost_usd": cost }),
                                action: None,
                                timeout_ms: None,
                            })
                            .await;
                    }
                }
                Ok(assistant)
            }
        },
    ));
}

/// One-shot lookup cache for the `router::decide` function. The first
/// caller probes `iii.list_functions()`; later callers read an atomic.
/// Concurrent first-callers are serialised by the `Mutex` so we issue at
/// most one bus probe per process lifetime.
#[derive(Default)]
struct RouterPresenceCache {
    /// `true` once `present` has been written by the slow path. Read
    /// atomically on the fast path so concurrent callers don't queue
    /// behind the mutex once the probe is done.
    initialised: std::sync::atomic::AtomicBool,
    /// Result of the one-shot bus probe; only valid once `initialised`
    /// flips to true.
    present: std::sync::atomic::AtomicBool,
    /// Serialises only the slow path (the bus probe). The fast path
    /// never touches it.
    init_lock: tokio::sync::Mutex<()>,
}

impl RouterPresenceCache {
    async fn has_router(&self, iii: &III) -> bool {
        // Fast path: lock-free atomic read.
        if self.initialised.load(std::sync::atomic::Ordering::Acquire) {
            return self.present.load(std::sync::atomic::Ordering::Acquire);
        }
        // Slow path: serialise the probe so concurrent first-callers
        // don't all hit the bus. Re-check inside the lock for the
        // double-checked pattern.
        let _guard = self.init_lock.lock().await;
        if self.initialised.load(std::sync::atomic::Ordering::Acquire) {
            return self.present.load(std::sync::atomic::Ordering::Acquire);
        }
        let present = match list_function_infos(iii).await {
            Ok(infos) => infos.iter().any(|f| f.function_id == "router::decide"),
            Err(e) => {
                tracing::warn!(error = %e, "RouterPresenceCache probe failed; assuming router absent");
                false
            }
        };
        self.present
            .store(present, std::sync::atomic::Ordering::Release);
        self.initialised
            .store(true, std::sync::atomic::Ordering::Release);
        present
    }
}

/// Build the `RoutingRequest` payload llm-router's `router::decide` expects.
///
/// llm-router (iii-hq/workers/llm-router) declares the request type with
/// `#[serde(deny_unknown_fields)]` and requires a non-empty `prompt`. We
/// can't blindly forward harness's payload — it has unknown fields and no
/// `prompt`. Instead, extract the last user message's text content as the
/// prompt, and pass through optional routing hints when the caller supplied
/// them (`tenant`, `feature`, `user`, `tags`, `budget_remaining_usd`,
/// `latency_slo_ms`, `min_quality`, `classifier_id`).
///
/// Returns `None` when the messages array is empty or has no extractable
/// user text — in that case the router is skipped and the loop dispatches
/// directly to the configured provider.
///
/// Contract note for router operators: the `prompt` field reflects the
/// *most recent user turn*, not the cumulative conversation. After a tool
/// dispatch the loop calls `stream_assistant` again, which re-invokes the
/// router with that same prompt — classifiers that key on intent see the
/// original ask, not mid-conversation drift. If you need turn-aware
/// routing, drive policy decisions off `tags` / `feature` instead.
fn build_routing_request(payload: &Value) -> Option<Value> {
    let messages = payload.get("messages").and_then(Value::as_array)?;
    let prompt = messages.iter().rev().find_map(|m| {
        let role = m.get("role").and_then(Value::as_str)?;
        if role != "user" {
            return None;
        }
        let content = m.get("content").and_then(Value::as_array)?;
        let text = content
            .iter()
            .filter_map(|c| {
                if c.get("type").and_then(Value::as_str) == Some("text") {
                    c.get("text").and_then(Value::as_str)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    })?;

    let mut req = json!({ "prompt": prompt });
    for hint in [
        "tenant",
        "feature",
        "user",
        "tags",
        "budget_remaining_usd",
        "latency_slo_ms",
        "min_quality",
        "classifier_id",
    ] {
        if let Some(v) = payload.get(hint) {
            req[hint] = v.clone();
        }
    }
    Some(req)
}

fn register_abort(iii: &III) {
    let iii_for_handler = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id("agent::abort".to_string())
            .with_description("Set abort signal in iii state.".to_string()),
        move |payload: Value| {
            let iii = iii_for_handler.clone();
            async move {
                let session_id = required_str(&payload, "session_id")?;
                if let Err(e) = iii
                    .trigger(TriggerRequest {
                        function_id: "flag::set".to_string(),
                        payload: build_flag_payload("abort", &session_id),
                        action: None,
                        timeout_ms: None,
                    })
                    .await
                {
                    tracing::warn!(error = %e, %session_id, "agent::abort: flag::set failed");
                }
                Ok(json!({ "ok": true }))
            }
        },
    ));
}

fn register_push_steering(iii: &III) {
    let iii_for_handler = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id("agent::push_steering".to_string())
            .with_description("Append messages to a session's steering queue.".to_string()),
        move |payload: Value| {
            let iii = iii_for_handler.clone();
            async move { push_queue(iii, payload, "steering").await }
        },
    ));
}

fn register_push_followup(iii: &III) {
    let iii_for_handler = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id("agent::push_followup".to_string())
            .with_description("Append messages to a session's follow-up queue.".to_string()),
        move |payload: Value| {
            let iii = iii_for_handler.clone();
            async move { push_queue(iii, payload, "followup").await }
        },
    ));
}

async fn push_queue(iii: III, payload: Value, name: &'static str) -> Result<Value, IIIError> {
    let session_id = required_str(&payload, "session_id")?;
    let messages = decode_field::<Vec<AgentMessage>>(&payload, "messages")?.unwrap_or_default();
    let count = messages.len();
    for m in messages {
        let item = serde_json::to_value(&m).map_err(|e| IIIError::Handler(e.to_string()))?;
        if let Err(e) = iii
            .trigger(TriggerRequest {
                function_id: "queue::push".to_string(),
                payload: json!({
                    "name": name,
                    "session_id": session_id,
                    "item": item,
                }),
                action: None,
                timeout_ms: None,
            })
            .await
        {
            tracing::warn!(error = %e, %name, %session_id, "queue::push failed during push_queue");
        }
    }
    Ok(json!({ "ok": true, "queued": count }))
}

fn register_http(iii: &III, api_path: &str, function_id: &str) -> anyhow::Result<()> {
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: function_id.to_string(),
        config: json!({ "api_path": api_path, "http_method": "POST" }),
        metadata: None,
    })
    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(())
}

fn required_str(payload: &Value, field: &str) -> Result<String, IIIError> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| IIIError::Handler(format!("missing required field: {field}")))
}

fn decode_field<T: serde::de::DeserializeOwned>(
    payload: &Value,
    field: &str,
) -> Result<Option<T>, IIIError> {
    payload
        .get(field)
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| IIIError::Handler(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_architecture_spec() {
        assert_eq!(STATE_SCOPE, "agent");
        assert_eq!(TOPIC_BEFORE, "agent::before_tool_call");
        assert_eq!(TOPIC_AFTER, "agent::after_tool_call");
        assert_eq!(EVENTS_STREAM, "agent::events");
    }

    #[test]
    fn error_assistant_carries_reason() {
        let a = error_assistant("boom");
        assert_eq!(a.error_message.as_deref(), Some("boom"));
        assert!(matches!(a.stop_reason, StopReason::Error));
    }

    #[test]
    fn subagent_depth_count_matches_chain() {
        // Depth = number of "::sub-" segments in the parent session id.
        // Mirrors the recursion-bound logic inside shell_subagent::start.
        assert_eq!("root".matches("::sub-").count(), 0);
        assert_eq!("root::sub-1".matches("::sub-").count(), 1);
        assert_eq!("root::sub-1::sub-2".matches("::sub-").count(), 2);
        assert_eq!("root::sub-1::sub-2::sub-3".matches("::sub-").count(), 3);
        // A session id without our chain prefix counts as depth 0.
        assert_eq!("free-form-id".matches("::sub-").count(), 0);
    }

    #[test]
    fn flag_payload_uses_name_and_session_id() {
        let payload = build_flag_payload("abort", "s1");
        assert_eq!(payload["name"], "abort");
        assert_eq!(payload["session_id"], "s1");
    }
}
