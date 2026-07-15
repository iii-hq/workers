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

async fn state_set(iii: &IIIClient, key: &str, value: Value) {
    let res = iii
        .trigger(TriggerRequest {
            function_id: "state::set".into(),
            payload: json!({ "scope": STATE_SCOPE, "key": key, "value": value }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
    if let Err(e) = res {
        tracing::warn!(key, error = %e, "schedule state write failed (state worker absent?)");
    }
}

pub async fn last_run_ms(iii: &IIIClient) -> u64 {
    state_get(iii, STATE_LAST_RUN)
        .await
        .and_then(|v| v.get("ms").and_then(Value::as_u64))
        .unwrap_or(0)
}

/// Run one full pass and persist schedule state. Also the body of the
/// scheduled tick.
pub async fn run(deps: &Deps, req: RunRequest) -> Result<RunResponse, Error> {
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

    // Dry runs also count as a completed pass for the schedule: the
    // operator chose planning mode; rerunning it early adds nothing.
    state_set(&deps.iii, STATE_LAST_RUN, json!({ "ms": now_ms() })).await;
    let report_json = serde_json::to_value(&reports).unwrap_or(Value::Null);
    state_set(
        &deps.iii,
        STATE_LAST_REPORT,
        json!({ "dry_run": dry_run, "banks": report_json }),
    )
    .await;

    tracing::info!(
        banks = banks.len(),
        superseded,
        dry_run,
        "consolidation pass complete"
    );
    Ok(RunResponse {
        dry_run,
        banks: reports,
        superseded,
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
