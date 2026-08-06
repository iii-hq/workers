//! Event-driven collection of all traces associated with one session.

use serde::Deserialize;
use serde_json::json;

use crate::client::{Client, DEFAULT_CALL_TIMEOUT_MS};
use crate::deadline::Deadline;
use crate::probe::{ScenarioProbe, LIFECYCLE_FUNCTION_ID};
use crate::types::trace::{TraceEvidenceV1, TraceTreeV1};

#[derive(Deserialize)]
struct GroupByResponse {
    groups: Vec<TraceGroup>,
}

#[derive(Deserialize)]
struct TraceGroup {
    value: String,
    trace_ids: Vec<String>,
}

/// Collect one correlated evidence set for a root session and its known
/// descendants. The trace store groups by session, so a tree run must union
/// every group before applying the stability oracle; otherwise child lifecycle
/// spans disappear from the evidence floor.
pub(crate) async fn collect_for_sessions(
    client: &Client,
    probe: &ScenarioProbe,
    session_ids: &[String],
    expected_terminal_turns: usize,
    expected_total_turns: usize,
    after_generation: u64,
    deadline: Deadline,
) -> anyhow::Result<TraceEvidenceV1> {
    let mut generation = probe
        .wait_for_trace_change(after_generation, deadline)
        .await?;
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
        let evidence = collect_once(client, session_ids, deadline).await?;
        last_summary = Some(evidence.summary.clone());
        if evidence.is_stable_for(
            expected_terminal_turns,
            expected_total_turns,
            LIFECYCLE_FUNCTION_ID,
        ) {
            return Ok(evidence);
        }
        generation = probe
            .wait_for_trace_change(generation, deadline)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "{error}; traces never stabilized for {expected_terminal_turns} terminal / \
                     {expected_total_turns} total turn(s); last summary: {}",
                    last_summary
                        .as_ref()
                        .and_then(|summary| serde_json::to_string(summary).ok())
                        .unwrap_or_else(|| "none".to_string())
                )
            })?;
    }
}

pub(crate) async fn collect_available_for_sessions(
    client: &Client,
    session_ids: &[String],
    deadline: Deadline,
) -> anyhow::Result<TraceEvidenceV1> {
    collect_once(client, session_ids, deadline).await
}

async fn collect_once(
    client: &Client,
    session_ids: &[String],
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
    let wanted = session_ids
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    let mut trace_ids = groups
        .groups
        .into_iter()
        .filter(|group| wanted.contains(&group.value))
        .flat_map(|group| group.trace_ids)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
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
    // Scope the summary's turn ids to the owner when there IS one owner: the
    // floor matches declared statuses positionally, so a parked-then-woken run
    // must not have a neighbour's turn shifting the positions. A tree run has
    // no single owner — every session in it contributes real completions the
    // floor checks against `tree_sessions`.
    Ok(match session_ids {
        [session_id] => TraceEvidenceV1::for_session(traces, session_id),
        _ => TraceEvidenceV1::new(traces),
    })
}
