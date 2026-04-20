use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::config::RouterConfig;
use crate::state;
use crate::types::{AbEvent, AbTest};

fn key_test(id: &str) -> String {
    format!("ab_tests:{}", id)
}
fn key_event(test_id: &str, timestamp_ms: u64, id: &str) -> String {
    format!("ab_events:{}:{:020}:{}", test_id, timestamp_ms, id)
}

pub fn create_handler(
    iii: III,
    cfg: Arc<RouterConfig>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let iii = iii.clone();
        let cfg = cfg.clone();
        Box::pin(async move {
            let mut v = payload;
            if v.get("id").is_none() {
                if let Value::Object(ref mut m) = v {
                    m.insert("id".into(), Value::String(format!("ab-{}", Uuid::new_v4())));
                }
            }
            let mut t: AbTest = serde_json::from_value(v)
                .map_err(|e| IIIError::Handler(format!("parse ab-test: {}", e)))?;
            t.created_at_ms = crate::functions::decide::now_ms();
            state::state_set(
                &iii,
                &cfg.state_scope,
                &key_test(&t.id),
                serde_json::to_value(&t).unwrap(),
            )
            .await?;
            Ok(json!({ "test_id": t.id, "created": true }))
        })
    }
}

pub fn record_handler(
    iii: III,
    cfg: Arc<RouterConfig>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let iii = iii.clone();
        let cfg = cfg.clone();
        Box::pin(async move {
            let test_id = payload
                .get("test_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| IIIError::Handler("missing 'test_id'".into()))?
                .to_string();
            let variant = payload
                .get("variant_model")
                .and_then(|v| v.as_str())
                .ok_or_else(|| IIIError::Handler("missing 'variant_model'".into()))?
                .to_string();
            let ev = AbEvent {
                test_id: test_id.clone(),
                variant_model: variant,
                quality_score: payload
                    .get("quality_score")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                latency_ms: payload
                    .get("latency_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                cost_usd: payload
                    .get("cost_usd")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                recorded_at_ms: crate::functions::decide::now_ms(),
            };
            let evt_id = format!("evt-{}", Uuid::new_v4());
            state::state_set(
                &iii,
                &cfg.state_scope,
                &key_event(&test_id, ev.recorded_at_ms, &evt_id),
                serde_json::to_value(&ev).unwrap(),
            )
            .await?;
            Ok(json!({ "recorded": true, "event_id": evt_id }))
        })
    }
}

pub fn report_handler(
    iii: III,
    cfg: Arc<RouterConfig>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let iii = iii.clone();
        let cfg = cfg.clone();
        Box::pin(async move {
            let test_id = payload
                .get("test_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| IIIError::Handler("missing 'test_id'".into()))?
                .to_string();

            let test_val = state::state_get(&iii, &cfg.state_scope, &key_test(&test_id))
                .await?
                .ok_or_else(|| IIIError::Handler(format!("no such ab-test: {}", test_id)))?;
            let test: AbTest = serde_json::from_value(test_val)
                .map_err(|e| IIIError::Handler(format!("parse test: {}", e)))?;

            let items = state::state_list(
                &iii,
                &cfg.state_scope,
                &format!("ab_events:{}:", test_id),
            )
            .await?;
            let events: Vec<AbEvent> = items
                .into_iter()
                .filter_map(|it| {
                    it.get("value")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                })
                .collect();

            let mut summary: std::collections::HashMap<String, (u64, f64, f64, f64)> =
                std::collections::HashMap::new();
            for e in &events {
                let row = summary.entry(e.variant_model.clone()).or_insert((0, 0.0, 0.0, 0.0));
                row.0 += 1;
                row.1 += e.quality_score;
                row.2 += e.latency_ms as f64;
                row.3 += e.cost_usd;
            }

            let variants_out: Vec<Value> = test
                .variants
                .iter()
                .map(|v| {
                    let (n, q, l, c) = summary
                        .get(&v.model)
                        .copied()
                        .unwrap_or((0, 0.0, 0.0, 0.0));
                    let n_f = (n as f64).max(1.0);
                    json!({
                        "model": v.model,
                        "weight": v.weight,
                        "samples": n,
                        "avg_quality": q / n_f,
                        "avg_latency_ms": l / n_f,
                        "avg_cost_usd": c / n_f,
                    })
                })
                .collect();

            let total_samples: u64 = summary.values().map(|(n, _, _, _)| *n).sum();
            let status = if test.status == "running" && total_samples < test.min_samples as u64 {
                "insufficient_data"
            } else if test.status == "concluded" {
                "concluded"
            } else {
                "running"
            };

            Ok(json!({
                "test_id": test.id,
                "name": test.name,
                "status": status,
                "total_samples": total_samples,
                "variants": variants_out,
            }))
        })
    }
}

pub fn conclude_handler(
    iii: III,
    cfg: Arc<RouterConfig>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let iii = iii.clone();
        let cfg = cfg.clone();
        Box::pin(async move {
            let test_id = payload
                .get("test_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| IIIError::Handler("missing 'test_id'".into()))?
                .to_string();
            let winner = payload
                .get("winner_model")
                .and_then(|v| v.as_str())
                .map(String::from);

            let test_val = state::state_get(&iii, &cfg.state_scope, &key_test(&test_id))
                .await?
                .ok_or_else(|| IIIError::Handler(format!("no such ab-test: {}", test_id)))?;
            let mut test: AbTest = serde_json::from_value(test_val)
                .map_err(|e| IIIError::Handler(format!("parse test: {}", e)))?;
            test.status = "concluded".into();
            state::state_set(
                &iii,
                &cfg.state_scope,
                &key_test(&test_id),
                serde_json::to_value(&test).unwrap(),
            )
            .await?;
            Ok(json!({
                "concluded": true,
                "test_id": test_id,
                "winner_model": winner,
            }))
        })
    }
}
