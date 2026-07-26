/// The terminal-state payload, shared by the global broadcast and the
/// caller-supplied callback.
fn completed_payload(record: &crate::types::WorkflowRunRecord) -> serde_json::Value {
    serde_json::json!({
        "run_id": record.run_id,
        "status": record.status,
        "result": record.result,
        "result_error": record.result_error,
    })
}

/// Global fire-and-forget event broadcast when any run reaches a terminal state.
/// Untargeted: bind a worker to `workflow::run-completed` to observe every run.
/// For a per-run "notify me" callback, see `emit_notify` + `NotifySpec`.
pub async fn emit_run_completed(
    deps: &crate::functions::Deps,
    record: &crate::types::WorkflowRunRecord,
) {
    let request = iii_sdk::protocol::TriggerRequest {
        function_id: "workflow::run-completed".into(),
        payload: completed_payload(record),
        action: Some(iii_sdk::TriggerAction::Void),
        timeout_ms: None,
    };
    let _ = match deps.iii.namespace() {
        Some(ns) => deps.iii.trigger(request.namespace(ns)).await,
        None => deps.iii.trigger(request).await,
    }; // best-effort fire-and-forget
}

/// Fire the caller-supplied completion callback, if the run carries one. The
/// caller named a `function_id` at `workflow::start` time; we push it the run
/// outcome so the caller never has to poll `workflow::status`.
///
/// Durable delivery (enqueued, defaulting to the `"default"` queue) and
/// at-least-once: this fires inside `finalize`, before the terminal status is
/// persisted, so a crash mid-finalize re-ticks and re-fires. The handler must
/// dedup on `run_id`.
pub async fn emit_notify(deps: &crate::functions::Deps, record: &crate::types::WorkflowRunRecord) {
    let Some(notify) = record.notify.as_ref() else {
        return;
    };
    if notify.function_id.trim().is_empty() {
        return;
    }
    let queue = notify
        .queue
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let request = iii_sdk::protocol::TriggerRequest {
        function_id: notify.function_id.clone(),
        payload: completed_payload(record),
        action: Some(iii_sdk::TriggerAction::Enqueue { queue }),
        timeout_ms: None,
    };
    let route_ns = deps.iii.namespace().filter(|_| {
        !matches!(
            notify.function_id.split("::").next(),
            Some(
                "state"
                    | "stream"
                    | "queue"
                    | "pubsub"
                    | "configuration"
                    | "cron"
                    | "http"
                    | "engine"
                    | "sandbox"
                    | "log"
                    | "secret"
                    | "kv"
                    | "iii"
            )
        )
    });
    let _ = match route_ns {
        Some(ns) => deps.iii.trigger(request.namespace(ns)).await,
        None => deps.iii.trigger(request).await,
    }; // best-effort; durable retry is the queue's job
}

/// Build the `harness::send` payload that posts the run outcome into the
/// reply_to session as a new user message. Pure; unit-tested without the bus.
fn reply_send_payload(
    record: &crate::types::WorkflowRunRecord,
    reply: &crate::types::ReplySpec,
    session_id: &str,
) -> serde_json::Value {
    use crate::types::RunStatus;
    let body = match record.status {
        RunStatus::Completed => format!(
            "Workflow {} completed.\n\nResult:\n{}",
            record.run_id,
            serde_json::to_string_pretty(
                record.result.as_ref().unwrap_or(&serde_json::Value::Null)
            )
            .unwrap_or_default()
        ),
        RunStatus::Failed => format!(
            "Workflow {} failed: {}",
            record.run_id,
            record.result_error.as_deref().unwrap_or("unknown error")
        ),
        RunStatus::Cancelled => format!("Workflow {} was cancelled.", record.run_id),
        // Non-terminal: emit_reply only calls this from finalize, so unreachable
        // in practice; return Null so the caller skips the send.
        _ => return serde_json::Value::Null,
    };
    let message = match &reply.template {
        Some(t) if !t.is_empty() => format!("{t}\n\n{body}"),
        _ => body,
    };
    // `harness::send` ALWAYS drives a turn — it seeds a fresh turn or merges into the
    // caller's in-flight one; it has no passive-append / "run" flag (see harness
    // SendRequest/SendOptions). The only lever over the woken turn is its per-turn
    // dispatch policy: when the stamp hook captured the caller's reach, pass it
    // through as `options.functions` so the turn keeps that reach instead of running
    // fail-closed (deny-all, tool-less). Without a captured policy we still deliver
    // the outcome message; that turn simply runs with no tool reach.
    let mut payload = serde_json::json!({
        "session_id": session_id,
        // Deterministic so an at-least-once re-fire is a harness-side no-op.
        "idempotency_key": format!("wfreply_{}", record.run_id),
        "message": message,
    });
    if let Some(f) = &reply.functions {
        payload["options"] = serde_json::json!({ "functions": f });
    }
    if let Some(m) = &reply.model {
        payload["model"] = serde_json::json!(m);
    }
    if let Some(p) = &reply.provider {
        payload["provider"] = serde_json::json!(p);
    }
    payload
}

