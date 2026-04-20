use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::{json, Value};

use crate::config::RouterConfig;
use crate::state;
use crate::types::RoutingLogEntry;

pub fn handler(
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
            let feature = payload
                .get("feature")
                .and_then(|v| v.as_str())
                .map(String::from);
            let days = payload
                .get("days")
                .and_then(|v| v.as_u64())
                .unwrap_or(cfg.stats_default_days as u64);

            let horizon = crate::functions::decide::now_ms()
                .saturating_sub(days.saturating_mul(86_400_000));

            let items = state::state_list(&iii, &cfg.state_scope, "routing_log:").await?;
            let mut total = 0u64;
            let mut by_model: std::collections::HashMap<String, u64> =
                std::collections::HashMap::new();
            let mut by_policy: std::collections::HashMap<String, u64> =
                std::collections::HashMap::new();

            for it in items {
                if let Some(v) = it.get("value") {
                    if let Ok(e) = serde_json::from_value::<RoutingLogEntry>(v.clone()) {
                        if e.timestamp_ms < horizon {
                            continue;
                        }
                        if let Some(t) = &tenant {
                            if e.tenant.as_deref() != Some(t.as_str()) {
                                continue;
                            }
                        }
                        if let Some(f) = &feature {
                            if e.feature.as_deref() != Some(f.as_str()) {
                                continue;
                            }
                        }
                        total += 1;
                        *by_model.entry(e.model_selected.clone()).or_insert(0) += 1;
                        if let Some(p) = e.policy_matched {
                            *by_policy.entry(p).or_insert(0) += 1;
                        }
                    }
                }
            }

            Ok(json!({
                "total_requests": total,
                "days": days,
                "by_model": by_model,
                "by_policy": by_policy,
            }))
        })
    }
}
