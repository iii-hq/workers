//! `memory-consolidate::*` — the manual run surface, status, and the
//! schedule bookkeeping (last run + last report persisted in the state
//! worker so catch-up-on-boot survives restarts).

use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::WorkerConfig;
use crate::consolidate::{self, BankReport};

pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const RUN_ID: &str = "memory-consolidate::run";
pub const RUN_DESC: &str =
    "Run one consolidation pass now (all configured banks, or one bank). Deterministic \
     near-duplicate dedup applied strictly through memory::supersede + memory::save; pinned \
     memories are untouchable. dry_run plans without writing.";

pub const STATUS_ID: &str = "memory-consolidate::status";
pub const STATUS_DESC: &str =
    "Schedule and last-pass report: enabled, interval, last run, whether a pass is due, and \
     the most recent per-bank results.";

pub const TICK_ID: &str = "memory-consolidate::on-tick";
pub const TICK_DESC: &str =
    "Internal: schedule heartbeat (cron trigger + boot catch-up backstop). Runs a pass only \
     when interval_hours have elapsed since the last one.";

const STATE_SCOPE: &str = "memory_consolidate";
const STATE_LAST_RUN: &str = "last_run";
const STATE_LAST_REPORT: &str = "last_report";

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct RunRequest {
    /// Consolidate only this bank (must be in the configured allowlist,
    /// when one is set).
    #[serde(default)]
    pub bank: Option<String>,
    /// Override the configured dry_run for this call.
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RunResponse {
    pub dry_run: bool,
    pub banks: Vec<BankReport>,
    /// Total supersedes applied across banks this pass.
    pub superseded: usize,
    /// The pass completed cleanly AND its last-run checkpoint persisted.
    /// False = the schedule will retry the pass next check.
    pub checkpointed: bool,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct StatusRequest {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StatusResponse {
    pub enabled: bool,
    pub interval_hours: u64,
    pub dry_run: bool,
    /// Milliseconds since epoch of the last completed pass; 0 = never.
    pub last_run: u64,
    /// A scheduled pass is overdue right now.
    pub due: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_report: Option<Value>,
}

pub struct Deps {
    pub iii: Arc<IIIClient>,
    pub config: ConfigCell,
    /// Serializes passes: the cron heartbeat and the catch-up backstop can
    /// fire close together; the gate plus a due re-check under it makes
    /// that a no-op instead of a double run.
    pub run_gate: tokio::sync::Mutex<()>,
}

impl Deps {
    pub async fn config(&self) -> Arc<WorkerConfig> {
        self.config.read().await.clone()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

async fn state_get(iii: &IIIClient, key: &str) -> Option<Value> {
    iii.trigger(TriggerRequest {
        function_id: "state::get".into(),
        payload: json!({ "scope": STATE_SCOPE, "key": key }),
        action: None,
        timeout_ms: Some(5_000),
    })
    .await
    .ok()
    .map(|v| v.get("value").cloned().unwrap_or(v))
    .filter(|v| !v.is_null())
}

async fn state_set(iii: &IIIClient, key: &str, value: Value) -> Result<(), String> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".into(),
        payload: json!({ "scope": STATE_SCOPE, "key": key, "value": value }),
        action: None,
        timeout_ms: Some(5_000),
    })
    .await
    .map(|_| ())
    .map_err(|e| format!("state::set {key}: {e}"))
}

pub async fn last_run_ms(iii: &IIIClient) -> u64 {
    state_get(iii, STATE_LAST_RUN)
        .await
        .and_then(|v| v.get("ms").and_then(Value::as_u64))
        .unwrap_or(0)
}

/// Run one full pass and persist schedule state (the public
/// `memory-consolidate::run`). Serialized behind the run gate with the
/// scheduled tick: a manual run and a heartbeat pass must never
/// interleave their writes.
pub async fn run(deps: &Deps, req: RunRequest) -> Result<RunResponse, Error> {
    let _gate = deps.run_gate.lock().await;
    run_locked(deps, req).await
}

/// Pass body; caller MUST hold the run gate (tokio mutexes are not
/// reentrant, so gate-holding paths call this directly).
async fn run_locked(deps: &Deps, req: RunRequest) -> Result<RunResponse, Error> {
    let cfg = deps.config().await;
    let dry_run = req.dry_run.unwrap_or(cfg.dry_run);
    let banks = match &req.bank {
        Some(bank) => {
            if !cfg.banks.is_empty() && !cfg.banks.iter().any(|b| b == bank) {
                return Err(Error::Handler(format!(
                    "bank `{bank}` is outside the configured allowlist"
                )));
            }
            vec![bank.clone()]
        }
        None => consolidate::list_banks(&deps.iii, &cfg.banks)
            .await
            .map_err(Error::Handler)?,
    };

    let mut budget = cfg.max_supersedes_per_run;
    let mut reports = Vec::new();
    for bank in &banks {
        reports.push(consolidate::run_bank(&deps.iii, bank, dry_run, &mut budget).await);
    }
    let superseded = reports.iter().map(|r| r.superseded).sum();

    // The report is best-effort telemetry; the CHECKPOINT is not. Only a
    // clean pass (no bank errors) advances last_run — an errored or
    // checkpoint-failed pass stays due, so the schedule retries it
    // instead of silently waiting out a full interval. Dry runs count:
    // the operator chose planning mode.
    let report_json = serde_json::to_value(&reports).unwrap_or(Value::Null);
    if let Err(e) = state_set(
        &deps.iii,
        STATE_LAST_REPORT,
        json!({ "dry_run": dry_run, "banks": report_json }),
    )
    .await
    {
        tracing::warn!(error = %e, "last-report write failed");
    }
    let clean = reports.iter().all(|r| r.errors.is_empty());
    let checkpointed = if clean {
        match state_set(&deps.iii, STATE_LAST_RUN, json!({ "ms": now_ms() })).await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(error = %e, "last-run checkpoint failed; pass stays due");
                false
            }
        }
    } else {
        tracing::warn!("pass finished with bank errors; not checkpointed, stays due");
        false
    };

    tracing::info!(
        banks = banks.len(),
        superseded,
        dry_run,
        checkpointed,
        "consolidation pass complete"
    );
    Ok(RunResponse {
        dry_run,
        banks: reports,
        superseded,
        checkpointed,
    })
}

