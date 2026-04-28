use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use iii_sdk::{IIIError, III};
use rand::SeedableRng;
use serde_json::{json, Value};
use std::any::type_name;
use uuid::Uuid;

use crate::config::RouterConfig;
use crate::router::{decide, DecideContext};
use crate::state;
use crate::types::{
    AbTest, ClassifierConfig, ModelHealth, ModelRegistration, Policy, RoutingLogEntry,
    RoutingRequest,
};

pub fn build_handler(
    iii: III,
    cfg: Arc<RouterConfig>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let iii = iii.clone();
        let cfg = cfg.clone();
        Box::pin(async move { handle(iii, cfg, payload).await })
    }
}

async fn handle(iii: III, cfg: Arc<RouterConfig>, payload: Value) -> Result<Value, IIIError> {
    let mut req: RoutingRequest = serde_json::from_value(payload)
        .map_err(|e| IIIError::Handler(format!("parse request: {}", e)))?;
    req.prompt = req.prompt.trim().to_string();
    if req.prompt.is_empty() {
        return Err(IIIError::Handler("missing or empty 'prompt'".to_string()));
    }

    let classifier_id = req
        .classifier_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| cfg.classifier_default_id.clone());

    let (policies, ab_tests, health, classifier, models) = tokio::join!(
        load_policies(&iii, &cfg),
        load_ab_tests(&iii, &cfg),
        load_health(&iii, &cfg),
        load_classifier(&iii, &cfg, &classifier_id),
        load_models(&iii, &cfg),
    );
    let policies = policies?;
    let ab_tests = ab_tests?;
    let health = health?;
    let classifier = classifier?;
    let models = models?;

    // Entropy-backed RNG — millisecond-timestamp seeding collided on burst
    // requests, biasing A/B variant picks.
    let mut rng = rand::rngs::StdRng::from_entropy();
    let ctx = DecideContext {
        policies: &policies,
        ab_tests: &ab_tests,
        health: &health,
        classifier: classifier.as_ref(),
        models: &models,
    };
    let mut decision = decide(&req, ctx, &cfg, &mut rng);

    // Enrich the decision with the provider attached to the chosen model's
    // registration, when one exists. This saves consumers from a separate
    // lookup against `router::model_list` or a parallel models-catalog call
    // when they need to dispatch by provider (e.g. agent harnesses calling
    // `provider::<name>::stream_assistant`).
    if !decision.model.is_empty() && decision.provider.is_none() {
        decision.provider = models
            .iter()
            .find(|mr| mr.model == decision.model)
            .and_then(|mr| mr.provider.clone());
    }

    let request_id = format!("req-{}", Uuid::new_v4());
    let log = RoutingLogEntry {
        timestamp_ms: now_ms(),
        request_id: request_id.clone(),
        tenant: req.tenant.clone(),
        feature: req.feature.clone(),
        model_selected: decision.model.clone(),
        policy_matched: decision.policy_id.clone(),
        ab_test_id: decision.ab_test_id.clone(),
        reason: decision.reason.clone(),
        cost_usd: None,
    };
    if let Err(e) = state::state_set(
        &iii,
        &cfg.state_scope,
        &format!("routing_log:{:020}:{}", log.timestamp_ms, request_id),
        serde_json::to_value(&log).unwrap_or(Value::Null),
    )
    .await
    {
        tracing::warn!(error = %e, "failed to write routing log");
    }

    let mut out = serde_json::to_value(&decision).unwrap_or(Value::Null);
    if let Value::Object(ref mut m) = out {
        m.insert("request_id".into(), json!(request_id));
    }
    Ok(out)
}

async fn load_policies(iii: &III, cfg: &RouterConfig) -> Result<Vec<Policy>, IIIError> {
    load_typed(iii, cfg, "policies:").await
}

async fn load_ab_tests(iii: &III, cfg: &RouterConfig) -> Result<Vec<AbTest>, IIIError> {
    load_typed(iii, cfg, "ab_tests:").await
}

async fn load_health(iii: &III, cfg: &RouterConfig) -> Result<Vec<ModelHealth>, IIIError> {
    load_typed(iii, cfg, "model_health:").await
}

async fn load_models(iii: &III, cfg: &RouterConfig) -> Result<Vec<ModelRegistration>, IIIError> {
    load_typed(iii, cfg, "models:").await
}

async fn load_classifier(
    iii: &III,
    cfg: &RouterConfig,
    id: &str,
) -> Result<Option<ClassifierConfig>, IIIError> {
    let key = format!("classifier:{}", id);
    match state::state_get(iii, &cfg.state_scope, &key).await? {
        Some(v) => match serde_json::from_value(v) {
            Ok(c) => Ok(Some(c)),
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse classifier config");
                Ok(None)
            }
        },
        None => Ok(None),
    }
}

async fn load_typed<T: serde::de::DeserializeOwned>(
    iii: &III,
    cfg: &RouterConfig,
    prefix: &str,
) -> Result<Vec<T>, IIIError> {
    let items = state::state_list(iii, &cfg.state_scope, prefix).await?;
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        let key_hint = it
            .as_object()
            .and_then(|o| o.get("key"))
            .and_then(|k| k.as_str());
        match state::parse_item::<T>(&it) {
            Some(parsed) => out.push(parsed),
            None => {
                tracing::warn!(
                    scope = %cfg.state_scope,
                    prefix = %prefix,
                    key = %key_hint.unwrap_or("<unknown>"),
                    target_type = %type_name::<T>(),
                    "skipping malformed state entry"
                );
            }
        }
    }
    Ok(out)
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
