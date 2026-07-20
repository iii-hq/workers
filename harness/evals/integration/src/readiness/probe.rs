use std::time::Duration;

use serde_json::{json, Value};

use crate::client::{Client, DEFAULT_CALL_TIMEOUT_MS};
use crate::deadline::Deadline;

use super::catalog::{
    config_failure, missing_functions, missing_trigger_types, registered_trigger_failures,
    topic_failures,
};
use super::{
    contract_failures, ExpectedFunctionContract, ExpectedTriggerBinding, ReadinessReport,
    ReadinessSpec,
};

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

/// Wait until every controlled function is present with its exact live
/// descriptor contract.
pub(crate) async fn wait_for_contracts(
    client: &Client,
    contracts: &[ExpectedFunctionContract],
    deadline: Deadline,
) -> Result<(), ReadinessReport> {
    loop {
        let missing = inspect_function_contracts(client, contracts, deadline).await;
        if missing.is_empty() {
            return Ok(());
        }
        if deadline.is_expired() {
            return Err(ReadinessReport { missing });
        }
        tokio::time::sleep(DISCOVERY_POLL_INTERVAL.min(deadline.remaining())).await;
    }
}

/// Wait until every bound function id is present in registered-trigger
/// discovery.
pub(crate) async fn wait_for_registered_triggers(
    client: &Client,
    bindings: &[ExpectedTriggerBinding],
    deadline: Deadline,
) -> Result<(), ReadinessReport> {
    loop {
        let failures = match registered_trigger_snapshot(client, deadline).await {
            Ok(listed) => registered_trigger_failures(bindings, &listed),
            Err(error) => vec![format!("registered trigger discovery unavailable: {error}")],
        };
        if failures.is_empty() {
            return Ok(());
        }
        if deadline.is_expired() {
            return Err(ReadinessReport { missing: failures });
        }
        tokio::time::sleep(DISCOVERY_POLL_INTERVAL.min(deadline.remaining())).await;
    }
}

pub(crate) async fn registered_trigger_snapshot(
    client: &Client,
    deadline: Deadline,
) -> anyhow::Result<Value> {
    client
        .call_with_deadline(
            "engine::registered-triggers::list",
            json!({ "include_internal": true }),
            deadline,
            DEFAULT_CALL_TIMEOUT_MS,
        )
        .await
        .map_err(anyhow::Error::msg)
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
        Ok(listed) => {
            missing.extend(missing_functions(spec, &listed));
            let available = super::catalog::listed_ids(&listed);
            let contracts = spec
                .functions
                .iter()
                .filter(|contract| available.contains(&contract.function_id))
                .cloned()
                .collect::<Vec<_>>();
            missing.extend(inspect_function_contracts(client, &contracts, deadline).await);
        }
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

async fn inspect_function_contracts(
    client: &Client,
    contracts: &[ExpectedFunctionContract],
    deadline: Deadline,
) -> Vec<String> {
    let mut failures = Vec::new();
    for contract in contracts {
        match client
            .call_with_deadline(
                "engine::functions::info",
                json!({ "function_id": contract.function_id }),
                deadline,
                DEFAULT_CALL_TIMEOUT_MS,
            )
            .await
        {
            Ok(actual) => failures.extend(contract_failures(contract, &actual)),
            Err(error) => failures.push(format!(
                "function {} descriptor unavailable: {error}",
                contract.function_id
            )),
        }
    }
    failures
}
