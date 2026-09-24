//! The engine-facing side: the same traits the pipeline is written against,
//! implemented over real iii calls.
//!
//! Every outbound call this worker makes is tagged `iii.tag.hidden` so its
//! own plumbing does not clutter the trace views it exists to serve, and
//! every one of them remembers its trace id in the phantom ring — the calls
//! *are* the traces that would otherwise tick the trigger that feeds this
//! worker.

pub mod harness;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use iii_helpers::observability::{current_trace_id, run_with_baggage};
use iii_sdk::protocol::{TriggerAction, TriggerRequest};
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use crate::ingest::{ring::PhantomRing, CheckoutVersions, IngestJob, Telemetry, TraceSummary};
use crate::registry::{EngineRegistry, FunctionEntry, WorkerEntry};
use crate::service::TraceAvailability;
use crate::store::{Db, NamedRow, Statement, StepResult};
use crate::{SentinelError, WorkerConfig};

/// The tag that keeps this worker's own calls out of the trace views.
const HIDDEN_TAG: (&str, &str) = ("iii.tag.hidden", "sentinel store");
const CALL_TIMEOUT_MS: u64 = 15_000;
const QUEUE_NAME: &str = "sentinel-ingest";
/// Logs at WARN and above ride along with a trace's evidence.
const LOG_SEVERITY_MIN: i32 = 13;
const LOG_LIMIT: usize = 200;

/// Shared call machinery.
#[derive(Clone)]
pub struct Runtime {
    iii: Arc<IIIClient>,
    ring: Arc<PhantomRing>,
}

impl Runtime {
    pub fn new(iii: Arc<IIIClient>, ring: Arc<PhantomRing>) -> Self {
        Self { iii, ring }
    }

    /// Call a function with this worker's own traffic marked hidden, and the
    /// resulting trace remembered so its tick is dropped rather than ingested.
    async fn call(&self, function_id: &str, payload: Value) -> Result<Value, SentinelError> {
        let iii = self.iii.clone();
        let ring = self.ring.clone();
        let function = function_id.to_string();
        run_with_baggage(&[HIDDEN_TAG], async move {
            if let Some(trace_id) = current_trace_id() {
                ring.remember(&trace_id);
            }
            iii.trigger(TriggerRequest {
                function_id: function.clone(),
                payload,
                action: None,
                timeout_ms: Some(CALL_TIMEOUT_MS),
            })
            .await
            .map_err(|error| SentinelError::dependency(format!("{function}: {error}")))
        })
        .await
    }
}

/// `database::*`.
pub struct IiiDb {
    runtime: Runtime,
    database: String,
}

impl IiiDb {
    pub fn new(runtime: Runtime, database: impl Into<String>) -> Self {
        Self {
            runtime,
            database: database.into(),
        }
    }
}

