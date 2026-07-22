//! Collection and stabilization of all traces associated with one session.

use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::client::{Client, DEFAULT_CALL_TIMEOUT_MS};
use crate::deadline::Deadline;
use crate::probe::LIFECYCLE_FUNCTION_ID;
use crate::types::trace::{TraceEvidenceV1, TraceTreeV1};

const TRACE_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Deserialize)]
struct GroupByResponse {
    groups: Vec<TraceGroup>,
}

#[derive(Deserialize)]
struct TraceGroup {
    value: String,
    trace_ids: Vec<String>,
}

pub(crate) async fn collect(
    client: &Client,
    session_id: &str,
    expected_terminal_turns: usize,
    deadline: Deadline,
) -> anyhow::Result<TraceEvidenceV1> {
    let mut previous = None;
    let mut last_summary = None;

    loop {
        if deadline.is_expired() {
            anyhow::bail!(
                "stable session traces exceeded their deadline; last trace summary: {}",
                last_summary
                    .and_then(|summary| serde_json::to_string(&summary).ok())
                    .unwrap_or_else(|| "none".to_string())
            );
        }
        let evidence = collect_once(client, session_id, deadline).await?;
        last_summary = Some(evidence.summary.clone());
        if evidence.is_stable_for(expected_terminal_turns, LIFECYCLE_FUNCTION_ID) {
            let fingerprint = serde_json::to_vec(&evidence.traces)?;
            if previous.as_ref() == Some(&fingerprint) {
                return Ok(evidence);
            }
            previous = Some(fingerprint);
        } else {
            previous = None;
        }
        deadline
            .timeout(
                "trace stabilization interval",
                tokio::time::sleep(TRACE_POLL_INTERVAL),
            )
            .await
            .map_err(|error| anyhow::anyhow!(error))?;
    }
}

pub(crate) async fn collect_available(
    client: &Client,
    session_id: &str,
    deadline: Deadline,
) -> anyhow::Result<TraceEvidenceV1> {
    collect_once(client, session_id, deadline).await
}

async fn collect_once(
    client: &Client,
    session_id: &str,
    deadline: Deadline,
) -> anyhow::Result<TraceEvidenceV1> {
    let response = client
        .call_with_deadline(
            "engine::traces::group_by",
            json!({
                "attribute": "iii.session.id",
                "limit": 100,
                "include_internal": true
            }),
            deadline,
            DEFAULT_CALL_TIMEOUT_MS,
        )
        .await
        .map_err(anyhow::Error::msg)?;
    let groups: GroupByResponse = serde_json::from_value(response)?;
    let mut trace_ids = groups
        .groups
        .into_iter()
        .find(|group| group.value == session_id)
        .map(|group| group.trace_ids)
        .unwrap_or_default();
    trace_ids.sort();

    let mut traces = Vec::with_capacity(trace_ids.len());
    for trace_id in trace_ids {
        let response = client
            .call_with_deadline(
                "engine::traces::tree",
                json!({ "trace_id": trace_id }),
                deadline,
                DEFAULT_CALL_TIMEOUT_MS,
            )
            .await
            .map_err(anyhow::Error::msg)?;
        traces.push(TraceTreeV1::from_engine_response(trace_id, response)?);
    }
    Ok(TraceEvidenceV1::new(traces))
}
