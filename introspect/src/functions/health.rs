use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::Value;

pub fn build_handler(
    iii: Arc<III>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |_payload: Value| {
        let iii = iii.clone();
        Box::pin(async move { handle(&iii).await })
    }
}

pub async fn handle(iii: &III) -> Result<Value, IIIError> {
    let (functions_result, workers_result, triggers_result) = tokio::join!(
        iii.list_functions(),
        iii.list_workers(),
        iii.list_triggers(false),
    );

    let functions = functions_result?;
    let workers = workers_result?;
    let triggers = triggers_result?;

    let mut checks: Vec<Value> = Vec::new();

    let triggered_functions: HashSet<String> =
        triggers.iter().map(|t| t.function_id.clone()).collect();
    let orphaned: Vec<String> = functions
        .iter()
        .filter(|f| !triggered_functions.contains(&f.function_id))
        .map(|f| f.function_id.clone())
        .collect();

    let orphan_ok = orphaned.is_empty();
    checks.push(serde_json::json!({
        "name": "orphaned_functions",
        "status": if orphan_ok { "pass" } else { "warn" },
        "detail": if orphan_ok {
            "All functions have at least one trigger".to_string()
        } else {
            format!("Functions without triggers: {}", orphaned.join(", "))
        }
    }));

    let empty_workers: Vec<String> = workers
        .iter()
        .filter(|w| w.name.is_some() && w.function_count == 0)
        .map(|w| w.name.clone().unwrap_or_else(|| w.id.clone()))
        .collect();

    let empty_ok = empty_workers.is_empty();
    checks.push(serde_json::json!({
        "name": "empty_workers",
        "status": if empty_ok { "pass" } else { "warn" },
        "detail": if empty_ok {
            "All workers have at least one function".to_string()
        } else {
            format!("Workers with zero functions: {}", empty_workers.join(", "))
        }
    }));

    let mut seen_ids: HashMap<String, usize> = HashMap::new();
    for f in &functions {
        *seen_ids.entry(f.function_id.clone()).or_insert(0) += 1;
    }
    let duplicates: Vec<String> = seen_ids
        .iter()
        .filter(|(_, count)| **count > 1)
        .map(|(id, count)| format!("{} (x{})", id, count))
        .collect();

    let dup_ok = duplicates.is_empty();
    checks.push(serde_json::json!({
        "name": "duplicate_function_ids",
        "status": if dup_ok { "pass" } else { "fail" },
        "detail": if dup_ok {
            "No duplicate function IDs".to_string()
        } else {
            format!("Duplicate function IDs: {}", duplicates.join(", "))
        }
    }));

    let healthy = checks.iter().all(|c| c["status"] == "pass");
    let timestamp = chrono::Utc::now().to_rfc3339();

    Ok(serde_json::json!({
        "healthy": healthy,
        "checks": checks,
        "timestamp": timestamp
    }))
}
