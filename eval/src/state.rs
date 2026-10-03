//! Durable records on the shared `state::*` functions. Compact records and
//! their evidence assets live in separate scopes so `list` and the sweep never
//! carry transcripts in one engine frame.

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::contract::{AnalysisAssetsV1, AnalysisRecordV1, CapacityRejectionV1, MonitorConfigV1};
use crate::error::EvalError;

pub const CONFIG_SCOPE: &str = "eval_monitor";
pub const CONFIG_KEY: &str = "config";
pub const LAST_REJECTION_KEY: &str = "last_rejection";
pub const OBSERVATION_SCOPE: &str = "eval_observation";
pub const ANALYSIS_SCOPE: &str = "eval_analysis";
pub const ASSETS_SCOPE: &str = "eval_analysis_assets";
const DISPATCH_TIMEOUT_MS: u64 = 10_000;

/// Marks one observed session turn as admitted. It outlives a deleted
/// analysis until retention, so a redelivered event cannot re-admit it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationIndexV1 {
    pub observation_key: String,
    pub session_id: String,
    pub turn_id: String,
    pub evaluation_id: String,
    pub admitted_at: i64,
}

pub async fn get_config(iii: &IIIClient) -> Result<Option<MonitorConfigV1>, EvalError> {
    get(iii, CONFIG_SCOPE, CONFIG_KEY).await
}

pub async fn put_config(iii: &IIIClient, config: &MonitorConfigV1) -> Result<(), EvalError> {
    set(iii, CONFIG_SCOPE, CONFIG_KEY, config).await
}

pub async fn get_last_rejection(iii: &IIIClient) -> Result<Option<CapacityRejectionV1>, EvalError> {
    get(iii, CONFIG_SCOPE, LAST_REJECTION_KEY).await
}

pub async fn put_last_rejection(
    iii: &IIIClient,
    rejection: &CapacityRejectionV1,
) -> Result<(), EvalError> {
    set(iii, CONFIG_SCOPE, LAST_REJECTION_KEY, rejection).await
}

pub async fn get_observation(
    iii: &IIIClient,
    key: &str,
) -> Result<Option<ObservationIndexV1>, EvalError> {
    get(iii, OBSERVATION_SCOPE, key).await
}

pub async fn put_observation(iii: &IIIClient, index: &ObservationIndexV1) -> Result<(), EvalError> {
    set(iii, OBSERVATION_SCOPE, &index.observation_key, index).await
}

pub async fn list_observations(iii: &IIIClient) -> Result<Vec<ObservationIndexV1>, EvalError> {
    list(iii, OBSERVATION_SCOPE).await
}

pub async fn delete_observation(iii: &IIIClient, key: &str) -> Result<(), EvalError> {
    delete(iii, OBSERVATION_SCOPE, key).await
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
