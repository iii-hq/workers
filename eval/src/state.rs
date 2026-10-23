//! Durable records on the shared `state::*` functions. Compact records and
//! their evidence assets live in separate scopes so `list` and the sweep never
//! carry transcripts in one engine frame.

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::contract::{
    AdmissionCheckpointV1, AdmissionV1, AnalysisAssetsV1, AnalysisRecordV1, CycleRecordV1,
    MonitorConfigV1,
};
use crate::error::EvalError;

pub const CONFIG_SCOPE: &str = "eval_monitor";
pub const CONFIG_KEY: &str = "config";
pub const DAILY_SPEND_KEY: &str = "daily_spend";
pub const ANALYSIS_SCOPE: &str = "eval_analysis";
pub const ASSETS_SCOPE: &str = "eval_analysis_assets";
/// The Harness's own turn records, keyed by session: only the latest turn.
pub const HARNESS_TURN_SCOPE: &str = "harness_turn";
/// The loop's own cycle records, keyed by `cycle_id`. Cycle functions read
/// and write only this scope, never the monitor's scopes above.
pub const CYCLE_SCOPE: &str = "eval_loop_cycle";
/// The loop's admission claims, keyed by admission key, and the sweep
/// checkpoint. Kept apart from the cycles so cycle listings carry neither.
pub const ADMISSION_SCOPE: &str = "eval_loop_admission";
pub const ADMISSION_CHECKPOINT_KEY: &str = "checkpoint";
const DISPATCH_TIMEOUT_MS: u64 = 10_000;

/// A stored configuration that no longer parses under the current schema (a
/// legacy record with the removed chat-monitor fields) stays inert: it is
/// ignored, as if never configured, and never migrated or read twice.
pub async fn get_config(iii: &IIIClient) -> Result<Option<MonitorConfigV1>, EvalError> {
    let Some(stored): Option<Value> = get(iii, CONFIG_SCOPE, CONFIG_KEY).await? else {
        return Ok(None);
    };
    match serde_json::from_value(stored) {
        Ok(config) => Ok(Some(config)),
        Err(error) => {
            tracing::warn!(%error, "ignoring unreadable legacy eval_monitor/config");
            Ok(None)
        }
    }
}

pub async fn put_config(iii: &IIIClient, config: &MonitorConfigV1) -> Result<(), EvalError> {
    set(iii, CONFIG_SCOPE, CONFIG_KEY, config).await
}

/// What the monitor spent on the UTC day starting at `since`, kept apart from
/// the analyses: deleting one, or retention, must not give the budget back.
/// `usd` is the capture spend, the one the daily cap compares.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DailySpendV1 {
    pub since: i64,
    /// Capture: what an analysis spends. The key predates the buckets, when it
    /// held everything, so a day stored then counts as capture (conservative).
    pub usd: f64,
}

pub async fn get_spend(iii: &IIIClient) -> Result<Option<DailySpendV1>, EvalError> {
    get(iii, CONFIG_SCOPE, DAILY_SPEND_KEY).await
}

pub async fn put_spend(iii: &IIIClient, spend: &DailySpendV1) -> Result<(), EvalError> {
    set(iii, CONFIG_SCOPE, DAILY_SPEND_KEY, spend).await
}

pub async fn get_record(
    iii: &IIIClient,
    evaluation_id: &str,
) -> Result<Option<AnalysisRecordV1>, EvalError> {
    get(iii, ANALYSIS_SCOPE, evaluation_id).await
}

pub async fn put_record(iii: &IIIClient, record: &AnalysisRecordV1) -> Result<(), EvalError> {
    set(iii, ANALYSIS_SCOPE, &record.evaluation_id, record).await
}

pub async fn list_records(iii: &IIIClient) -> Result<Vec<AnalysisRecordV1>, EvalError> {
    list(iii, ANALYSIS_SCOPE).await
}

pub async fn get_assets(
    iii: &IIIClient,
    evaluation_id: &str,
) -> Result<AnalysisAssetsV1, EvalError> {
    Ok(get(iii, ASSETS_SCOPE, evaluation_id)
        .await?
        .unwrap_or_else(|| AnalysisAssetsV1 {
            evaluation_id: evaluation_id.into(),
            ..AnalysisAssetsV1::default()
        }))
}

pub async fn put_assets(iii: &IIIClient, assets: &AnalysisAssetsV1) -> Result<(), EvalError> {
    set(iii, ASSETS_SCOPE, &assets.evaluation_id, assets).await
}

pub async fn get_cycle(
    iii: &IIIClient,
    cycle_id: &str,
) -> Result<Option<CycleRecordV1>, EvalError> {
    get(iii, CYCLE_SCOPE, cycle_id).await
}

pub async fn put_cycle(iii: &IIIClient, record: &CycleRecordV1) -> Result<(), EvalError> {
    set(iii, CYCLE_SCOPE, &record.cycle_id, record).await
}

pub async fn list_cycles(iii: &IIIClient) -> Result<Vec<CycleRecordV1>, EvalError> {
    list(iii, CYCLE_SCOPE).await
}

