//! Poll engine discovery until required surfaces appear. This is only a boot
//! race barrier — not schema/contract validation.

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::{json, Value};

use crate::client::{Client, DEFAULT_CALL_TIMEOUT_MS};
use crate::deadline::Deadline;

const DISCOVERY_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Functions that must be registered before a turn can run: the harness
/// entrypoint plus the worker surfaces it calls mid-turn. Router and the
/// lifecycle sink are excluded — `RunServices::start` registers those itself.
pub const TURN_SURFACE: &[&str] = &["harness::send", "session::messages", "context::assemble"];

/// Wait until every function id is present in `engine::functions::list`.
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
            let ids = collect_ids(&listed, &["function_id", "id"]);
            if function_ids.iter().all(|id| ids.contains(*id)) {
                Ok(Some(()))
            } else {
                Ok(None)
            }
        })
        .await
}

/// Wait until every trigger type is present in `engine::triggers::list`.
pub async fn wait_for_trigger_types(
    client: &Client,
    trigger_types: &[&str],
    deadline: Deadline,
) -> anyhow::Result<()> {
    let label = format!("trigger types {}", trigger_types.join(", "));
    deadline
        .poll_until(label, DISCOVERY_POLL_INTERVAL, || async {
            let listed = client
                .call_with_deadline(
                    "engine::triggers::list",
                    json!({ "include_internal": true }),
                    deadline,
                    DEFAULT_CALL_TIMEOUT_MS,
                )
                .await
                .map_err(anyhow::Error::msg)?;
            let ids = collect_ids(&listed, &["trigger_type", "id", "name", "type"]);
            if trigger_types.iter().all(|id| ids.contains(*id)) {
                Ok(Some(()))
            } else {
                Ok(None)
            }
        })
        .await
}

fn collect_ids(listed: &Value, keys: &[&str]) -> BTreeSet<String> {
    let items: Vec<&Value> = match listed {
        Value::Array(items) => items.iter().collect(),
        Value::Object(map) => map
            .values()
            .find_map(|value| value.as_array())
            .map(|items| items.iter().collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    items
        .iter()
        .filter_map(|item| {
            if let Some(text) = item.as_str() {
                return Some(text.to_string());
            }
            keys.iter()
                .find_map(|key| item.get(key).and_then(Value::as_str))
                .map(String::from)
        })
        .collect()
}