#[async_trait]
impl Db for IiiDb {
    async fn query(&self, sql: &str, params: Vec<Value>) -> Result<Vec<NamedRow>, SentinelError> {
        let response = self
            .runtime
            .call(
                "database::query",
                json!({ "db": self.database, "sql": sql, "params": params }),
            )
            .await?;
        Ok(response
            .get("rows")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| row.as_object().cloned())
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn execute(&self, sql: &str, params: Vec<Value>) -> Result<u64, SentinelError> {
        let response = self
            .runtime
            .call(
                "database::execute",
                json!({ "db": self.database, "sql": sql, "params": params }),
            )
            .await?;
        Ok(response
            .get("affected_rows")
            .and_then(Value::as_u64)
            .unwrap_or(0))
    }

    async fn transaction(
        &self,
        statements: &[Statement],
    ) -> Result<Vec<StepResult>, SentinelError> {
        let payload = json!({
            "db": self.database,
            "statements": statements
                .iter()
                .map(|statement| json!({ "sql": statement.sql, "params": statement.params }))
                .collect::<Vec<_>>(),
        });
        let response = self.runtime.call("database::transaction", payload).await?;
        // A rolled-back transaction answers with `committed: false` rather
        // than an error, and names the step that failed.
        if response.get("committed").and_then(Value::as_bool) != Some(true) {
            let failed = response
                .get("failed_index")
                .and_then(Value::as_u64)
                .map(|index| format!(" at statement {index}"))
                .unwrap_or_default();
            let detail = response
                .get("error")
                .map(|error| error.to_string())
                .unwrap_or_else(|| "no detail".into());
            return Err(SentinelError::dependency(format!(
                "transaction rolled back{failed}: {detail}"
            )));
        }
        Ok(response
            .get("results")
            .and_then(Value::as_array)
            .map(|steps| {
                steps
                    .iter()
                    .map(|step| StepResult {
                        affected_rows: step
                            .get("affected_rows")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                        rows: step
                            .get("rows")
                            .and_then(Value::as_array)
                            .map(|rows| {
                                rows.iter()
                                    .map(|row| {
                                        row.as_array().cloned().unwrap_or_else(|| vec![row.clone()])
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }
}

/// `engine::traces::*` and `engine::logs::list`.
pub struct IiiTelemetry {
    runtime: Runtime,
}

impl IiiTelemetry {
    pub fn new(runtime: Runtime) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl Telemetry for IiiTelemetry {
    async fn traces(&self, trace_ids: &[String]) -> Result<Vec<TraceSummary>, SentinelError> {
        let response = self
            .runtime
            .call(
                "engine::traces::list",
                json!({ "trace_ids": trace_ids, "limit": trace_ids.len().max(1) }),
            )
            .await?;
        Ok(response
            .get("traces")
            .and_then(Value::as_array)
            .map(|traces| traces.iter().map(summary_of).collect())
            .unwrap_or_default())
    }

    async fn tree(&self, trace_id: &str) -> Result<Vec<Value>, SentinelError> {
        bounded_tree(&self.runtime, trace_id).await
    }

    async fn logs(&self, trace_id: &str) -> Result<Vec<Value>, SentinelError> {
        let response = self
            .runtime
            .call(
                "engine::logs::list",
                json!({
                    "trace_id": trace_id,
                    "severity_min": LOG_SEVERITY_MIN,
                    "limit": LOG_LIMIT,
                }),
            )
            .await?;
        Ok(response
            .get("logs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }
}

#[async_trait]
impl TraceAvailability for IiiTelemetry {
    async fn trace_exists(&self, trace_id: &str) -> bool {
        self.traces(&[trace_id.to_string()])
            .await
            .map(|traces| !traces.is_empty())
            .unwrap_or(false)
    }
}

/// Spans a trace may have and still be read whole, as one `traces::tree`.
///
/// The SDK refuses a frame over 16 MiB and drops the connection with it —
/// every call in flight fails, the console loses the page, and the ingest job
/// that asked is redelivered to ask again. A long harness turn carries each
/// prompt and reply as a payload event: 2 428 spans came to 80 MiB.
const TREE_SPANS_MAX: u64 = 300;
/// Error spans read per page, and in all, from a trace too big for its tree.
const ERROR_SPANS_PAGE: usize = 25;
const ERROR_SPANS_MAX: usize = 200;

/// A trace's tree, or — past [`TREE_SPANS_MAX`] — its error spans only,
/// read a page at a time and nested by their own parent links. The error
/// spans are what grouping and evidence need: the failure, and the chain it
/// propagated up.
pub(crate) async fn bounded_tree(
    runtime: &Runtime,
    trace_id: &str,
) -> Result<Vec<Value>, SentinelError> {
    let span_count = runtime
        .call(
            "engine::traces::list",
            json!({ "trace_ids": [trace_id], "limit": 1 }),
        )
        .await?
        .get("traces")
        .and_then(Value::as_array)
        .and_then(|traces| traces.first())
        .and_then(|trace| trace.get("span_count"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if span_count <= TREE_SPANS_MAX {
        let response = runtime
            .call("engine::traces::tree", json!({ "trace_id": trace_id }))
            .await?;
        return Ok(response
            .get("roots")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default());
    }
    let mut spans = Vec::new();
    while spans.len() < ERROR_SPANS_MAX {
        let page = runtime
            .call(
                "engine::traces::spans",
                json!({
                    "trace_id": trace_id,
                    "status": "error",
                    "search_all_spans": true,
                    "limit": ERROR_SPANS_PAGE,
                    "offset": spans.len(),
                }),
            )
            .await?
            .get("spans")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let last = page.len() < ERROR_SPANS_PAGE;
        spans.extend(page);
        if last {
            break;
        }
    }
    Ok(nest(spans))
}

/// Flat spans into the shape `traces::tree` answers with: each span under
/// its parent when the parent is among them, a root otherwise. A span whose
/// parent chain loops is kept as a root rather than lost.
fn nest(spans: Vec<Value>) -> Vec<Value> {
    use std::collections::{HashMap, HashSet};
    let id_of = |span: &Value| {
        span.get("span_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let ids: HashSet<String> = spans.iter().map(id_of).collect();
    let mut children: HashMap<String, Vec<Value>> = HashMap::new();
    let mut roots = Vec::new();
    for span in spans {
        match span.get("parent_span_id").and_then(Value::as_str) {
            Some(parent) if ids.contains(parent) && parent != id_of(&span) => {
                children.entry(parent.to_string()).or_default().push(span)
            }
            _ => roots.push(span),
        }
    }
    fn attach(mut node: Value, children: &mut HashMap<String, Vec<Value>>) -> Value {
        let id = node
            .get("span_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let kids = children.remove(&id).unwrap_or_default();
        let kids: Vec<Value> = kids.into_iter().map(|kid| attach(kid, children)).collect();
        if let Some(object) = node.as_object_mut() {
            object.insert("children".into(), Value::Array(kids));
        }
        node
    }
    let mut tree: Vec<Value> = roots
        .into_iter()
        .map(|root| attach(root, &mut children))
        .collect();
    // Whatever no root reached sat on a loop; it stays, flat.
    let rest: Vec<Value> = children.drain().flat_map(|(_, kids)| kids).collect();
    tree.extend(
        rest.into_iter()
            .map(|span| attach(span, &mut HashMap::new())),
    );
    tree
}

fn summary_of(value: &Value) -> TraceSummary {
    TraceSummary {
        trace_id: value
            .get("trace_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        status: value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        error_count: value
            .get("error_count")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        service_name: value
            .get("service_name")
            .and_then(Value::as_str)
            .map(str::to_string),
        function_id: value
            .get("function_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        name: value
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string),
        span_count: value.get("span_count").and_then(Value::as_u64).unwrap_or(0),
        trace_tags: value
            .get("trace_tags")
            .and_then(Value::as_object)
            .map(|tags| {
                tags.iter()
                    .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// `engine::functions::list` and `engine::workers::list`.
pub struct IiiRegistry {
    runtime: Runtime,
}

impl IiiRegistry {
    pub fn new(runtime: Runtime) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl EngineRegistry for IiiRegistry {
    async fn list_functions(&self) -> Result<Vec<FunctionEntry>, SentinelError> {
        let response = self
            .runtime
            .call(
                "engine::functions::list",
                // Internal handlers own functions too, and a failure inside
                // one still belongs to its worker.
                json!({ "include_internal": true }),
            )
            .await?;
        Ok(response
            .get("functions")
            .and_then(Value::as_array)
            .map(|functions| {
                functions
                    .iter()
                    .filter_map(|entry| {
                        Some(FunctionEntry {
                            function_id: entry.get("function_id")?.as_str()?.to_string(),
                            namespace: entry
                                .get("namespace")
                                .and_then(Value::as_str)
                                .unwrap_or("default")
                                .to_string(),
                            worker_name: entry.get("worker_name")?.as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn list_workers(&self) -> Result<Vec<WorkerEntry>, SentinelError> {
        let response = self
            .runtime
            .call("engine::workers::list", json!({}))
            .await?;
        Ok(response
            .get("workers")
            .and_then(Value::as_array)
            .map(|workers| {
                workers
                    .iter()
                    .filter_map(|entry| {
                        Some(WorkerEntry {
                            name: entry.get("name")?.as_str()?.to_string(),
                            namespace: entry
                                .get("namespace")
                                .and_then(Value::as_str)
                                .unwrap_or("default")
                                .to_string(),
                            version: entry
                                .get("version")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default())
    }
}

/// The durable ingest queue.
pub struct IngestQueue {
    runtime: Runtime,
    concurrency: u32,
}

impl IngestQueue {
    pub fn new(runtime: Runtime, concurrency: u32) -> Self {
        Self {
            runtime,
            concurrency,
        }
    }

    /// Define the queue. One attempt: how long to keep trying is the
    /// caller's call, because "the queue worker is not up yet" is a boot
    /// condition rather than an error.
    pub async fn ensure(&self) -> Result<(), SentinelError> {
        let concurrency = self.concurrency;
        let definition = json!({
            "queue": QUEUE_NAME,
            "config": {
                "type": "fifo",
                // One trace's spans and logs are handled in order, so the
                // join window is a wait rather than a race.
                "message_group_field": "trace_id",
                "concurrency": concurrency,
                "max_retries": 3,
                "backoff_ms": 1_000,
                "poll_interval_ms": 100,
                "redeliver_on_engine_restart": true,
            },
        });
        match self.runtime.call("queue::define", definition.clone()).await {
            Ok(_) => Ok(()),
            Err(error) => {
                // An older queue worker rejects the redelivery flag; the queue
                // without it is still correct, just less durable across an
                // engine restart.
                let message = error.to_string();
                if message.contains("redeliver_on_engine_restart") {
                    let mut fallback = definition;
                    if let Some(config) = fallback.get_mut("config").and_then(Value::as_object_mut)
                    {
                        config.remove("redeliver_on_engine_restart");
                    }
                    if self.runtime.call("queue::define", fallback).await.is_ok() {
                        tracing::warn!(
                            "the queue worker does not support redelivery on engine restart"
                        );
                        return Ok(());
                    }
                }
                Err(SentinelError::dependency(format!(
                    "could not define the {QUEUE_NAME} queue: {message}"
                )))
            }
        }
    }

    /// Hand a job to the queue rather than running it inline.
    pub async fn enqueue(&self, job: &IngestJob) -> Result<(), SentinelError> {
        let payload = serde_json::to_value(job)
            .map_err(|error| SentinelError::dependency(error.to_string()))?;
        let iii = self.runtime.iii.clone();
        let ring = self.runtime.ring.clone();
        run_with_baggage(&[HIDDEN_TAG], async move {
            if let Some(trace_id) = current_trace_id() {
                ring.remember(&trace_id);
            }
            iii.trigger(TriggerRequest {
                function_id: crate::functions::INGEST_ID.to_string(),
                payload,
                action: Some(TriggerAction::Enqueue {
                    queue: QUEUE_NAME.to_string(),
                }),
                timeout_ms: Some(CALL_TIMEOUT_MS),
            })
            .await
            .map_err(|error| SentinelError::dependency(format!("enqueue: {error}")))
        })
        .await?;
        Ok(())
    }
}

/// The commit a mapped checkout is on, for workers whose self-reported
/// version is too indistinct to detect a regression.
pub struct Checkouts {
    config: crate::ConfigCell,
}

impl Checkouts {
    pub fn new(config: crate::ConfigCell) -> Self {
        Self { config }
    }
}

#[async_trait]
impl CheckoutVersions for Checkouts {
    async fn version_for(&self, worker: &str) -> Option<String> {
        let config = self.config.read().await.clone();
        let repository = config.repository_for_worker(worker)?;
        head_commit(&repository.path).await
    }
}

/// `git rev-parse --short HEAD`, or nothing. A missing or non-git path is
/// normal — it just means this worker has no version to compare against.
pub async fn head_commit(path: &str) -> Option<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new("git")
            .args(["-C", path, "rev-parse", "--short", "HEAD"])
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!commit.is_empty()).then(|| format!("git:{commit}"))
}

/// Where the ingest reads its settings from at the moment a job runs.
pub async fn snapshot(config: &crate::ConfigCell) -> Arc<WorkerConfig> {
    config.read().await.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_path_that_is_not_a_checkout_has_no_version_to_report() {
        assert_eq!(head_commit("/nonexistent/checkout").await, None);
    }

    #[tokio::test]
    async fn this_repository_reports_its_commit_with_a_prefix() {
        // The crate lives in a git checkout, so this is the real path.
        let commit = head_commit(env!("CARGO_MANIFEST_DIR")).await;
        assert!(
            commit
                .as_deref()
                .is_some_and(|value| value.starts_with("git:")),
            "{commit:?}"
        );
    }
}

#[cfg(test)]
mod nest_tests {
    use super::nest;
    use serde_json::json;

    #[test]
    fn error_spans_nest_under_the_parents_among_them_and_loops_are_kept() {
        let spans = vec![
            json!({ "span_id": "leaf", "parent_span_id": "mid" }),
            json!({ "span_id": "mid", "parent_span_id": "root" }),
            json!({ "span_id": "root", "parent_span_id": "absent" }),
            json!({ "span_id": "a", "parent_span_id": "b" }),
            json!({ "span_id": "b", "parent_span_id": "a" }),
        ];
        let tree = nest(spans);
        let root = tree
            .iter()
            .find(|node| node["span_id"] == "root")
            .expect("the chain has its root");
        assert_eq!(root["children"][0]["span_id"], "mid");
        assert_eq!(root["children"][0]["children"][0]["span_id"], "leaf");
        let count = |id: &str| tree.iter().filter(|node| node["span_id"] == id).count();
        assert_eq!(
            count("a") + count("b"),
            2,
            "a loop is kept, not lost and not repeated"
        );
    }
}
