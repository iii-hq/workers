//! Bounded boot-time discovery for the function surface required by a turn.
//!
//! The pinned engine's `engine::functions-available` event has no catch-up
//! snapshot, so it cannot safely establish initial readiness. This barrier
//! reads the authoritative registry only during Arm; completion and trace
//! stabilization remain event-driven.

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::{json, Value};

use crate::client::{Client, DEFAULT_CALL_TIMEOUT_MS};
use crate::deadline::Deadline;

const DISCOVERY_POLL_INTERVAL: Duration = Duration::from_millis(200);

pub const TURN_SURFACE: &[&str] = &["harness::send", "session::messages", "context::assemble"];

pub async fn wait_for_functions(
    client: &Client,
    function_ids: &[&str],
    deadline: Deadline,
) -> anyhow::Result<()> {
    let label = format!("functions {}", function_ids.join(", "));
    deadline
        .poll_until(label, DISCOVERY_POLL_INTERVAL, || async {
            let listed = client
                .call_with_deadline(
                    "engine::functions::list",
                    json!({ "include_internal": true }),
                    deadline,
                    DEFAULT_CALL_TIMEOUT_MS,
                )
                .await
                .map_err(anyhow::Error::msg)?;
            let ids = collect_function_ids(&listed);
            Ok(function_ids
                .iter()
                .all(|function_id| ids.contains(*function_id))
                .then_some(()))
        })
        .await
}

fn collect_function_ids(listed: &Value) -> BTreeSet<String> {
    let items = listed
        .as_array()
        .or_else(|| listed.as_object()?.values().find_map(Value::as_array));
    items
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item.as_str()
                .or_else(|| item.get("function_id").and_then(Value::as_str))
                .or_else(|| item.get("id").and_then(Value::as_str))
                .map(str::to_owned)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_function_ids_from_engine_response_shapes() {
        let listed = json!({
            "functions": [
                { "function_id": "harness::send" },
                { "id": "session::messages" }
            ]
        });
        let ids = collect_function_ids(&listed);
        assert!(ids.contains("harness::send"));
        assert!(ids.contains("session::messages"));
    }
}
