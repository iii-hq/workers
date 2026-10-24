//! Durable loop records on the shared `state::*` functions, in the loop's
//! own scopes only. Records of earlier eval versions live in other scopes
//! and are never read.

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

use crate::contract::{
    AdmissionCheckpointV1, AdmissionV1, CycleLedgerV1, CycleRecordV1, ExecutionEvidenceV1,
    StoredConfigV1,
};
use crate::decision;
use crate::error::EvalError;

pub const CONFIG_SCOPE: &str = "eval_cycle_config";
pub const CONFIG_KEY: &str = "current";
/// Cycle records, keyed by `cycle_id`.
pub const CYCLE_SCOPE: &str = "eval_cycle";
/// Admission claims keyed by admission key, and the sweep checkpoint.
pub const ADMISSION_SCOPE: &str = "eval_cycle_admission";
pub const ADMISSION_CHECKPOINT_KEY: &str = "checkpoint";
/// Imported native report bundles, keyed by admission key.
pub const EVIDENCE_SCOPE: &str = "eval_cycle_evidence";
/// Budget ledgers, keyed by `cycle_id`.
pub const BUDGET_SCOPE: &str = "eval_cycle_budget";
/// Open (non-terminal) cycles, keyed by `cycle_id`: what the sweep and the
/// admission read, never every cycle ever kept.
pub const OPEN_SCOPE: &str = "eval_cycle_open";
/// An open entry is written just before its cycle is created: without a
/// cycle it is kept this long, then dropped.
const OPEN_ORPHAN_MS: i64 = 10 * 60 * 1000;
const CAS_ATTEMPTS: usize = 8;
const DISPATCH_TIMEOUT_MS: u64 = 10_000;

pub async fn get_config(iii: &IIIClient) -> Result<Option<StoredConfigV1>, EvalError> {
    get(iii, CONFIG_SCOPE, CONFIG_KEY).await
}

pub async fn put_config(iii: &IIIClient, config: &StoredConfigV1) -> Result<(), EvalError> {
    set(iii, CONFIG_SCOPE, CONFIG_KEY, config).await
}

pub async fn get_cycle(
    iii: &IIIClient,
    cycle_id: &str,
) -> Result<Option<CycleRecordV1>, EvalError> {
    get(iii, CYCLE_SCOPE, cycle_id).await
}

pub async fn list_cycles(iii: &IIIClient) -> Result<Vec<CycleRecordV1>, EvalError> {
    list(iii, CYCLE_SCOPE).await
}

#[derive(serde::Deserialize, Serialize)]
struct OpenCycle {
    cycle_id: String,
    at: i64,
}

/// Every open cycle, from the open index. An entry whose cycle ended (or
/// was never created) is cleared here, so a lost clear heals itself.
pub async fn list_open_cycles(iii: &IIIClient) -> Result<Vec<CycleRecordV1>, EvalError> {
    let now = crate::ids::now_ms();
    let mut open = Vec::new();
    for entry in list::<OpenCycle>(iii, OPEN_SCOPE).await? {
        match get_cycle(iii, &entry.cycle_id).await? {
            Some(cycle) if !cycle.status.is_terminal() => open.push(cycle),
            None if now - entry.at < OPEN_ORPHAN_MS => {}
            _ => delete(iii, OPEN_SCOPE, &entry.cycle_id).await?,
        }
    }
    Ok(open)
}

/// Creates a cycle that must not exist yet, indexed as open first so it is
/// never missing from the sweep.
pub async fn create_cycle(iii: &IIIClient, record: &CycleRecordV1) -> Result<bool, EvalError> {
    let entry = OpenCycle {
        cycle_id: record.cycle_id.clone(),
        at: crate::ids::now_ms(),
    };
    set(iii, OPEN_SCOPE, &record.cycle_id, &entry).await?;
    cas(iii, CYCLE_SCOPE, &record.cycle_id, None, record).await
}

