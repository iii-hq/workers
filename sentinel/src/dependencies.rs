//! Durable dependency readiness.
//!
//! The worker registers its interface first and claims its durable
//! dependencies after, in the background: the registry capture and the
//! console must see the function surface immediately, and CI boots the binary
//! against an isolated engine where neither `database` nor `queue` exists.
//!
//! Until both answer, the worker is *up* but not *enabled*: ingest stays
//! closed, `sentinel::status.enabled` is false, and the operator can see
//! exactly which dependency is missing instead of watching errors scroll.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use crate::Counters;

/// Functions whose presence means the store and the queue can be used.
pub const REQUIRED_FUNCTIONS: [&str; 2] = ["database::execute", "queue::define"];

const PROBE_TIMEOUT_MS: u64 = 5_000;
const PROBE_INTERVAL: Duration = Duration::from_secs(1);

/// Poll until every required dependency is registered, then open the gate.
/// Runs for the life of the process: a dependency that disappears later is
/// handled by the ingest breaker, not by closing the gate again.
pub async fn wait_until_ready(iii: Arc<IIIClient>, counters: Arc<Counters>) {
    let mut announced = false;
    loop {
        match missing_functions(&iii).await {
            Ok(missing) if missing.is_empty() => {
                counters.mark_ready();
                tracing::info!("sentinel durable dependencies are ready");
                return;
            }
            Ok(missing) => {
                if !announced {
                    tracing::warn!(
                        missing = %missing.join(", "),
                        "waiting for durable dependencies; ingest stays closed until they register"
                    );
                    announced = true;
                }
            }
            Err(error) => {
                if !announced {
                    tracing::warn!(%error, "could not probe durable dependencies; retrying");
                    announced = true;
                }
            }
        }
        tokio::time::sleep(PROBE_INTERVAL).await;
    }
}

/// Which of the required functions the engine does not know about. One batch
/// call — `engine::functions::info` answers per id, so a missing worker is a
/// per-entry error rather than a failed request.
async fn missing_functions(iii: &IIIClient) -> Result<Vec<String>, String> {
    let response = iii
        .trigger(TriggerRequest {
            function_id: "engine::functions::info".into(),
            payload: json!({ "function_ids": REQUIRED_FUNCTIONS }),
            action: None,
            timeout_ms: Some(PROBE_TIMEOUT_MS),
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(collect_missing(&response))
}

fn collect_missing(response: &Value) -> Vec<String> {
    let Some(entries) = response.get("functions").and_then(Value::as_array) else {
        return REQUIRED_FUNCTIONS.iter().map(|id| id.to_string()).collect();
    };
    let mut missing: Vec<String> = REQUIRED_FUNCTIONS
        .iter()
        .filter(|required| {
            !entries.iter().any(|entry| {
                entry.get("error").is_none()
                    && entry.get("function_id").and_then(Value::as_str) == Some(*required)
            })
        })
        .map(|required| required.to_string())
        .collect();
    missing.sort();
    missing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_dependency_reads_as_present() {
        let response = json!({
            "functions": [
                { "function_id": "database::execute", "worker_name": "database" },
                { "function_id": "queue::define", "worker_name": "queue" },
            ]
        });
        assert!(collect_missing(&response).is_empty());
    }

    #[test]
    fn a_per_entry_error_names_the_dependency_that_is_missing() {
        let response = json!({
            "functions": [
                { "function_id": "database::execute", "error": "not_found" },
                { "function_id": "queue::define", "worker_name": "queue" },
            ]
        });
        assert_eq!(collect_missing(&response), vec!["database::execute"]);
    }

    #[test]
    fn an_unexpected_shape_reads_as_nothing_ready() {
        assert_eq!(collect_missing(&json!({})).len(), REQUIRED_FUNCTIONS.len());
    }
}
