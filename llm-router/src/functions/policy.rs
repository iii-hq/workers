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
use crate::types::{Policy, RoutingRequest};

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
            merge_policy(&mut p, &payload);
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
                .filter_map(|it| {
                    it.get("value")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                })
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
            let items = state::state_list(&iii, &cfg.state_scope, "policies:").await?;
            let policies: Vec<Policy> = items
                .into_iter()
                .filter_map(|it| {
                    it.get("value")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                })
                .collect();
            let matched: Vec<_> = policies
                .iter()
                .filter(|p| match_policy(&req, p))
                .cloned()
                .collect();

            let mut rng = rand::rngs::StdRng::seed_from_u64(0);
            let ctx = DecideContext {
                policies: &policies,
                ..DecideContext::default()
            };
            let decision = decide(&req, ctx, &cfg, &mut rng);
            Ok(json!({
                "matched_policies": matched,
                "decision": decision,
            }))
        })
    }
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

fn merge_policy(target: &mut Policy, patch: &Value) {
    if let Some(n) = patch.get("name").and_then(|v| v.as_str()) {
        target.name = n.to_string();
    }
    if let Some(m) = patch.get("match") {
        if let Ok(mm) = serde_json::from_value(m.clone()) {
            target.match_rule = mm;
        }
    }
    if let Some(a) = patch.get("action") {
        if let Ok(aa) = serde_json::from_value(a.clone()) {
            target.action = aa;
        }
    }
    if let Some(p) = patch.get("priority").and_then(|v| v.as_i64()) {
        target.priority = p as i32;
    }
    if let Some(e) = patch.get("enabled").and_then(|v| v.as_bool()) {
        target.enabled = e;
    }
}