/// Applies `apply` to the stored cycle and writes the result with
/// compare-and-set against exactly what was read, after
/// [`decision::check_write`]: a frozen plan or a recorded decision never
/// changes, and a concurrent write (cancel, admission) is never lost.
/// `None` from `apply` writes nothing.
pub async fn update_cycle<T>(
    iii: &IIIClient,
    cycle_id: &str,
    mut apply: impl FnMut(&CycleRecordV1) -> Result<(Option<CycleRecordV1>, T), EvalError>,
) -> Result<Option<T>, EvalError> {
    for _ in 0..CAS_ATTEMPTS {
        let Some(raw): Option<Value> = get(iii, CYCLE_SCOPE, cycle_id).await? else {
            return Ok(None);
        };
        let stored: CycleRecordV1 = parse(CYCLE_SCOPE, cycle_id, raw.clone())?;
        let (next, out) = apply(&stored)?;
        let Some(next) = next else {
            return Ok(Some(out));
        };
        decision::check_write(Some(&stored), &next)?;
        if cas(iii, CYCLE_SCOPE, cycle_id, Some(raw), &next).await? {
            if next.status.is_terminal() && !stored.status.is_terminal() {
                // Best effort: `list_open_cycles` clears what this misses.
                if let Err(error) = delete(iii, OPEN_SCOPE, cycle_id).await {
                    tracing::warn!(%cycle_id, %error, "could not clear an ended cycle's open entry");
                }
            }
            return Ok(Some(out));
        }
    }
    Err(EvalError::Conflict(format!(
        "{CYCLE_SCOPE}/{cycle_id} kept changing under {CAS_ATTEMPTS} attempts"
    )))
}

/// Claims `claim.admission_key` only when absent. Returns the stored claim
/// and whether this call set it.
pub async fn claim_admission(
    iii: &IIIClient,
    claim: &AdmissionV1,
) -> Result<(AdmissionV1, bool), EvalError> {
    if cas(iii, ADMISSION_SCOPE, &claim.admission_key, None, claim).await? {
        return Ok((claim.clone(), true));
    }
    get(iii, ADMISSION_SCOPE, &claim.admission_key)
        .await?
        .map(|stored| (stored, false))
        .ok_or_else(|| {
            EvalError::State(format!(
                "compare-and-set {ADMISSION_SCOPE}/{} neither swapped nor stored",
                claim.admission_key
            ))
        })
}

pub async fn get_admission(
    iii: &IIIClient,
    admission_key: &str,
) -> Result<Option<AdmissionV1>, EvalError> {
    get(iii, ADMISSION_SCOPE, admission_key).await
}

pub async fn get_ledger(
    iii: &IIIClient,
    cycle_id: &str,
) -> Result<Option<CycleLedgerV1>, EvalError> {
    get(iii, BUDGET_SCOPE, cycle_id).await
}

pub async fn list_ledgers(iii: &IIIClient) -> Result<Vec<CycleLedgerV1>, EvalError> {
    list(iii, BUDGET_SCOPE).await
}

/// Compare-and-set update of a cycle's ledger, retried on a concurrent
/// write. `None` writes nothing.
pub async fn update_ledger<T>(
    iii: &IIIClient,
    cycle_id: &str,
    mut apply: impl FnMut(Option<CycleLedgerV1>) -> Result<(Option<CycleLedgerV1>, T), EvalError>,
) -> Result<T, EvalError> {
    for _ in 0..CAS_ATTEMPTS {
        let raw: Option<Value> = get(iii, BUDGET_SCOPE, cycle_id).await?;
        let current = raw
            .clone()
            .map(|raw| parse(BUDGET_SCOPE, cycle_id, raw))
            .transpose()?;
        let (next, out) = apply(current)?;
        let Some(next) = next else {
            return Ok(out);
        };
        if cas(iii, BUDGET_SCOPE, cycle_id, raw, &next).await? {
            return Ok(out);
        }
    }
    Err(EvalError::Conflict(format!(
        "{BUDGET_SCOPE}/{cycle_id} kept changing under {CAS_ATTEMPTS} attempts"
    )))
}

pub async fn get_evidence(
    iii: &IIIClient,
    admission_key: &str,
) -> Result<Option<ExecutionEvidenceV1>, EvalError> {
    get(iii, EVIDENCE_SCOPE, admission_key).await
}

/// Stores the first copy only: a re-import keeps it.
pub async fn put_evidence_once(
    iii: &IIIClient,
    evidence: &ExecutionEvidenceV1,
) -> Result<(), EvalError> {
    cas(iii, EVIDENCE_SCOPE, &evidence.admission_key, None, evidence)
        .await
        .map(|_| ())
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

fn parse<T: DeserializeOwned>(scope: &str, key: &str, value: Value) -> Result<T, EvalError> {
    serde_json::from_value(value)
        .map_err(|error| EvalError::State(format!("parse {scope}/{key}: {error}")))
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
    parse(scope, key, value).map(Some)
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

/// `state::compare-and-set` against `expected` (`None`: only when absent).
async fn cas(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    expected: Option<Value>,
    value: &impl Serialize,
) -> Result<bool, EvalError> {
    let mut payload = json!({ "scope": scope, "key": key, "value": serde_json::to_value(value)? });
    if let Some(expected) = expected {
        payload["expected"] = expected;
    }
    let response = call(iii, "state::compare-and-set", payload).await?;
    Ok(response["swapped"].as_bool() == Some(true))
}

/// `state::list` values; unreadable rows are logged and skipped.
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
