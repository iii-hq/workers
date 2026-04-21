use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::{json, Value};

use crate::config::RouterConfig;
use crate::state;
use crate::types::RoutingLogEntry;

// Routing log keys are `routing_log:<timestamp_ms:020>:<request_id>`. The
// 20-digit zero-padded timestamp makes lexicographic order match chronological
// order, so we can build a bucket-prefixed list per day in the window instead
// of loading every log entry into memory.
//
// Hard cap on total entries per stats call — a stalled consumer shouldn't be
// able to pull megabytes of log into process memory.
const SCAN_HARD_CAP: usize = 50_000;

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

            let now = crate::functions::decide::now_ms();
            let horizon = now.saturating_sub(days.saturating_mul(86_400_000));

            // Narrow the prefix to the shared leading digits of horizon..now,
            // so state::list returns only entries that might match instead of
            // the full audit log.
            let prefix = scan_prefix(horizon, now);
            let items = state::state_list(&iii, &cfg.state_scope, &prefix).await?;

            let mut total = 0u64;
            let mut scanned = 0usize;
            let mut truncated = false;
            let mut by_model: std::collections::HashMap<String, u64> =
                std::collections::HashMap::new();
            let mut by_policy: std::collections::HashMap<String, u64> =
                std::collections::HashMap::new();

            for it in items {
                scanned += 1;
                if scanned > SCAN_HARD_CAP {
                    truncated = true;
                    break;
                }
                let v = match it.as_object() {
                    Some(obj) if obj.contains_key("value") => obj.get("value").cloned(),
                    _ => Some(it.clone()),
                };
                let Some(v) = v else { continue };
                let e = match serde_json::from_value::<RoutingLogEntry>(v) {
                    Ok(e) => e,
                    Err(err) => {
                        tracing::warn!(error = %err, "skipping malformed routing_log entry");
                        continue;
                    }
                };
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

            Ok(json!({
                "total_requests": total,
                "days": days,
                "by_model": by_model,
                "by_policy": by_policy,
                "scanned": scanned,
                "truncated": truncated,
                "scan_hard_cap": SCAN_HARD_CAP,
            }))
        })
    }
}

// Build the narrowest log key prefix that still covers [horizon, now].
// E.g. now=1713696000000, horizon=1713091200000 share the first 4 digits,
// so prefix = "routing_log:1713".
fn scan_prefix(horizon_ms: u64, now_ms: u64) -> String {
    let lo = format!("{:020}", horizon_ms);
    let hi = format!("{:020}", now_ms);
    let mut shared = 0;
    for (a, b) in lo.chars().zip(hi.chars()) {
        if a == b {
            shared += 1;
        } else {
            break;
        }
    }
    format!("routing_log:{}", &lo[..shared])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_prefix_narrows_when_window_is_small() {
        let now: u64 = 1_713_696_000_000;
        let hour_ago = now - 3_600_000;
        let prefix = scan_prefix(hour_ago, now);
        // Last few digits differ, leading 16 are identical.
        assert!(prefix.starts_with("routing_log:"));
        assert!(prefix.len() > "routing_log:".len() + 10);
    }

    #[test]
    fn scan_prefix_falls_back_when_window_is_large() {
        // Window of months / years — the first differing 13th-ish digit means
        // prefix won't narrow much beyond the zero-padding at the front. We
        // only care that we don't over-narrow.
        let now: u64 = 2_000_000_000_000;
        let very_old: u64 = 1_000_000_000_000;
        let prefix = scan_prefix(very_old, now);
        assert!(prefix.starts_with("routing_log:"));
        // Shared prefix is at most the zero-padding (13 digits + leading zeros).
        let suffix = &prefix["routing_log:".len()..];
        assert!(suffix.len() < 13, "over-narrowed: {}", prefix);
    }
}
