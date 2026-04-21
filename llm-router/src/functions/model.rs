use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::{json, Value};

use crate::config::RouterConfig;
use crate::state;
use crate::types::ModelRegistration;

fn key(name: &str) -> String {
    format!("models:{}", name)
}

pub fn register_handler(
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
            let mut m: ModelRegistration = serde_json::from_value(payload)
                .map_err(|e| IIIError::Handler(format!("parse model: {}", e)))?;
            m.model = m.model.trim().to_string();
            if m.model.is_empty() {
                return Err(IIIError::Handler("missing or empty 'model'".into()));
            }
            for (field, value) in [
                ("input_per_1m", m.input_per_1m),
                ("output_per_1m", m.output_per_1m),
            ] {
                if let Some(v) = value {
                    if v < 0.0 || v.is_nan() {
                        return Err(IIIError::Handler(format!(
                            "{} must be >= 0 (got {})",
                            field, v
                        )));
                    }
                }
            }
            m.registered_at_ms = crate::functions::decide::now_ms();
            state::state_set(
                &iii,
                &cfg.state_scope,
                &key(&m.model),
                serde_json::to_value(&m).unwrap(),
            )
            .await?;
            Ok(json!({ "registered": true, "model": m.model }))
        })
    }
}

pub fn unregister_handler(
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
            state::state_delete(&iii, &cfg.state_scope, &key(&model)).await?;
            Ok(json!({ "unregistered": true, "model": model }))
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
            let items = state::state_list(&iii, &cfg.state_scope, "models:").await?;
            let out: Vec<ModelRegistration> = items
                .into_iter()
                .filter_map(|it| {
                    it.get("value")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                })
                .collect();
            Ok(json!({ "models": out, "count": out.len() }))
        })
    }
}
