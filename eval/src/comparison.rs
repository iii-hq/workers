//! Live, read-only comparison of existing root sessions.
//!
//! This module deliberately has no state or queue integration. Every request
//! reads the current session metadata, lifecycle and Harness metrics, then
//! derives mathematical summaries and deltas for the caller to interpret.

use std::collections::BTreeMap;
use std::sync::Arc;

use harness::functions::metrics::{SessionMetricsRequestV1, SessionMetricsResponseV1};
use harness::functions::status::{StatusReport, StatusRequest};
use harness::types::turn::TurnStatus;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::task::JoinSet;

use crate::error::EvalError;
use crate::ids;
use crate::runtime::Deps;

pub const SCHEMA_VERSION: &str = "1";
const COLLECTION_TIMEOUT_MS: u64 = 30_000;

/// The only input needed to compare sessions. The caller is responsible for
/// choosing sessions that are meaningful to compare; the worker only checks
/// shape and root identity.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompareSessionsRequestV1 {
    #[schemars(length(min = 2, max = 5))]
    pub session_ids: Vec<String>,
    pub baseline_session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionComparisonResponseV1 {
    pub schema_version: String,
    pub captured_at: i64,
    pub baseline_session_id: String,
    pub sessions: Vec<SessionComparisonItemV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionComparisonItemV1 {
    pub session: SessionMetaProjectionV1,
    pub lifecycle: SessionLifecycleV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<SessionMetricsResponseV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<ObjectiveSummaryV1>,
    /// One entry per objective numeric metric. A missing value is represented
    /// by null in either delta field, never by zero.
    #[serde(default)]
    pub deltas: BTreeMap<String, MetricDeltaV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionLifecycleV1 {
    pub session_status: SessionStatusV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_status: Option<TurnStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_error: Option<String>,
    /// Null means `harness::status` did not provide a turn record.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<bool>,
    /// Null means the status read was unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expects_wake: Option<bool>,
    /// Null means the metrics read was unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complete: Option<bool>,
    pub partial: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatusV1 {
    #[default]
    Idle,
    Working,
    Done,
    Error,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SessionMetaProjectionV1 {
    pub session_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub status: SessionStatusV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub message_count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ObjectiveSummaryV1 {
    pub total_tokens: Option<u64>,
    pub generations: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub subject_cost_usd: Option<f64>,
    pub tokens_per_generation: Option<f64>,
    pub cost_per_generation_usd: Option<f64>,
    pub function_calls: Option<u64>,
    pub function_call_errors: Option<u64>,
    pub function_error_rate: Option<f64>,
    pub trace_duration_ms: Option<u64>,
    pub trace_count: Option<u64>,
    pub span_count: Option<u64>,
    pub error_span_count: Option<u64>,
    pub sessions: Option<u64>,
    pub descendants: Option<u64>,
    pub max_depth: Option<u32>,
    pub compacted_sessions: Option<u64>,
    pub context_total_tokens: Option<u64>,
    pub context_usable_tokens: Option<u64>,
    pub context_free_tokens: Option<u64>,
    pub context_occupancy: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct MetricDeltaV1 {
    pub absolute: Option<f64>,
    pub percent: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct SessionGetResponseV1 {
    meta: SessionMetaProjectionV1,
}

#[derive(Debug)]
struct SessionObservation {
    meta: SessionMetaProjectionV1,
    status: Result<Option<StatusReport>, EvalError>,
    metrics: Result<SessionMetricsResponseV1, EvalError>,
}

/// Collect all three live sources concurrently for each requested session and
/// all requested sessions concurrently. Metadata is validated before the
/// response is returned, while later status/metrics failures stay local.
pub async fn compare(
    deps: &Deps,
    request: CompareSessionsRequestV1,
) -> Result<SessionComparisonResponseV1, EvalError> {
    validate_request(&request)?;

    let mut jobs = JoinSet::new();
    for (index, session_id) in request.session_ids.iter().cloned().enumerate() {
        let iii = deps.iii.clone();
        jobs.spawn(async move {
            let (meta, status, metrics) = tokio::join!(
                trigger::<_, Option<SessionGetResponseV1>>(
                    &iii,
                    "session::get",
                    json!({ "session_id": session_id.clone() }),
                ),
                trigger::<_, Option<StatusReport>>(
                    &iii,
                    "harness::status",
                    StatusRequest {
                        session_id: session_id.clone(),
                    },
                ),
                trigger::<_, SessionMetricsResponseV1>(
                    &iii,
                    "harness::metrics",
                    SessionMetricsRequestV1 {
                        root_session_id: session_id.clone(),
                    },
                ),
            );
            let meta = meta?.ok_or_else(|| EvalError::SessionNotFound(session_id.clone()))?;
            Ok::<_, EvalError>((
                index,
                SessionObservation {
                    meta: meta.meta,
                    status,
                    metrics,
                },
            ))
        });
    }

    let mut observations: Vec<Option<SessionObservation>> = std::iter::repeat_with(|| None)
        .take(request.session_ids.len())
        .collect();
    while let Some(joined) = jobs.join_next().await {
        let (index, observation) = joined.map_err(|error| {
            EvalError::Dependency(format!("session collection task failed: {error}"))
        })??;
        observations[index] = Some(observation);
    }
    let observations = observations
        .into_iter()
        .map(|observation| {
            observation.ok_or_else(|| {
                EvalError::Dependency("session collection returned no result".into())
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    for observation in &observations {
        if let Some(parent) = metadata_parent_key(&observation.meta) {
            return Err(EvalError::InvalidRequest(format!(
                "session {} is a descendant (metadata.{parent}) and only root sessions can be compared",
                observation.meta.session_id
            )));
        }
    }

    let baseline_summary = observations
        .iter()
        .find(|observation| observation.meta.session_id == request.baseline_session_id)
        .and_then(|observation| observation.metrics.as_ref().ok())
        .map(ObjectiveSummaryV1::from_metrics);

    let mut sessions = observations
        .into_iter()
        .map(|observation| item_from_observation(observation, baseline_summary.as_ref()))
        .collect::<Vec<_>>();
    // Preserve the user's explicit order, with the reference first only when
    // they sent it first; selection order remains meaningful in the matrix.
    sessions.shrink_to_fit();

    Ok(SessionComparisonResponseV1 {
        schema_version: SCHEMA_VERSION.into(),
        captured_at: ids::now_ms(),
        baseline_session_id: request.baseline_session_id,
        sessions,
    })
}

fn validate_request(request: &CompareSessionsRequestV1) -> Result<(), EvalError> {
    if !(2..=5).contains(&request.session_ids.len()) {
        return Err(EvalError::InvalidRequest(
            "session_ids must contain between 2 and 5 sessions".into(),
        ));
    }
    if request
        .session_ids
        .iter()
        .any(|session_id| session_id.trim().is_empty())
    {
        return Err(EvalError::InvalidRequest(
            "session_ids cannot contain empty values".into(),
        ));
    }
    let unique = request
        .session_ids
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    if unique.len() != request.session_ids.len() {
        return Err(EvalError::InvalidRequest(
            "session_ids must be unique".into(),
        ));
    }
    if !request
        .session_ids
        .iter()
        .any(|session_id| session_id == &request.baseline_session_id)
    {
        return Err(EvalError::InvalidRequest(
            "baseline_session_id must belong to session_ids".into(),
        ));
    }
    Ok(())
}

fn metadata_parent_key(meta: &SessionMetaProjectionV1) -> Option<&'static str> {
    meta.metadata
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|metadata| {
            metadata
                .contains_key("parent_session_id")
                .then_some("parent_session_id")
        })
}

fn item_from_observation(
    observation: SessionObservation,
    baseline: Option<&ObjectiveSummaryV1>,
) -> SessionComparisonItemV1 {
    let status_error = observation.status.as_ref().err().map(ToString::to_string);
    let status_report = observation
        .status
        .as_ref()
        .ok()
        .and_then(|status| status.as_ref());
    let metrics_error = observation.metrics.as_ref().err().map(ToString::to_string);
    let metrics = observation.metrics.ok();
    let mut errors = Vec::new();
    if let Some(error) = status_error {
        errors.push(format!("harness::status: {error}"));
    }
    if let Some(error) = metrics_error {
        errors.push(format!("harness::metrics: {error}"));
    }

    let summary = metrics.as_ref().map(ObjectiveSummaryV1::from_metrics);
    let lifecycle = lifecycle_from(
        observation.meta.status,
        status_report,
        metrics.as_ref().map(|metrics| metrics.complete),
    );
    let deltas = summary
        .as_ref()
        .map(|summary| metric_deltas(summary, baseline))
        .unwrap_or_else(null_metric_deltas);

    SessionComparisonItemV1 {
        session: observation.meta,
        lifecycle,
        metrics,
        summary,
        deltas,
        errors,
    }
}

fn lifecycle_from(
    session_status: SessionStatusV1,
    status: Option<&StatusReport>,
    complete: Option<bool>,
) -> SessionLifecycleV1 {
    let turn_status = status.map(|status| status.status);
    SessionLifecycleV1 {
        session_status,
        turn_id: status.and_then(|status| status.turn_id.clone()),
        terminal: status.map(|status| status.status.is_terminal() && !status.expects_wake),
        turn_status,
        result_error: status.and_then(|status| status.result_error.clone()),
        expects_wake: status.map(|status| status.expects_wake),
        partial: complete != Some(true),
        complete,
    }
}

impl ObjectiveSummaryV1 {
    fn from_metrics(metrics: &SessionMetricsResponseV1) -> Self {
        let totals = &metrics.totals;
        let total_tokens = totals
            .input_tokens
            .zip(totals.output_tokens)
            .map(|(input, output)| input.saturating_add(output));
        let generations = totals.turns;
        let function_error_rate = (totals.function_calls > 0)
            .then(|| totals.function_call_errors as f64 / totals.function_calls as f64);
        let tokens_per_generation = (generations > 0)
            .then(|| total_tokens.map(|tokens| tokens as f64 / generations as f64))
            .flatten();
        let cost_per_generation_usd = (generations > 0)
            .then(|| totals.cost_usd.map(|cost| cost / generations as f64))
            .flatten();
        let max_depth = metrics.by_session.iter().map(|session| session.depth).max();
        let root_context = metrics
            .by_session
            .iter()
            .find(|session| session.session_id == metrics.root_session_id)
            .and_then(|session| session.context.as_ref());
        let compacted_sessions = metrics
            .by_session
            .iter()
            .filter(|session| {
                session
                    .context
                    .as_ref()
                    .is_some_and(|context| context.compacted)
            })
            .count() as u64;
        let context_sessions = metrics
            .by_session
            .iter()
            .filter(|session| session.context.is_some())
            .count();

        Self {
            total_tokens,
            generations: Some(generations),
            input_tokens: totals.input_tokens,
            output_tokens: totals.output_tokens,
            cache_read_tokens: totals.cache_read_tokens,
            cache_write_tokens: totals.cache_write_tokens,
            reasoning_tokens: totals.reasoning_tokens,
            subject_cost_usd: totals.cost_usd,
            tokens_per_generation,
            cost_per_generation_usd,
            function_calls: Some(totals.function_calls),
            function_call_errors: Some(totals.function_call_errors),
            function_error_rate,
            trace_duration_ms: metrics.traces.as_ref().map(|traces| traces.duration_ms),
            trace_count: metrics.traces.as_ref().map(|traces| traces.trace_count),
            span_count: metrics.traces.as_ref().map(|traces| traces.span_count),
            error_span_count: metrics
                .traces
                .as_ref()
                .map(|traces| traces.error_span_count),
            sessions: Some(totals.sessions),
            descendants: Some(totals.sessions.saturating_sub(1)),
            max_depth,
            compacted_sessions: (context_sessions > 0).then_some(compacted_sessions),
            context_total_tokens: root_context.map(|context| context.total),
            context_usable_tokens: root_context.map(|context| context.usable),
            context_free_tokens: root_context.map(|context| context.free),
            context_occupancy: root_context.and_then(|context| {
                (context.usable > 0).then(|| context.total as f64 / context.usable as f64)
            }),
        }
    }

    fn values(&self) -> BTreeMap<String, Option<f64>> {
        BTreeMap::from([
            ("total_tokens".into(), self.total_tokens.map(|v| v as f64)),
            ("generations".into(), self.generations.map(|v| v as f64)),
            ("input_tokens".into(), self.input_tokens.map(|v| v as f64)),
            ("output_tokens".into(), self.output_tokens.map(|v| v as f64)),
            (
                "cache_read_tokens".into(),
                self.cache_read_tokens.map(|v| v as f64),
            ),
            (
                "cache_write_tokens".into(),
                self.cache_write_tokens.map(|v| v as f64),
            ),
            (
                "reasoning_tokens".into(),
                self.reasoning_tokens.map(|v| v as f64),
            ),
            ("subject_cost_usd".into(), self.subject_cost_usd),
            ("tokens_per_generation".into(), self.tokens_per_generation),
            (
                "cost_per_generation_usd".into(),
                self.cost_per_generation_usd,
            ),
            (
                "function_calls".into(),
                self.function_calls.map(|v| v as f64),
            ),
            (
                "function_call_errors".into(),
                self.function_call_errors.map(|v| v as f64),
            ),
            ("function_error_rate".into(), self.function_error_rate),
            (
                "trace_duration_ms".into(),
                self.trace_duration_ms.map(|v| v as f64),
            ),
            ("trace_count".into(), self.trace_count.map(|v| v as f64)),
            ("span_count".into(), self.span_count.map(|v| v as f64)),
            (
                "error_span_count".into(),
                self.error_span_count.map(|v| v as f64),
            ),
            ("sessions".into(), self.sessions.map(|v| v as f64)),
            ("descendants".into(), self.descendants.map(|v| v as f64)),
            ("max_depth".into(), self.max_depth.map(|v| v as f64)),
            (
                "compacted_sessions".into(),
                self.compacted_sessions.map(|v| v as f64),
            ),
            (
                "context_total_tokens".into(),
                self.context_total_tokens.map(|v| v as f64),
            ),
            (
                "context_usable_tokens".into(),
                self.context_usable_tokens.map(|v| v as f64),
            ),
            (
                "context_free_tokens".into(),
                self.context_free_tokens.map(|v| v as f64),
            ),
            ("context_occupancy".into(), self.context_occupancy),
        ])
    }
}

fn metric_deltas(
    summary: &ObjectiveSummaryV1,
    baseline: Option<&ObjectiveSummaryV1>,
) -> BTreeMap<String, MetricDeltaV1> {
    let current = summary.values();
    let baseline = baseline.map(ObjectiveSummaryV1::values);
    current
        .into_iter()
        .map(|(name, value)| {
            let baseline_value = baseline
                .as_ref()
                .and_then(|values| values.get(&name).copied())
                .flatten();
            let absolute = value
                .zip(baseline_value)
                .map(|(value, baseline)| value - baseline);
            let percent = absolute
                .zip(baseline_value)
                .filter(|(_, baseline)| *baseline != 0.0)
                .map(|(absolute, baseline)| absolute / baseline * 100.0);
            (name, MetricDeltaV1 { absolute, percent })
        })
        .collect()
}

fn null_metric_deltas() -> BTreeMap<String, MetricDeltaV1> {
    ObjectiveSummaryV1::default()
        .values()
        .into_keys()
        .map(|name| (name, MetricDeltaV1::default()))
        .collect()
}

async fn trigger<I, O>(iii: &Arc<IIIClient>, function_id: &str, input: I) -> Result<O, EvalError>
where
    I: Serialize,
    O: DeserializeOwned,
{
    let request = TriggerRequest {
        function_id: function_id.into(),
        payload: serde_json::to_value(input)?,
        action: None,
        timeout_ms: Some(COLLECTION_TIMEOUT_MS),
    };
    let value = iii
        .trigger(request)
        .await
        .map_err(|error| EvalError::Dependency(format!("{function_id} failed: {error}")))?;
    serde_json::from_value(value)
        .map_err(|error| EvalError::Serialization(format!("{function_id} response: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness::functions::metrics::{
        SessionMetricsResponseV1, SessionUsageTotalsV1, SessionUsageV1,
    };

    fn metrics(input: Option<u64>, output: Option<u64>, turns: u64) -> SessionMetricsResponseV1 {
        SessionMetricsResponseV1 {
            root_session_id: "root".into(),
            complete: true,
            totals: SessionUsageTotalsV1 {
                sessions: 1,
                turns,
                input_tokens: input,
                output_tokens: output,
                ..SessionUsageTotalsV1::default()
            },
            by_session: vec![SessionUsageV1 {
                session_id: "root".into(),
                parent_session_id: None,
                depth: 0,
                turns,
                function_calls: 0,
                function_call_errors: 0,
                input_tokens: input,
                output_tokens: output,
                cache_read_tokens: None,
                cache_write_tokens: None,
                reasoning_tokens: None,
                cost_usd: None,
                context: None,
            }],
            traces: None,
        }
    }

    #[test]
    fn validates_shape_and_baseline() {
        let base = CompareSessionsRequestV1 {
            session_ids: vec!["a".into(), "a".into()],
            baseline_session_id: "a".into(),
        };
        assert!(validate_request(&base).is_err());
        let base = CompareSessionsRequestV1 {
            session_ids: vec!["a".into(), "b".into()],
            baseline_session_id: "c".into(),
        };
        assert!(validate_request(&base).is_err());
        for count in [0, 1, 6] {
            let request = CompareSessionsRequestV1 {
                session_ids: (0..count).map(|index| format!("session-{index}")).collect(),
                baseline_session_id: "session-0".into(),
            };
            assert!(validate_request(&request).is_err());
        }
    }

    #[test]
    fn identifies_descendants_by_metadata_without_rejecting_forks() {
        let root = SessionMetaProjectionV1 {
            session_id: "root".into(),
            metadata: Some(json!({ "forked": true })),
            ..SessionMetaProjectionV1::default()
        };
        let child = SessionMetaProjectionV1 {
            session_id: "child".into(),
            metadata: Some(json!({ "parent_session_id": "root" })),
            ..SessionMetaProjectionV1::default()
        };
        assert_eq!(metadata_parent_key(&root), None);
        assert_eq!(metadata_parent_key(&child), Some("parent_session_id"));
    }

    #[test]
    fn derives_totals_and_nulls_missing_usage() {
        let complete = ObjectiveSummaryV1::from_metrics(&metrics(Some(10), Some(2), 2));
        assert_eq!(complete.total_tokens, Some(12));
        assert_eq!(complete.tokens_per_generation, Some(6.0));
        let partial = ObjectiveSummaryV1::from_metrics(&metrics(Some(10), None, 2));
        assert_eq!(partial.total_tokens, None);
        assert_eq!(partial.tokens_per_generation, None);
    }

    #[test]
    fn percentage_delta_is_null_for_missing_or_zero_baseline() {
        let current = ObjectiveSummaryV1 {
            total_tokens: Some(12),
            ..ObjectiveSummaryV1::default()
        };
        let zero = ObjectiveSummaryV1 {
            total_tokens: Some(0),
            ..ObjectiveSummaryV1::default()
        };
        let delta = metric_deltas(&current, Some(&zero));
        assert_eq!(delta["total_tokens"].absolute, Some(12.0));
        assert_eq!(delta["total_tokens"].percent, None);
        let missing = metric_deltas(&current, None);
        assert_eq!(missing["total_tokens"].absolute, None);
        assert_eq!(missing["total_tokens"].percent, None);
        assert_eq!(null_metric_deltas().len(), current.values().len());
    }
}
