use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::{json, Value};

use crate::config::RouterConfig;
use crate::router::heuristic_complexity;
use crate::state;
use crate::types::ClassifierConfig;

pub fn classify_handler(
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
            let prompt = payload
                .get("prompt")
                .and_then(|v| v.as_str())
                .ok_or_else(|| IIIError::Handler("missing 'prompt'".into()))?
                .trim()
                .to_string();
            if prompt.is_empty() {
                return Err(IIIError::Handler("empty 'prompt'".into()));
            }
            let classifier_id = payload
                .get("classifier_id")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| cfg.classifier_default_id.clone());

            let (category, confidence) = heuristic_complexity(&prompt);

            let classifier = state::state_get(
                &iii,
                &cfg.state_scope,
                &format!("classifier:{}", classifier_id),
            )
            .await?;
            let mapped_model = classifier
                .as_ref()
                .and_then(|v| serde_json::from_value::<ClassifierConfig>(v.clone()).ok())
                .and_then(|c| c.thresholds.get(category).cloned());

            Ok(json!({
                "classifier_id": classifier_id,
                "complexity": category,
                "confidence": confidence,
                "suggested_model": mapped_model,
            }))
        })
    }
}

pub fn config_handler(
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
            let id = payload
                .get("id")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| cfg.classifier_default_id.clone());
            let mut c: ClassifierConfig = serde_json::from_value(payload)
                .map_err(|e| IIIError::Handler(format!("parse classifier: {}", e)))?;
            c.id = id.clone();
            c.created_at_ms = crate::functions::decide::now_ms();
            state::state_set(
                &iii,
                &cfg.state_scope,
                &format!("classifier:{}", id),
                serde_json::to_value(&c).unwrap_or(Value::Null),
            )
            .await?;
            Ok(json!({ "configured": true, "id": id }))
        })
    }
}
