use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::{json, Value};

use crate::config::RouterConfig;
use crate::state;
use crate::types::ModelHealth;

fn key(model: &str) -> String {
    format!("model_health:{}", model)
}

pub fn update_handler(
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
            let model = payload
                .get("model")
                .and_then(|v| v.as_str())
                .ok_or_else(|| IIIError::Handler("missing 'model'".into()))?
                .trim()
                .to_string();
            if model.is_empty() {
                return Err(IIIError::Handler("empty 'model'".into()));
            }
            let mut h: ModelHealth = serde_json::from_value(payload)
                .map_err(|e| IIIError::Handler(format!("parse health: {}", e)))?;
            if let Some(rate) = h.error_rate {
                if !(0.0..=1.0).contains(&rate) || rate.is_nan() {
                    return Err(IIIError::Handler(format!(
                        "error_rate must be within 0.0..=1.0 (got {})",
                        rate
                    )));
                }
            }
            h.model = model.clone();
            h.last_checked_ms = crate::functions::decide::now_ms();
            state::state_set(
                &iii,
                &cfg.state_scope,
                &key(&model),
                serde_json::to_value(&h).unwrap(),
            )
            .await?;
            Ok(json!({ "updated": true, "model": model }))
        })
    }
}

pub fn list_handler(
    iii: III,
    cfg: Arc<RouterConfig>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |_payload: Value| {
        let iii = iii.clone();
        let cfg = cfg.clone();
        Box::pin(async move {
            let items = state::state_list(&iii, &cfg.state_scope, "model_health:").await?;
            let out: Vec<ModelHealth> = items
                .into_iter()
                .filter_map(|it| state::parse_item::<ModelHealth>(&it))
                .collect();
            Ok(json!({ "models": out, "count": out.len() }))
        })
    }
}
