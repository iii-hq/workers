//! `hook-fanout::publish_collect` handler.

use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::{IIIError, RegisterFunctionMessage, TriggerRequest, Value, III};
use serde_json::json;
use uuid::Uuid;

use crate::config::WorkerConfig;
use crate::{
    build_publish_envelope, merge_field_merge, merge_first_block_wins, merge_pipeline_last_wins,
    MergeRule, FUNCTION_ID, HOOK_REPLY_STREAM,
};

pub async fn execute(
    iii: Arc<III>,
    cfg: Arc<WorkerConfig>,
    payload: Value,
) -> Result<Value, IIIError> {
    let topic = payload
        .get("topic")
        .and_then(Value::as_str)
        .ok_or_else(|| IIIError::Handler("missing required field: topic".into()))?
        .to_string();
    let inner = payload.get("payload").cloned().unwrap_or_else(|| json!({}));
    let merge_rule_str = payload
        .get("merge_rule")
        .and_then(Value::as_str)
        .ok_or_else(|| IIIError::Handler("missing required field: merge_rule".into()))?;
    let merge_rule = MergeRule::parse(merge_rule_str)
        .ok_or_else(|| IIIError::Handler(format!("unknown merge_rule: {merge_rule_str}")))?;
    let timeout_ms = payload
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(cfg.default_timeout_ms);
    let expected_replies = payload.get("expected_replies").and_then(Value::as_u64);
    let quiescence_ms = payload
        .get("quiescence_ms")
        .and_then(Value::as_u64)
        .unwrap_or(cfg.quiescence_ms);

    let min_timeout = cfg.min_timeout_ms;
    let poll_interval = Duration::from_millis(cfg.poll_interval_ms);
    let quiescence = Duration::from_millis(quiescence_ms);

    let event_id = Uuid::new_v4().to_string();
    let envelope = build_publish_envelope(&topic, &event_id, inner.clone());
    let started_at = Instant::now();
    let mut publish_failed = false;
    let mut publish_error: Option<String> = None;
    if let Err(e) = iii
        .trigger(TriggerRequest {
            function_id: "iii::durable::publish".into(),
            payload: envelope,
            action: None,
            timeout_ms: None,
        })
        .await
    {
        tracing::warn!(error = %e, %topic, "hook-fanout::publish_collect: publish trigger failed");
        publish_failed = true;
        publish_error = Some(e.to_string());
    }

    let deadline = started_at + Duration::from_millis(timeout_ms.max(min_timeout));
    let mut replies: Vec<Value> = Vec::new();
    let mut last_index: usize = 0;
    let mut first_reply_at: Option<Instant> = None;
    let mut last_reply_at: Option<Instant> = None;
    let exit_reason: &'static str;
    loop {
        let before_len = replies.len();
        if let Ok(value) = iii
            .trigger(TriggerRequest {
                function_id: "stream::list".into(),
                payload: json!({ "stream_name": HOOK_REPLY_STREAM, "group_id": event_id }),
                action: None,
                timeout_ms: None,
            })
            .await
        {
            collect_stream_items(&value, &mut replies, &mut last_index);
        }
        let now = Instant::now();
        if replies.len() > before_len {
            if first_reply_at.is_none() {
                first_reply_at = Some(now);
            }
            last_reply_at = Some(now);
        }

        if let Some(reason) = decide_exit(
            replies.len() as u64,
            expected_replies,
            quiescence_ms,
            quiescence,
            last_reply_at,
            now,
            deadline,
        ) {
            exit_reason = reason;
            break;
        }
        tokio::time::sleep(poll_interval).await;
    }

    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    let first_reply_ms = first_reply_at
        .map(|t| t.duration_since(started_at).as_millis() as u64)
        .unwrap_or(0);
    iii_sdk::set_current_span_attribute("hook_fanout.topic", topic.clone());
    iii_sdk::set_current_span_attribute("hook_fanout.replies", replies.len().to_string());
    iii_sdk::set_current_span_attribute("hook_fanout.elapsed_ms", elapsed_ms.to_string());
    iii_sdk::set_current_span_attribute("hook_fanout.first_reply_ms", first_reply_ms.to_string());
    iii_sdk::set_current_span_attribute("hook_fanout.exit_reason", exit_reason);
    if publish_failed {
        iii_sdk::set_current_span_attribute("hook_fanout.publish_failed", "true");
    }
    tracing::info!(
        topic = %topic,
        replies = replies.len(),
        elapsed_ms = elapsed_ms,
        first_reply_ms = first_reply_ms,
        exit_reason = exit_reason,
        publish_failed = publish_failed,
        "hook-fanout::publish_collect completed"
    );

    let merged = match merge_rule {
        MergeRule::FirstBlockWins => merge_first_block_wins(&replies),
        MergeRule::FieldMerge => merge_field_merge(inner.clone(), &replies),
        MergeRule::PipelineLastWins => merge_pipeline_last_wins(inner.clone(), &replies),
    };

    Ok(build_response(
        &event_id,
        replies,
        merged,
        publish_failed,
        publish_error.as_deref(),
    ))
}

