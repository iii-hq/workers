//! `hooks::publish_collect` handler.

use std::time::{Duration, Instant};

use iii_sdk::{IIIError, RegisterFunctionMessage, TriggerRequest, Value, III};
use serde_json::json;
use uuid::Uuid;

use crate::{
    build_publish_envelope, merge_field_merge, merge_first_block_wins, merge_pipeline_last_wins,
    MergeRule, FUNCTION_ID, HOOK_REPLY_STREAM,
};

const POLL_INTERVAL_MS: u64 = 25;
const MIN_TIMEOUT_MS: u64 = 50;

pub async fn execute(iii: III, payload: Value) -> Result<Value, IIIError> {
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
        .unwrap_or(10_000);

    let event_id = Uuid::new_v4().to_string();
    let envelope = build_publish_envelope(&topic, &event_id, inner.clone());
    if let Err(e) = iii
        .trigger(TriggerRequest {
            function_id: "publish".into(),
            payload: envelope,
            action: None,
            timeout_ms: None,
        })
        .await
    {
        tracing::warn!(error = %e, %topic, "hooks::publish_collect: publish trigger failed");
    }

    let deadline = Instant::now() + Duration::from_millis(timeout_ms.max(MIN_TIMEOUT_MS));
    let mut replies: Vec<Value> = Vec::new();
    let mut last_index: usize = 0;
    loop {
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
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }

    let merged = match merge_rule {
        MergeRule::FirstBlockWins => merge_first_block_wins(&replies),
        MergeRule::FieldMerge => merge_field_merge(inner.clone(), &replies),
        MergeRule::PipelineLastWins => merge_pipeline_last_wins(inner.clone(), &replies),
    };

    Ok(json!({
        "event_id": event_id,
        "replies": replies,
        "merged": merged,
    }))
}

pub fn register(iii: &III) {
    let iii_for_handler = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(FUNCTION_ID.to_string()).with_description(
            "Publish a topic, collect subscriber replies until timeout, apply merge_rule."
                .to_string(),
        ),
        move |payload: Value| {
            let iii = iii_for_handler.clone();
            async move { execute(iii, payload).await }
        },
    ));
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
}