/// Deliver the run outcome into the caller's session as a new message, if the
/// run carries a `reply_to`. Fires inside `finalize` (at-least-once); the
/// deterministic `idempotency_key` makes a re-fire a harness-side no-op. Skips
/// silently when the caller session/model weren't stamped (hook didn't run or
/// the caller didn't opt in) — `harness::send` requires a model.
pub async fn emit_reply(deps: &crate::functions::Deps, record: &crate::types::WorkflowRunRecord) {
    let Some(reply) = record.reply_to.as_ref() else {
        return;
    };
    let Some(session_id) = reply.session_id.as_deref().filter(|s| !s.is_empty()) else {
        return;
    };
    if reply.model.as_deref().map(str::is_empty).unwrap_or(true) {
        return; // harness::send requires a model; without it we cannot deliver
    }
    let payload = reply_send_payload(record, reply, session_id);
    if payload.is_null() {
        return;
    }
    let request = iii_sdk::protocol::TriggerRequest {
        function_id: "harness::send".into(),
        payload,
        action: None,
        timeout_ms: None,
    };
    let _ = match deps.iii.namespace() {
        Some(ns) => deps.iii.trigger(request.namespace(ns)).await,
        None => deps.iii.trigger(request).await,
    }; // best-effort; deterministic idempotency_key makes re-fire safe
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record_with(status: &str, result: serde_json::Value) -> crate::types::WorkflowRunRecord {
        serde_json::from_value(json!({
            "run_id": "r1", "step": 0, "status": status, "def_ref": "r1",
            "input": {}, "result": result, "created_at": 0, "updated_at": 0
        }))
        .expect("minimal record")
    }

    #[test]
    fn completed_payload_carries_the_outcome() {
        let rec = record_with("completed", json!({"ok": true}));
        let p = completed_payload(&rec);
        assert_eq!(p["run_id"], "r1");
        assert_eq!(p["status"], "completed");
        assert_eq!(p["result"], json!({"ok": true}));
        // result_error is absent on a clean completion → serialized as null.
        assert!(p["result_error"].is_null());
    }

    fn reply(spec: serde_json::Value) -> crate::types::ReplySpec {
        serde_json::from_value(spec).expect("ReplySpec")
    }

    #[test]
    fn reply_payload_completed_carries_result_and_keys() {
        let rec = record_with("completed", json!({ "winner": "X" }));
        let p = reply_send_payload(
            &rec,
            &reply(json!({ "session_id": "s1", "model": "m1", "template": "Done:" })),
            "s1",
        );
        assert_eq!(p["session_id"], "s1");
        assert_eq!(p["model"], "m1");
        assert_eq!(p["idempotency_key"], "wfreply_r1");
        let msg = p["message"].as_str().unwrap();
        assert!(msg.starts_with("Done:"), "msg: {msg}");
        assert!(msg.contains("winner"), "msg: {msg}");
        // No captured caller policy → no `options.functions`, so the woken turn runs
        // fail-closed. (`harness::send` always drives a turn; there is no run/passive
        // flag — the wake-with-reach path is covered below.)
        assert!(
            p.get("options").is_none(),
            "no options without a captured policy"
        );
        assert!(p.get("run").is_none(), "no phantom top-level `run` field");
    }

    #[test]
    fn reply_with_captured_policy_wakes_caller_with_reach() {
        let rec = record_with("completed", json!({ "ok": true }));
        let p = reply_send_payload(
            &rec,
            &reply(json!({
                "session_id": "s1",
                "model": "m1",
                "functions": { "allow": ["*"], "deny": ["workflow::*"] }
            })),
            "s1",
        );
        // A captured caller policy is passed through as the woken turn's reach.
        assert_eq!(p["options"]["functions"]["allow"][0], "*");
        assert_eq!(p["options"]["functions"]["deny"][0], "workflow::*");
    }

    #[test]
    fn reply_payload_failed_carries_error() {
        let mut rec = record_with("failed", json!(null));
        rec.result_error = Some("boom".into());
        let p = reply_send_payload(
            &rec,
            &reply(json!({ "session_id": "s1", "model": "m1" })),
            "s1",
        );
        assert!(p["message"].as_str().unwrap().contains("boom"));
        assert!(p.get("provider").is_none(), "provider omitted when unset");
    }
}