pub async fn status(deps: &Deps, _req: StatusRequest) -> Result<StatusResponse, Error> {
    let cfg = deps.config().await;
    let last_run = last_run_ms(&deps.iii).await;
    let due = cfg.enabled
        && now_ms().saturating_sub(last_run) >= cfg.interval_hours.saturating_mul(3_600_000);
    Ok(StatusResponse {
        enabled: cfg.enabled,
        interval_hours: cfg.interval_hours,
        dry_run: cfg.dry_run,
        last_run,
        due,
        last_report: state_get(&deps.iii, STATE_LAST_REPORT).await,
    })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TickResponse {
    /// A pass actually ran (false = not due yet or disabled).
    pub ran: bool,
}

/// Heartbeat body shared by the cron trigger and the backstop loop: run a
/// pass only when due, serialized behind the run gate.
pub async fn tick(deps: &Deps) -> Result<TickResponse, Error> {
    let cfg = deps.config().await;
    if !cfg.enabled {
        return Ok(TickResponse { ran: false });
    }
    let _gate = deps.run_gate.lock().await;
    let last = last_run_ms(&deps.iii).await;
    let interval_ms = cfg.interval_hours.saturating_mul(3_600_000);
    // A small slack keeps an hourly heartbeat that lands minutes early
    // from postponing the pass a whole extra hour.
    let slack_ms = 300_000u64.min(interval_ms / 10);
    if now_ms().saturating_sub(last) + slack_ms < interval_ms {
        return Ok(TickResponse { ran: false });
    }
    tracing::info!(
        last_run_ms = last,
        interval_hours = cfg.interval_hours,
        "scheduled consolidation pass due"
    );
    let res = run_locked(deps, RunRequest::default()).await?;
    tracing::info!(
        superseded = res.superseded,
        dry_run = res.dry_run,
        "scheduled pass complete"
    );
    Ok(TickResponse { ran: true })
}

/// One registered function's wire pin (mirrors the memory worker's
/// catalog pattern; golden-tested).
pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

fn spec<Req: JsonSchema, Resp: JsonSchema>(
    function_id: &'static str,
    description: &'static str,
) -> FunctionSpec {
    FunctionSpec {
        function_id,
        description,
        request_schema: schemars::schema_for!(Req),
        response_schema: schemars::schema_for!(Resp),
    }
}

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<RunRequest, RunResponse>(RUN_ID, RUN_DESC),
        spec::<StatusRequest, StatusResponse>(STATUS_ID, STATUS_DESC),
    ]
}

fn register<Req, Resp, F, Fut>(
    iii: &IIIClient,
    deps: &Arc<Deps>,
    id: &'static str,
    desc: &'static str,
    f: F,
) where
    Req: DeserializeOwned + JsonSchema + Send + 'static,
    Resp: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Deps>, Req) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<Resp, Error>> + Send + 'static,
{
    let deps = deps.clone();
    iii.register_function(
        id,
        RegisterFunction::new_async(move |req: Req| {
            let deps = deps.clone();
            let f = f.clone();
            async move { f(deps, req).await }
        })
        .description(desc),
    );
}

pub fn register_all(iii: &IIIClient, deps: &Arc<Deps>) {
    register::<RunRequest, RunResponse, _, _>(iii, deps, RUN_ID, RUN_DESC, |d, r| async move {
        run(&d, r).await
    });
    register::<StatusRequest, StatusResponse, _, _>(
        iii,
        deps,
        STATUS_ID,
        STATUS_DESC,
        |d, r| async move { status(&d, r).await },
    );
    tracing::info!("all memory-consolidate::* functions registered");
}