/// Claims `claim.admission_key` only when absent. Returns the stored claim and
/// whether this call set it.
pub async fn claim_admission(
    iii: &IIIClient,
    claim: &AdmissionV1,
) -> Result<(AdmissionV1, bool), EvalError> {
    let value = serde_json::to_value(claim)?;
    let response = call(
        iii,
        "state::compare-and-set",
        json!({ "scope": ADMISSION_SCOPE, "key": claim.admission_key, "value": value }),
    )
    .await?;
    if response["swapped"].as_bool() == Some(true) {
        return Ok((claim.clone(), true));
    }
    let current: Option<AdmissionV1> = match &response["current"] {
        Value::Null => get(iii, ADMISSION_SCOPE, &claim.admission_key).await?,
        current => Some(serde_json::from_value(current.clone()).map_err(|error| {
            EvalError::State(format!(
                "parse {ADMISSION_SCOPE}/{}: {error}",
                claim.admission_key
            ))
        })?),
    };
    current.map(|stored| (stored, false)).ok_or_else(|| {
        EvalError::State(format!(
            "compare-and-set {ADMISSION_SCOPE}/{} neither swapped nor stored",
            claim.admission_key
        ))
    })
}

pub async fn get_admission_checkpoint(
    iii: &IIIClient,
) -> Result<Option<AdmissionCheckpointV1>, EvalError> {
    get(iii, ADMISSION_SCOPE, ADMISSION_CHECKPOINT_KEY).await
}

pub async fn put_admission_checkpoint(
    iii: &IIIClient,
    checkpoint: &AdmissionCheckpointV1,
) -> Result<(), EvalError> {
    set(iii, ADMISSION_SCOPE, ADMISSION_CHECKPOINT_KEY, checkpoint).await
}

/// The Harness's record of a session's latest turn, with its content-addressed
/// prompt bodies restored.
pub async fn get_turn_record(
    iii: &IIIClient,
    session_id: &str,
) -> Result<Option<Value>, EvalError> {
    let Some(mut stored): Option<Value> = get(iii, HARNESS_TURN_SCOPE, session_id).await? else {
        return Ok(None);
    };
    let Some(hydrated) = harness::state::get_turn(iii, session_id, DISPATCH_TIMEOUT_MS)
        .await
        .map_err(|error| EvalError::State(format!("read harness turn {session_id}: {error}")))?
    else {
        return Ok(None);
    };
    if stored["turn_id"].as_str() != Some(hydrated.turn_id.as_str()) {
        return Ok(None);
    }
    if let Some(prompt) = hydrated.options.system_prompt {
        stored["options"]["system_prompt"] = Value::String(prompt);
    }
    if let Some(baseline) = hydrated
        .options
        .skill_context
        .and_then(|context| context.baseline)
    {
        stored["options"]["skill_context"]["baseline"] = Value::String(baseline);
    }
    Ok(Some(stored))
}

/// Removes the assets before the record, so a partial delete never leaves a
/// listed record without its evidence.
pub async fn delete_analysis(iii: &IIIClient, evaluation_id: &str) -> Result<(), EvalError> {
    delete(iii, ASSETS_SCOPE, evaluation_id).await?;
    delete(iii, ANALYSIS_SCOPE, evaluation_id).await
}

async fn get<T: DeserializeOwned>(
    iii: &IIIClient,
    scope: &str,
    key: &str,
) -> Result<Option<T>, EvalError> {
    let value = call(iii, "state::get", json!({ "scope": scope, "key": key })).await?;
    if value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(|error| EvalError::State(format!("parse {scope}/{key}: {error}")))
}

async fn set(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    value: &impl Serialize,
) -> Result<(), EvalError> {
    let value = serde_json::to_value(value)?;
    call(
        iii,
        "state::set",
        json!({ "scope": scope, "key": key, "value": value }),
    )
    .await
    .map(|_| ())
}

async fn delete(iii: &IIIClient, scope: &str, key: &str) -> Result<(), EvalError> {
    call(iii, "state::delete", json!({ "scope": scope, "key": key }))
        .await
        .map(|_| ())
}

/// `state::list` returns the stored values; unreadable rows are logged and
/// skipped rather than failing every listing.
async fn list<T: DeserializeOwned>(iii: &IIIClient, scope: &str) -> Result<Vec<T>, EvalError> {
    let value = call(iii, "state::list", json!({ "scope": scope })).await?;
    let rows: Vec<Value> = match value {
        Value::Array(items) => items,
        Value::Object(mut map) => match map.remove("values").or_else(|| map.remove("items")) {
            Some(Value::Array(items)) => items,
            _ => map.into_iter().map(|(_, value)| value).collect(),
        },
        _ => Vec::new(),
    };
    Ok(rows
        .into_iter()
        .filter_map(|row| match serde_json::from_value(row) {
            Ok(parsed) => Some(parsed),
            Err(error) => {
                tracing::warn!(scope, %error, "skipping unreadable eval state row");
                None
            }
        })
        .collect())
}

async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value, EvalError> {
    let scope = payload["scope"].as_str().unwrap_or_default().to_string();
    iii.trigger(TriggerRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(DISPATCH_TIMEOUT_MS),
    })
    .await
    .map_err(|error| EvalError::State(format!("{function_id} {scope}: {error}")))
}
