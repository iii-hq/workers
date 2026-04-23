use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, III};
use rand::SeedableRng;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::config::RouterConfig;
use crate::router::{decide, match_policy, DecideContext};
use crate::state;
use crate::types::{
    AbTest, ClassifierConfig, ModelHealth, ModelRegistration, Policy, RoutingRequest,
};

fn key_for(id: &str) -> String {
    format!("policies:{}", id)
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
            let mut p: Policy = parse_policy(payload)?;
            if p.id.is_empty() {
                p.id = format!("pol-{}", Uuid::new_v4());
            }
            validate_policy_semantics(&p)?;
            p.created_at_ms = crate::functions::decide::now_ms();
            state::state_set(
                &iii,
                &cfg.state_scope,
                &key_for(&p.id),
                serde_json::to_value(&p).unwrap(),
            )
            .await?;
            Ok(json!({ "policy_id": p.id, "created": true }))
        })
    }
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
            let id = payload
                .get("policy_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| IIIError::Handler("missing 'policy_id'".into()))?
                .to_string();
            let existing = state::state_get(&iii, &cfg.state_scope, &key_for(&id))
                .await?
                .ok_or_else(|| IIIError::Handler(format!("policy not found: {}", id)))?;
            let mut p: Policy = serde_json::from_value(existing)
                .map_err(|e| IIIError::Handler(format!("parse stored policy: {}", e)))?;
            merge_policy(&mut p, &payload)?;
            validate_policy_semantics(&p)?;
            state::state_set(
                &iii,
                &cfg.state_scope,
                &key_for(&id),
                serde_json::to_value(&p).unwrap(),
            )
            .await?;
            Ok(serde_json::to_value(&p).unwrap_or(Value::Null))
        })
    }
}

pub fn delete_handler(
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
                .get("policy_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| IIIError::Handler("missing 'policy_id'".into()))?
                .to_string();
            state::state_delete(&iii, &cfg.state_scope, &key_for(&id)).await?;
            Ok(json!({ "deleted": true, "policy_id": id }))
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
    move |payload: Value| {
        let iii = iii.clone();
        let cfg = cfg.clone();
        Box::pin(async move {
            let tenant = payload
                .get("tenant")
                .and_then(|v| v.as_str())
                .map(String::from);
            let enabled_only = payload
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let items = state::state_list(&iii, &cfg.state_scope, "policies:").await?;
            let mut out: Vec<Policy> = items
                .into_iter()
                .filter_map(|it| state::parse_item::<Policy>(&it))
                .collect();
            if let Some(t) = &tenant {
                out.retain(|p| p.match_rule.tenant.as_deref() == Some(t.as_str()));
            }
            if enabled_only {
                out.retain(|p| p.enabled);
            }
            Ok(json!({ "policies": out, "count": out.len() }))
        })
    }
}

pub fn test_handler(
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
            let req: RoutingRequest = serde_json::from_value(payload)
                .map_err(|e| IIIError::Handler(format!("parse request: {}", e)))?;

            // Mirror the production decide path — load every scope the real
            // handler consults so dry-runs can't silently diverge.
            let classifier_id = req
                .classifier_id
                .clone()
                .unwrap_or_else(|| cfg.classifier_default_id.clone());
            let policies = load_policies(&iii, &cfg).await?;
            let ab_tests = load_list::<AbTest>(&iii, &cfg, "ab_tests:").await?;
            let health = load_list::<ModelHealth>(&iii, &cfg, "model_health:").await?;
            let models = load_list::<ModelRegistration>(&iii, &cfg, "models:").await?;
            let classifier = match state::state_get(
                &iii,
                &cfg.state_scope,
                &format!("classifier:{}", classifier_id),
            )
            .await?
            {
                Some(v) => serde_json::from_value::<ClassifierConfig>(v).ok(),
                None => None,
            };

            let matched: Vec<_> = policies
                .iter()
                .filter(|p| match_policy(&req, p))
                .cloned()
                .collect();

            let mut rng = rand::rngs::StdRng::seed_from_u64(0);
            let ctx = DecideContext {
                policies: &policies,
                ab_tests: &ab_tests,
                health: &health,
                classifier: classifier.as_ref(),
                models: &models,
            };
            let decision = decide(&req, ctx, &cfg, &mut rng);
            Ok(json!({
                "matched_policies": matched,
                "decision": decision,
            }))
        })
    }
}

async fn load_policies(iii: &III, cfg: &RouterConfig) -> Result<Vec<Policy>, IIIError> {
    load_list::<Policy>(iii, cfg, "policies:").await
}

async fn load_list<T: serde::de::DeserializeOwned>(
    iii: &III,
    cfg: &RouterConfig,
    prefix: &str,
) -> Result<Vec<T>, IIIError> {
    let items = state::state_list(iii, &cfg.state_scope, prefix).await?;
    Ok(items
        .into_iter()
        .filter_map(|it| state::parse_item::<T>(&it))
        .collect())
}

fn parse_policy(payload: Value) -> Result<Policy, IIIError> {
    let mut v = payload;
    if v.get("id").is_none() {
        if let Value::Object(ref mut m) = v {
            m.insert("id".into(), Value::String(String::new()));
        }
    }
    serde_json::from_value::<Policy>(v)
        .map_err(|e| IIIError::Handler(format!("parse policy: {}", e)))
}

fn validate_policy_semantics(p: &Policy) -> Result<(), IIIError> {
    if p.action.model.trim().is_empty() {
        return Err(IIIError::Handler(
            "policy.action.model must be non-empty".into(),
        ));
    }
    if let Some(max) = p.action.max_cost_per_request_usd {
        if max < 0.0 || max.is_nan() {
            return Err(IIIError::Handler(format!(
                "policy.action.max_cost_per_request_usd must be >= 0 (got {})",
                max
            )));
        }
    }
    Ok(())
}

fn merge_policy(target: &mut Policy, patch: &Value) -> Result<(), IIIError> {
    if let Some(n) = patch.get("name").and_then(|v| v.as_str()) {
        target.name = n.to_string();
    }
    if let Some(m) = patch.get("match") {
        target.match_rule = serde_json::from_value(m.clone())
            .map_err(|e| IIIError::Handler(format!("invalid 'match' in patch: {}", e)))?;
    }
    if let Some(a) = patch.get("action") {
        target.action = serde_json::from_value(a.clone())
            .map_err(|e| IIIError::Handler(format!("invalid 'action' in patch: {}", e)))?;
    }
    if let Some(raw) = patch.get("priority") {
        let p = raw
            .as_i64()
            .ok_or_else(|| IIIError::Handler("invalid 'priority': not an integer".into()))?;
        if p < i32::MIN as i64 || p > i32::MAX as i64 {
            return Err(IIIError::Handler(format!(
                "'priority' out of range for i32: {}",
                p
            )));
        }
        target.priority = p as i32;
    }
    if let Some(e) = patch.get("enabled").and_then(|v| v.as_bool()) {
        target.enabled = e;
    }
    Ok(())
}