/// Pure response builder. Splits out the JSON assembly so we can unit-test
/// the safety-critical `publish_failed` surface without booting an iii
/// engine. Callers in turn-orchestrator branch on `publish.ok == false`
/// to fail closed (finding #1 follow-up): a publish that the durable
/// substrate refused to deliver MUST NOT look identical to a publish
/// that delivered to zero subscribers.
pub(crate) fn build_response(
    event_id: &str,
    replies: Vec<Value>,
    merged: Value,
    publish_failed: bool,
    publish_error: Option<&str>,
) -> Value {
    let mut out = json!({
        "event_id": event_id,
        "replies": replies,
        "merged": merged,
    });
    // Always include the `publish` block — present even on success
    // (`{ok: true}`) so consumers don't have to guess between "field
    // absent because old worker" and "publish actually succeeded".
    let publish = match (publish_failed, publish_error) {
        (false, _) => json!({ "ok": true }),
        (true, Some(err)) => json!({ "ok": false, "error": err }),
        (true, None) => json!({ "ok": false }),
    };
    out["publish"] = publish;
    // Top-level flag preserved for one release as a legacy/audit aid;
    // new consumers should branch on `publish.ok`.
    if publish_failed {
        out["publish_failed"] = json!(true);
    }
    out
}

pub fn register(iii: &Arc<III>, config: &Arc<WorkerConfig>) {
    let iii_for_handler = iii.clone();
    let cfg = config.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(FUNCTION_ID.to_string()).with_description(
            "Publish a topic, collect subscriber replies until timeout, apply merge_rule."
                .to_string(),
        ),
        move |payload: Value| {
            let iii = iii_for_handler.clone();
            let cfg = cfg.clone();
            async move { execute(iii, cfg, payload).await }
        },
    ));
}

/// Pure exit-decision for the collect loop. Returns `Some(reason)` when
/// the loop must stop, `None` when polling should continue.
///
/// Precedence (matches the order checked in `execute`):
/// 1. `expected_replies` — exits immediately once the reply count
///    meets the caller-supplied target.
/// 2. `quiescence` — exits when at least one reply has landed AND
///    `quiescence` has elapsed since the last one. Disabled when
///    `quiescence_ms == 0`.
/// 3. `deadline` / `deadline_no_replies` — exits when wall-clock
///    crosses the deadline; the variant depends on whether any reply
///    was seen.
fn decide_exit(
    replies_count: u64,
    expected_replies: Option<u64>,
    quiescence_ms: u64,
    quiescence: Duration,
    last_reply_at: Option<Instant>,
    now: Instant,
    deadline: Instant,
) -> Option<&'static str> {
    if let Some(expected) = expected_replies {
        if replies_count >= expected {
            return Some("expected_replies");
        }
    }
    if quiescence_ms > 0 {
        if let Some(last) = last_reply_at {
            if now.duration_since(last) >= quiescence {
                return Some("quiescence");
            }
        }
    }
    if now >= deadline {
        return Some(if last_reply_at.is_some() {
            "deadline"
        } else {
            "deadline_no_replies"
        });
    }
    None
}

