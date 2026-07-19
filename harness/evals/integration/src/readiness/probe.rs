use std::time::Duration;

use serde_json::{json, Value};

use crate::client::{Client, DEFAULT_CALL_TIMEOUT_MS};
use crate::deadline::Deadline;

use super::catalog::{
    config_failure, has_all_discovery_ids, missing_functions, missing_trigger_types, topic_failures,
};
use super::{ReadinessReport, ReadinessSpec};

const READINESS_POLL_INTERVAL: Duration = Duration::from_millis(250);
const DISCOVERY_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Probe until ready or deadline. Returns the last report on timeout.
pub async fn probe(
    client: &Client,
    spec: &ReadinessSpec,
    deadline: Deadline,
) -> Result<(), ReadinessReport> {
    loop {
        let report = probe_once(client, spec, deadline).await;
        if report.missing.is_empty() {
            return Ok(());
        }
        if deadline.is_expired() {
            return Err(report);
        }
        tokio::time::sleep(READINESS_POLL_INTERVAL.min(deadline.remaining())).await;
    }
}

/// Wait until every controlled function id is present in engine discovery.
pub(crate) async fn wait_for_functions(
    client: &Client,
    function_ids: &[String],
    deadline: Deadline,
) -> anyhow::Result<()> {
    wait_for_catalog(
        client,
        function_ids,
        deadline,
        "engine::functions::list",
        "controlled function discovery",
        has_all_discovery_ids,
    )
    .await
}

/// Wait until every bound function id is present in registered-trigger
/// discovery.
pub(crate) async fn wait_for_registered_triggers(
    client: &Client,
    function_ids: &[String],
    deadline: Deadline,
) -> anyhow::Result<()> {
    wait_for_catalog(
        client,
        function_ids,
        deadline,
        "engine::registered-triggers::list",
        "registered trigger discovery",
        has_all_discovery_ids,
    )
    .await
}

async fn wait_for_catalog(
    client: &Client,
    function_ids: &[String],
    deadline: Deadline,
    method: &'static str,
    operation: &'static str,
    contains_all: fn(&Value, &[String]) -> bool,
) -> anyhow::Result<()> {
    deadline
        .poll_until(operation, DISCOVERY_POLL_INTERVAL, || async {
            let listed = client
                .call_with_deadline(
                    method,
                    json!({ "include_internal": true }),
                    deadline,
                    DEFAULT_CALL_TIMEOUT_MS,
                )
                .await;
            Ok(listed
                .ok()
                .and_then(|listed| contains_all(&listed, function_ids).then_some(())))
        })
        .await
}

async fn probe_once(client: &Client, spec: &ReadinessSpec, deadline: Deadline) -> ReadinessReport {
    let mut missing = Vec::new();

    // 1. Discovery responds, and every required function id is registered.
    match client
        .call_with_deadline(
            "engine::functions::list",
            json!({ "include_internal": true }),
            deadline,
            DEFAULT_CALL_TIMEOUT_MS,
        )
        .await
    {
        Ok(listed) => missing.extend(missing_functions(spec, &listed)),
        Err(error) => missing.push(format!("engine::functions::list unavailable: {error}")),
    }

    // 2. Trigger types.
    if !spec.trigger_types.is_empty() {
        match client
            .call_with_deadline(
                "engine::triggers::list",
                json!({ "include_internal": true }),
                deadline,
                DEFAULT_CALL_TIMEOUT_MS,
            )
            .await
        {
            Ok(listed) => missing.extend(missing_trigger_types(spec, &listed)),
            Err(error) => missing.push(format!("engine::triggers::list unavailable: {error}")),
        }
    }

    // 3. Queue topics with broker type.
    if !spec.queue_topics.is_empty() {
        match client
            .call_with_deadline(
                "engine::queue::list_topics",
                json!({}),
                deadline,
                DEFAULT_CALL_TIMEOUT_MS,
            )
            .await
        {
            Ok(listed) => missing.extend(topic_failures(spec, &listed)),
            Err(error) => {
                missing.push(format!("engine::queue::list_topics unavailable: {error}"));
            }
        }
    }

    // 4. Seeded configuration entries are authoritative. Workers store their
    // RESOLVED config (seed merged with defaults — observed on first boot),
    // so the check is: every seeded key is present with exactly the seeded
    // value. Recorded as a spec correction to the original byte-compare.
    for (id, expected) in &spec.config_entries {
        match client
            .call_with_deadline(
                "configuration::get",
                json!({ "id": id }),
                deadline,
                DEFAULT_CALL_TIMEOUT_MS,
            )
            .await
        {
            Ok(response) => missing.extend(config_failure(id, expected, &response)),
            Err(error) => missing.push(format!("configuration {id} unavailable: {error}")),
        }
    }

    ReadinessReport { missing }
}