fn collect_stream_items(value: &Value, collected: &mut Vec<Value>, last_index: &mut usize) {
    let items = value
        .as_array()
        .cloned()
        .or_else(|| value.get("items").and_then(Value::as_array).cloned());
    if let Some(items) = items {
        for item in items.iter().skip(*last_index) {
            let payload = item.get("data").cloned().unwrap_or_else(|| item.clone());
            collected.push(payload);
        }
        *last_index = items.len();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn collects_items_in_array_shape() {
        let value = json!([
            { "data": { "block": false }},
            { "data": { "block": true, "reason": "x" }},
        ]);
        let mut out = Vec::new();
        let mut idx = 0;
        collect_stream_items(&value, &mut out, &mut idx);
        assert_eq!(out.len(), 2);
        assert_eq!(idx, 2);
        assert_eq!(out[1]["block"], true);
    }

    #[test]
    fn collects_items_in_legacy_wrapped_shape() {
        let value = json!({ "items": [{ "data": { "ok": 1 }}] });
        let mut out = Vec::new();
        let mut idx = 0;
        collect_stream_items(&value, &mut out, &mut idx);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["ok"], 1);
    }

    #[test]
    fn skips_already_seen_items() {
        let v1 = json!([{ "data": { "n": 1 }}]);
        let v2 = json!([{ "data": { "n": 1 }}, { "data": { "n": 2 }}]);
        let mut out = Vec::new();
        let mut idx = 0;
        collect_stream_items(&v1, &mut out, &mut idx);
        collect_stream_items(&v2, &mut out, &mut idx);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1]["n"], 2);
    }

    /// `expected_replies` short-circuit fires the moment the count meets
    /// the target — even when quiescence and deadline haven't triggered.
    /// Highest precedence in `decide_exit`; pins that ordering.
    #[test]
    fn decide_exit_fires_expected_replies_at_threshold() {
        let start = Instant::now();
        // Deadline far in the future, no quiescence elapsed yet — only
        // the expected count matters.
        let deadline = start + Duration::from_secs(60);
        let reason = decide_exit(
            3,
            Some(3),
            200,
            Duration::from_millis(200),
            Some(start),
            start,
            deadline,
        );
        assert_eq!(reason, Some("expected_replies"));

        // Same setup, but count is one short — must continue.
        let reason_short = decide_exit(
            2,
            Some(3),
            200,
            Duration::from_millis(200),
            Some(start),
            start,
            deadline,
        );
        assert_eq!(reason_short, None);
    }

    /// `quiescence` fires once `now - last_reply_at >= quiescence` AND
    /// at least one reply has landed. With no reply observed yet, the
    /// branch must NOT fire (otherwise zero-subscriber topics would
    /// exit immediately).
    #[test]
    fn decide_exit_fires_quiescence_after_idle_window() {
        let start = Instant::now();
        let deadline = start + Duration::from_secs(60);
        let last = start + Duration::from_millis(100);
        // 350ms since first reply, quiescence window = 200ms → fire.
        let now = start + Duration::from_millis(350);
        let reason = decide_exit(
            1,
            None,
            200,
            Duration::from_millis(200),
            Some(last),
            now,
            deadline,
        );
        assert_eq!(reason, Some("quiescence"));

        // last_reply_at = None (no reply yet) → quiescence must NOT
        // fire even though deadline isn't reached either.
        let reason_no_reply = decide_exit(
            0,
            None,
            200,
            Duration::from_millis(200),
            None,
            now,
            deadline,
        );
        assert_eq!(reason_no_reply, None);
    }

    /// `quiescence_ms == 0` disables the quiescence branch entirely
    /// (the documented opt-out). Pins so a refactor can't accidentally
    /// flip the guard.
    #[test]
    fn decide_exit_quiescence_zero_disables_branch() {
        let start = Instant::now();
        let deadline = start + Duration::from_secs(60);
        let now = start + Duration::from_millis(500);
        let reason = decide_exit(
            1,
            None,
            0,
            Duration::from_millis(0),
            Some(start),
            now,
            deadline,
        );
        assert_eq!(reason, None, "quiescence_ms=0 must NOT trigger exit");
    }

    /// `deadline` variant fires when `now >= deadline` AND at least one
    /// reply has landed. Counterpart: `deadline_no_replies` is covered
    /// in the next test.
    #[test]
    fn decide_exit_deadline_with_reply() {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(100);
        let now = start + Duration::from_millis(150);
        let reason = decide_exit(
            1,
            None,
            0,
            Duration::from_millis(0),
            Some(start + Duration::from_millis(50)),
            now,
            deadline,
        );
        assert_eq!(reason, Some("deadline"));
    }

    /// `deadline_no_replies` fires when the deadline is reached and
    /// `last_reply_at` is still None — distinguishes the zero-subscriber
    /// case from the slow-subscriber case in observability dashboards.
    #[test]
    fn decide_exit_deadline_no_replies() {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(100);
        let now = start + Duration::from_millis(150);
        let reason = decide_exit(0, None, 0, Duration::from_millis(0), None, now, deadline);
        assert_eq!(reason, Some("deadline_no_replies"));
    }

    // ──────────────────────────────────────────────────────────────────
    // Finding #1 follow-up: publish_failed must be visible in the
    // response so consumers (turn-orchestrator) can fail closed.
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn build_response_marks_publish_ok_on_success() {
        let out = build_response(
            "evt-1",
            vec![json!({ "block": false })],
            json!({ "block": false }),
            false,
            None,
        );
        assert_eq!(out["event_id"], "evt-1");
        assert_eq!(out["publish"]["ok"], true);
        assert!(out["publish"]["error"].is_null());
        assert!(
            !out.as_object().unwrap().contains_key("publish_failed"),
            "publish_failed top-level flag only appears on failure",
        );
    }

    #[test]
    fn build_response_marks_publish_failed_with_error_text() {
        let out = build_response("evt-2", vec![], json!({ "block": false }), true, Some("ws closed"));
        assert_eq!(out["publish"]["ok"], false);
        assert_eq!(out["publish"]["error"], "ws closed");
        assert_eq!(
            out["publish_failed"], true,
            "legacy top-level flag stays for one release",
        );
        // Critical safety property: the response can be merged-shape
        // identical to "no subscribers blocked" — the ONLY distinguisher
        // is the publish.ok field. Without it the orchestrator can't tell
        // a broken bus from a green policy decision.
        assert_eq!(out["merged"]["block"], false);
    }

    /// Precedence sanity: when expected_replies AND quiescence AND
    /// deadline all match, expected_replies wins (highest priority).
    #[test]
    fn decide_exit_expected_replies_beats_quiescence_and_deadline() {
        let start = Instant::now();
        let deadline = start; // already at deadline
        let last = start;
        let now = start + Duration::from_millis(500);
        let reason = decide_exit(
            5,
            Some(3), // expected met
            200,
            Duration::from_millis(200), // quiescence elapsed too
            Some(last),
            now,
            deadline,
        );
        assert_eq!(reason, Some("expected_replies"));
    }
}
