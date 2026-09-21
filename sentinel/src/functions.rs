//! The registered function surface and its schema catalog.
//!
//! Internal handlers — the queue step, the trigger targets, the
//! configuration doorbell — carry `internal: true` so they stay off the
//! catalog agents browse, and `trace_hidden: true` so their spans do not
//! clutter the trace views of the very pipeline they observe
//! (`docs/sops/trace-hidden-functions.md`).

use std::sync::Arc;

use iii_sdk::{IIIClient, RegisterFunction};
use schemars::{schema::RootSchema, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::configuration::ConfigCell;
use crate::events::Emitter;
use crate::iii_runtime::{IiiDb, IngestQueue};
use crate::ingest::{Ingest, IngestJob};
use crate::investigation::proxies::Proxies;
use crate::investigation::record::Caller;
use crate::investigation::{Announcement, Investigations};
use crate::registry::EngineRegistry;
use crate::service::Service;
use crate::store::Store;
use crate::{
    ids, Counters, DiagnosesListRequestV1, DiagnosesListResponseV1, DiagnosisRecordRequestV1,
    DiagnosisRecordResponseV1, DoorbellResponseV1, EvidenceGetRequestV1, EvidenceGetResponseV1,
    GroupActionRequestV1, GroupChangedOpV1, GroupGetRequestV1, GroupGetResponseV1,
    GroupStateResponseV1, GroupsListRequestV1, GroupsListResponseV1, IgnoreRequestV1,
    InvestigateRequestV1, InvestigateResponseV1, InvestigationCancelRequestV1,
    InvestigationGetRequestV1, InvestigationGetResponseV1, InvestigationSummaryV1,
    InvestigationsListRequestV1, InvestigationsListResponseV1, LogsListRequestV1,
    LogsListResponseV1, OccurrencesListRequestV1, OccurrencesListResponseV1, ResolveRequestV1,
    StatusRequestV1, StatusResponseV1, TraceGetRequestV1, TraceGetResponseV1, TurnCompletedEventV1,
};

pub const STATUS_ID: &str = "sentinel::status";
pub const STATUS_DESC: &str = "Health and counters: whether ingest is enabled, the state of the engine's telemetry stores, per-source ingest counters (including values redacted on capture and traces lost before capture), group and investigation counts, and which mapped repositories are present on this machine.";

pub const GROUPS_LIST_ID: &str = "sentinel::groups::list";
pub const GROUPS_LIST_DESC: &str = "List error groups, regressions first. Filters by state, worker, time window and a search over title, message and function id; defaults to the open states.";
pub const GROUPS_GET_ID: &str = "sentinel::groups::get";
pub const GROUPS_GET_DESC: &str = "Read one error group with its latest occurrence, and whether that occurrence's trace is still readable in the engine or only as the frozen snapshot.";
pub const GROUPS_RESOLVE_ID: &str = "sentinel::groups::resolve";
pub const GROUPS_RESOLVE_DESC: &str = "Mark a group resolved. With until_version_change, occurrences on the resolved version keep counting without reopening it — the fix is not deployed there yet.";
pub const GROUPS_IGNORE_ID: &str = "sentinel::groups::ignore";
pub const GROUPS_IGNORE_DESC: &str = "Ignore a group forever, for a number of further occurrences, or until the worker version changes. Occurrences keep counting either way.";
pub const GROUPS_UNIGNORE_ID: &str = "sentinel::groups::unignore";
pub const GROUPS_UNIGNORE_DESC: &str =
    "Take a group off the ignore list and back into the open list.";
pub const GROUPS_REOPEN_ID: &str = "sentinel::groups::reopen";
pub const GROUPS_REOPEN_DESC: &str =
    "Reopen a resolved or ignored group, clearing the rule that closed it.";
pub const OCCURRENCES_LIST_ID: &str = "sentinel::occurrences::list";
pub const OCCURRENCES_LIST_DESC: &str = "List the recorded occurrences of one group, newest first, each saying whether its frozen evidence is still kept.";
pub const EVIDENCE_GET_ID: &str = "sentinel::evidence::get";
pub const EVIDENCE_GET_DESC: &str = "Read the frozen evidence of one occurrence: the span tree, the logs of that trace and the session tags, captured before the engine's ring discarded them. Reports pruned when retention has removed it.";

pub const INGEST_ID: &str = "sentinel::ingest";
pub const INGEST_DESC: &str =
    "Internal durable queue step: reads one trace or log and records what failed.";
pub const ON_TRACE_ACTIVITY_ID: &str = "sentinel::on-trace-activity";
pub const ON_TRACE_ACTIVITY_DESC: &str =
    "Internal handler for the engine's error-span trigger; queues the traces worth reading.";
pub const ON_LOG_ID: &str = "sentinel::on-log";
pub const ON_LOG_DESC: &str = "Internal handler for the engine's ERROR-log trigger.";
pub const ON_TURN_COMPLETED_ID: &str = "sentinel::on-turn-completed";
pub const ON_TURN_COMPLETED_DESC: &str =
    "Internal doorbell for a finished investigation turn; the status is re-read rather than trusted.";
pub const ON_SCHEDULE_ID: &str = "sentinel::on-schedule";
pub const ON_SCHEDULE_DESC: &str = "Internal daily prune of buckets and archived groups.";

pub const DIAGNOSES_LIST_ID: &str = "sentinel::diagnoses::list";
pub const DIAGNOSES_LIST_DESC: &str = "List every diagnosis recorded against a group, newest first. Each recording is a version and none are overwritten, so the same failure diagnosed twice can be compared.";

pub const INVESTIGATE_ID: &str = "sentinel::investigate";
pub const INVESTIGATE_DESC: &str = "Open an investigation on a group: a harness session beside the page that starts from the frozen evidence. Mode assisted runs a first pass immediately; mode chat only seats the evidence and waits for a person. A first pass already running is returned rather than duplicated.";
pub const INVESTIGATIONS_GET_ID: &str = "sentinel::investigations::get";
pub const INVESTIGATIONS_GET_DESC: &str =
    "Read one investigation and every diagnosis it recorded, newest first.";
pub const INVESTIGATIONS_LIST_ID: &str = "sentinel::investigations::list";
pub const INVESTIGATIONS_LIST_DESC: &str =
    "List investigations, newest first, optionally narrowed to one group or a set of states.";
pub const INVESTIGATIONS_CANCEL_ID: &str = "sentinel::investigations::cancel";
pub const INVESTIGATIONS_CANCEL_DESC: &str = "Stop an investigation's first pass. The group returns to the state it was in unless a diagnosis was already recorded.";

pub const TRACE_GET_ID: &str = "sentinel::trace::get";
pub const TRACE_GET_DESC: &str = "Read a trace from the live engine through the redactor, for an investigation comparing the frozen snapshot with what is there now. Answers null once the trace has left the engine's ring.";
pub const LOGS_LIST_ID: &str = "sentinel::logs::list";
pub const LOGS_LIST_DESC: &str =
    "Read logs from the live engine through the redactor, for an investigation.";

/// The one write an investigation may make.
pub const DIAGNOSIS_RECORD_ID: &str = "sentinel::diagnosis::record";
pub const DIAGNOSIS_RECORD_DESC: &str = "Record what an investigation concluded about a group. Called by the agent from inside its own investigation session; the session's identity comes from the invocation, never from the payload. Each call is a new version and the most recent one stands.";

pub use crate::configuration::{CONFIG_CHANGE_DESC, CONFIG_CHANGE_ID};

/// The tick the engine's `trace` trigger delivers: ids only, coalesced.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct TraceActivityEventV1 {
    #[serde(default)]
    pub trace_ids: Vec<String>,
}

/// One ERROR log as the engine's `log` trigger delivers it.
///
/// Typed rather than taken as a free value: the published surface is what
/// interface capture records, and an untyped request there tells a reader
/// nothing. Every field is optional and anything unrecognised is carried
/// through, so a newer engine cannot break the handler.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct StoredLogEventV1 {
    #[serde(default)]
    pub timestamp_unix_nano: u64,
    #[serde(default)]
    pub observed_timestamp_unix_nano: u64,
    #[serde(default)]
    pub severity_number: i32,
    #[serde(default)]
    pub severity_text: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub attributes: serde_json::Map<String, Value>,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub span_id: Option<String>,
    #[serde(default)]
    pub resource: serde_json::Map<String, Value>,
    #[serde(default)]
    pub service_name: String,
    #[serde(default)]
    pub instrumentation_scope_name: Option<String>,
    #[serde(default)]
    pub instrumentation_scope_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct QueuedResponseV1 {
    pub queued: u64,
    /// Ticks for this worker's own traces, dropped before the queue.
    pub phantom_dropped: u64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct IngestResponseV1 {
    pub recorded: u64,
    pub deduped: u64,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ScheduleEventV1 {
    #[serde(default)]
    #[schemars(skip)]
    pub _caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PruneResponseV1 {
    pub buckets_removed: u64,
    pub groups_archived: u64,
    pub evidence_pruned: u64,
}

pub struct Deps<E: EngineRegistry + 'static> {
    pub config: ConfigCell,
    pub config_error: crate::ConfigErrorCell,
    pub counters: Arc<Counters>,
    pub store: Arc<Store<IiiDb>>,
    pub service: Arc<Service<IiiDb>>,
    pub ingest: Arc<Ingest<IiiDb, E>>,
    pub investigations: Arc<Investigations<IiiDb>>,
    pub proxies: Arc<Proxies>,
    pub queue: Arc<IngestQueue>,
    pub emitter: Arc<Emitter>,
}

pub fn register_all<E: EngineRegistry + 'static>(iii: &IIIClient, deps: &Arc<Deps<E>>) {
    let current = deps.clone();
    iii.register_function(
        STATUS_ID,
        RegisterFunction::new_async(move |_request: StatusRequestV1| {
            let deps = current.clone();
            async move { Ok::<_, iii_sdk::errors::Error>(status(&deps).await) }
        })
        .description(STATUS_DESC),
    );

    let current = deps.clone();
    iii.register_function(
        GROUPS_LIST_ID,
        RegisterFunction::new_async(move |request: GroupsListRequestV1| {
            let deps = current.clone();
            async move { deps.service.list(request).await.map_err(Into::into) }
        })
        .description(GROUPS_LIST_DESC),
    );

    let current = deps.clone();
    iii.register_function(
        GROUPS_GET_ID,
        RegisterFunction::new_async(move |request: GroupGetRequestV1| {
            let deps = current.clone();
            async move { deps.service.get(request).await.map_err(Into::into) }
        })
        .description(GROUPS_GET_DESC),
    );

    let current = deps.clone();
    iii.register_function(
        GROUPS_RESOLVE_ID,
        RegisterFunction::new_async(move |request: ResolveRequestV1| {
            let deps = current.clone();
            async move {
                let response = deps.service.resolve(request).await?;
                announce(&deps, &response).await;
                Ok::<_, iii_sdk::errors::Error>(response)
            }
        })
        .description(GROUPS_RESOLVE_DESC),
    );

    let current = deps.clone();
    iii.register_function(
        GROUPS_IGNORE_ID,
        RegisterFunction::new_async(move |request: IgnoreRequestV1| {
            let deps = current.clone();
            async move {
                let response = deps.service.ignore(request).await?;
                announce(&deps, &response).await;
                Ok::<_, iii_sdk::errors::Error>(response)
            }
        })
        .description(GROUPS_IGNORE_DESC),
    );

    let current = deps.clone();
    iii.register_function(
        GROUPS_UNIGNORE_ID,
        RegisterFunction::new_async(move |request: GroupActionRequestV1| {
            let deps = current.clone();
            async move {
                let response = deps.service.unignore(request).await?;
                announce(&deps, &response).await;
                Ok::<_, iii_sdk::errors::Error>(response)
            }
        })
        .description(GROUPS_UNIGNORE_DESC),
    );

    let current = deps.clone();
    iii.register_function(
        GROUPS_REOPEN_ID,
        RegisterFunction::new_async(move |request: GroupActionRequestV1| {
            let deps = current.clone();
            async move {
                let response = deps.service.reopen(request).await?;
                announce(&deps, &response).await;
                Ok::<_, iii_sdk::errors::Error>(response)
            }
        })
        .description(GROUPS_REOPEN_DESC),
    );

    let current = deps.clone();
    iii.register_function(
        OCCURRENCES_LIST_ID,
        RegisterFunction::new_async(move |request: OccurrencesListRequestV1| {
            let deps = current.clone();
            async move { deps.service.occurrences(request).await.map_err(Into::into) }
        })
        .description(OCCURRENCES_LIST_DESC),
    );

    let current = deps.clone();
    iii.register_function(
        EVIDENCE_GET_ID,
        RegisterFunction::new_async(move |request: EvidenceGetRequestV1| {
            let deps = current.clone();
            async move { deps.service.evidence(request).await.map_err(Into::into) }
        })
        .description(EVIDENCE_GET_DESC),
    );

    let current = deps.clone();
    iii.register_function(
        DIAGNOSES_LIST_ID,
        RegisterFunction::new_async(move |request: DiagnosesListRequestV1| {
            let deps = current.clone();
            async move { deps.service.diagnoses(request).await.map_err(Into::into) }
        })
        .description(DIAGNOSES_LIST_DESC),
    );

    // ── investigation ───────────────────────────────────────────────────
    let current = deps.clone();
    iii.register_function(
        INVESTIGATE_ID,
        RegisterFunction::new_async(move |request: InvestigateRequestV1| {
            let deps = current.clone();
            async move {
                let outcome = deps.investigations.investigate(request).await?;
                broadcast(&deps, outcome.events).await;
                Ok::<InvestigateResponseV1, iii_sdk::errors::Error>(outcome.value)
            }
        })
        .description(INVESTIGATE_DESC),
    );

    let current = deps.clone();
    iii.register_function(
        INVESTIGATIONS_GET_ID,
        RegisterFunction::new_async(move |request: InvestigationGetRequestV1| {
            let deps = current.clone();
            async move {
                deps.investigations
                    .get(request)
                    .await
                    .map_err(iii_sdk::errors::Error::from)
            }
        })
        .description(INVESTIGATIONS_GET_DESC),
    );

    let current = deps.clone();
    iii.register_function(
        INVESTIGATIONS_LIST_ID,
        RegisterFunction::new_async(move |request: InvestigationsListRequestV1| {
            let deps = current.clone();
            async move {
                deps.investigations
                    .list(request)
                    .await
                    .map_err(iii_sdk::errors::Error::from)
            }
        })
        .description(INVESTIGATIONS_LIST_DESC),
    );

    let current = deps.clone();
    iii.register_function(
        INVESTIGATIONS_CANCEL_ID,
        RegisterFunction::new_async(move |request: InvestigationCancelRequestV1| {
            let deps = current.clone();
            async move {
                let outcome = deps.investigations.cancel(request).await?;
                broadcast(&deps, outcome.events).await;
                Ok::<InvestigationSummaryV1, iii_sdk::errors::Error>(outcome.value)
            }
        })
        .description(INVESTIGATIONS_CANCEL_DESC),
    );

    // The agent's one write. Its identity comes from the invocation's
    // baggage — and *where* that is read is load-bearing. The SDK evaluates
    // `handler(request)` and only then wraps the future it returned with the
    // invocation's OTel context (`iii-sdk-0.23.0/src/iii.rs:2327`), so the
    // closure body runs **outside** that context and sees no baggage at all.
    // Reading it inside the async block is what puts the read under the
    // context; the same reason a bare `tokio::spawn` would lose it.
    let current = deps.clone();
    iii.register_function(
        DIAGNOSIS_RECORD_ID,
        RegisterFunction::new_async(move |request: DiagnosisRecordRequestV1| {
            let deps = current.clone();
            async move {
                let caller = Caller {
                    session_id: iii_helpers::observability::get_baggage_entry("iii.session.id"),
                    turn_id: iii_helpers::observability::get_baggage_entry("iii.message.id"),
                };
                let outcome = deps.investigations.record(caller, request).await?;
                broadcast(&deps, outcome.events).await;
                Ok::<DiagnosisRecordResponseV1, iii_sdk::errors::Error>(outcome.value)
            }
        })
        .description(DIAGNOSIS_RECORD_DESC),
    );

    let current = deps.clone();
    iii.register_function(
        TRACE_GET_ID,
        RegisterFunction::new_async(move |request: TraceGetRequestV1| {
            let deps = current.clone();
            async move {
                deps.proxies
                    .trace(request)
                    .await
                    .map_err(iii_sdk::errors::Error::from)
            }
        })
        .description(TRACE_GET_DESC),
    );

    let current = deps.clone();
    iii.register_function(
        LOGS_LIST_ID,
        RegisterFunction::new_async(move |request: LogsListRequestV1| {
            let deps = current.clone();
            async move {
                deps.proxies
                    .logs(request)
                    .await
                    .map_err(iii_sdk::errors::Error::from)
            }
        })
        .description(LOGS_LIST_DESC),
    );

    // ── internal: the pipeline's own plumbing ───────────────────────────
    let current = deps.clone();
    iii.register_function(
        ON_TRACE_ACTIVITY_ID,
        RegisterFunction::new_async(move |event: TraceActivityEventV1| {
            let deps = current.clone();
            async move { Ok::<_, iii_sdk::errors::Error>(on_trace_activity(&deps, event).await) }
        })
        .description(ON_TRACE_ACTIVITY_DESC)
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    let current = deps.clone();
    iii.register_function(
        ON_LOG_ID,
        RegisterFunction::new_async(move |log: StoredLogEventV1| {
            let deps = current.clone();
            async move { Ok::<_, iii_sdk::errors::Error>(on_log(&deps, log).await) }
        })
        .description(ON_LOG_DESC)
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    let current = deps.clone();
    iii.register_function(
        INGEST_ID,
        RegisterFunction::new_async(move |job: IngestJob| {
            let deps = current.clone();
            async move { ingest(&deps, job).await.map_err(Into::into) }
        })
        .description(INGEST_DESC)
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    let current = deps.clone();
    iii.register_function(
        ON_TURN_COMPLETED_ID,
        RegisterFunction::new_async(move |event: TurnCompletedEventV1| {
            let deps = current.clone();
            async move {
                let outcome = deps.investigations.on_turn_completed(event).await?;
                broadcast(&deps, outcome.events).await;
                Ok::<DoorbellResponseV1, iii_sdk::errors::Error>(outcome.value)
            }
        })
        .description(ON_TURN_COMPLETED_DESC)
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    let current = deps.clone();
    iii.register_function(
        ON_SCHEDULE_ID,
        RegisterFunction::new_async(move |_event: ScheduleEventV1| {
            let deps = current.clone();
            async move { prune(&deps).await.map_err(Into::into) }
        })
        .description(ON_SCHEDULE_DESC)
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );
}

async fn status<E: EngineRegistry>(deps: &Deps<E>) -> StatusResponseV1 {
    let config = deps.config.read().await.clone();
    let config_error = deps.config_error.read().await.clone();
    let groups = deps.store.group_counts().await.unwrap_or_default();
    let investigations = deps.store.investigation_counts().await.unwrap_or_default();
    if let Ok(pending) = deps.store.pending_log_count().await {
        deps.counters.set_logs_pending_join(pending);
    }
    deps.counters
        .snapshot(&config, ids::now_ms(), groups, investigations, config_error)
}

/// The tick handler does as little as possible: the engine ignores its
/// result, and anything slow here holds up the trigger's own loop.
async fn on_trace_activity<E: EngineRegistry>(
    deps: &Deps<E>,
    event: TraceActivityEventV1,
) -> QueuedResponseV1 {
    if !deps.counters.is_ready() {
        deps.counters.add_dropped_not_ready(1);
        return QueuedResponseV1 {
            queued: 0,
            phantom_dropped: 0,
        };
    }
    let (wanted, phantom_dropped) = deps.ingest.filter_tick(&event.trace_ids);
    let mut queued = 0;
    for trace_id in wanted {
        let job = IngestJob::Trace { trace_id };
        if let Err(error) = deps.queue.enqueue(&job).await {
            tracing::warn!(%error, "could not queue a trace for ingest");
            continue;
        }
        deps.counters.add_queued(1);
        queued += 1;
    }
    QueuedResponseV1 {
        queued,
        phantom_dropped,
    }
}

async fn on_log<E: EngineRegistry>(deps: &Deps<E>, log: StoredLogEventV1) -> QueuedResponseV1 {
    let idle = QueuedResponseV1 {
        queued: 0,
        phantom_dropped: 0,
    };
    if !deps.counters.is_ready() {
        deps.counters.add_dropped_not_ready(1);
        return idle;
    }
    let trace_id = log
        .trace_id
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "-".into());
    let log = serde_json::to_value(log).unwrap_or(Value::Null);
    let job = IngestJob::Log { trace_id, log };
    match deps.queue.enqueue(&job).await {
        Ok(()) => {
            deps.counters.add_queued(1);
            QueuedResponseV1 {
                queued: 1,
                phantom_dropped: 0,
            }
        }
        Err(error) => {
            tracing::warn!(%error, "could not queue a log for ingest");
            idle
        }
    }
}

async fn ingest<E: EngineRegistry>(
    deps: &Deps<E>,
    job: IngestJob,
) -> Result<IngestResponseV1, crate::SentinelError> {
    if !deps.counters.is_ready() {
        deps.counters.add_dropped_not_ready(1);
        // The queue redelivers rather than dropping it.
        return Err(crate::SentinelError::NotReady(
            "the store and queue are not claimed yet".into(),
        ));
    }
    let config = deps.config.read().await.clone();
    let report = deps.ingest.handle(job, &config).await?;
    deps.counters.sub_queued(1);

    for event in &report.events {
        let op = if event.created {
            GroupChangedOpV1::Created
        } else if event.reason.is_some() {
            GroupChangedOpV1::Status
        } else {
            GroupChangedOpV1::Occurrence
        };
        deps.emitter
            .group_changed(crate::events::group_event(
                op,
                &event.group_id,
                event.status,
                occurrence_count(deps, &event.group_id).await,
                event.reason,
            ))
            .await;
    }
    for follow_up in report.follow_up {
        if let Err(error) = deps.queue.enqueue(&follow_up).await {
            tracing::warn!(%error, "could not queue follow-up ingest work");
        } else {
            deps.counters.add_queued(1);
        }
    }
    Ok(IngestResponseV1 {
        recorded: report.recorded,
        deduped: report.deduped,
    })
}

async fn prune<E: EngineRegistry>(deps: &Deps<E>) -> Result<PruneResponseV1, crate::SentinelError> {
    let config = deps.config.read().await.clone();
    let outcome = crate::retention::prune(&deps.store, &config).await?;
    Ok(PruneResponseV1 {
        buckets_removed: outcome.buckets_removed,
        groups_archived: outcome.groups_archived,
        evidence_pruned: outcome.evidence_pruned,
    })
}

/// Deliver whatever an investigation decided. The decision logic returns
/// events rather than emitting them, so every path through it is testable
/// without a live client.
pub async fn broadcast<E: EngineRegistry>(deps: &Arc<Deps<E>>, events: Vec<Announcement>) {
    for event in events {
        match event {
            Announcement::Group(state) => announce(deps, &state).await,
            Announcement::Investigation(event) => {
                if let Ok(payload) = serde_json::to_value(&event) {
                    deps.emitter.investigation_changed(payload).await;
                }
            }
        }
    }
}

/// Tell the console a human decision landed.
async fn announce<E: EngineRegistry>(deps: &Arc<Deps<E>>, response: &GroupStateResponseV1) {
    if response.reason.is_none() {
        return;
    }
    deps.emitter
        .group_changed(crate::events::group_event(
            GroupChangedOpV1::Status,
            &response.group_id,
            response.status,
            occurrence_count(deps, &response.group_id).await,
            response.reason,
        ))
        .await;
}

async fn occurrence_count<E: EngineRegistry>(deps: &Deps<E>, group_id: &str) -> u64 {
    deps.store
        .group_by_id(group_id)
        .await
        .ok()
        .flatten()
        .map(|group| group.state.occurrence_count)
        .unwrap_or_default()
}

pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: RootSchema,
    pub response_schema: RootSchema,
}

fn schema_of<T: JsonSchema>() -> RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req: JsonSchema, Resp: JsonSchema>(
    function_id: &'static str,
    description: &'static str,
) -> FunctionSpec {
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

/// Every function this worker registers, with its published schemas. Golden
/// tested: a surface change that is not deliberate fails the build.
pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<StatusRequestV1, StatusResponseV1>(STATUS_ID, STATUS_DESC),
        spec::<GroupsListRequestV1, GroupsListResponseV1>(GROUPS_LIST_ID, GROUPS_LIST_DESC),
        spec::<GroupGetRequestV1, GroupGetResponseV1>(GROUPS_GET_ID, GROUPS_GET_DESC),
        spec::<ResolveRequestV1, GroupStateResponseV1>(GROUPS_RESOLVE_ID, GROUPS_RESOLVE_DESC),
        spec::<IgnoreRequestV1, GroupStateResponseV1>(GROUPS_IGNORE_ID, GROUPS_IGNORE_DESC),
        spec::<GroupActionRequestV1, GroupStateResponseV1>(
            GROUPS_UNIGNORE_ID,
            GROUPS_UNIGNORE_DESC,
        ),
        spec::<GroupActionRequestV1, GroupStateResponseV1>(GROUPS_REOPEN_ID, GROUPS_REOPEN_DESC),
        spec::<OccurrencesListRequestV1, OccurrencesListResponseV1>(
            OCCURRENCES_LIST_ID,
            OCCURRENCES_LIST_DESC,
        ),
        spec::<EvidenceGetRequestV1, EvidenceGetResponseV1>(EVIDENCE_GET_ID, EVIDENCE_GET_DESC),
        spec::<DiagnosesListRequestV1, DiagnosesListResponseV1>(
            DIAGNOSES_LIST_ID,
            DIAGNOSES_LIST_DESC,
        ),
        spec::<InvestigateRequestV1, InvestigateResponseV1>(INVESTIGATE_ID, INVESTIGATE_DESC),
        spec::<InvestigationGetRequestV1, InvestigationGetResponseV1>(
            INVESTIGATIONS_GET_ID,
            INVESTIGATIONS_GET_DESC,
        ),
        spec::<InvestigationsListRequestV1, InvestigationsListResponseV1>(
            INVESTIGATIONS_LIST_ID,
            INVESTIGATIONS_LIST_DESC,
        ),
        spec::<InvestigationCancelRequestV1, InvestigationSummaryV1>(
            INVESTIGATIONS_CANCEL_ID,
            INVESTIGATIONS_CANCEL_DESC,
        ),
        spec::<DiagnosisRecordRequestV1, DiagnosisRecordResponseV1>(
            DIAGNOSIS_RECORD_ID,
            DIAGNOSIS_RECORD_DESC,
        ),
        spec::<TraceGetRequestV1, TraceGetResponseV1>(TRACE_GET_ID, TRACE_GET_DESC),
        spec::<LogsListRequestV1, LogsListResponseV1>(LOGS_LIST_ID, LOGS_LIST_DESC),
        spec::<TraceActivityEventV1, QueuedResponseV1>(
            ON_TRACE_ACTIVITY_ID,
            ON_TRACE_ACTIVITY_DESC,
        ),
        spec::<StoredLogEventV1, QueuedResponseV1>(ON_LOG_ID, ON_LOG_DESC),
        spec::<IngestJob, IngestResponseV1>(INGEST_ID, INGEST_DESC),
        spec::<TurnCompletedEventV1, DoorbellResponseV1>(
            ON_TURN_COMPLETED_ID,
            ON_TURN_COMPLETED_DESC,
        ),
        spec::<ScheduleEventV1, PruneResponseV1>(ON_SCHEDULE_ID, ON_SCHEDULE_DESC),
        spec::<iii_config_client::OnConfigChangeEvent, iii_config_client::OnConfigChangeResponse>(
            CONFIG_CHANGE_ID,
            CONFIG_CHANGE_DESC,
        ),
    ]
}

/// Function ids an investigation session may call. There is no approval gate
/// in this design, so this list is the whole permission model: it holds the
/// reads an investigation needs and exactly one write, which is this worker's
/// own record of the diagnosis.
pub const INVESTIGATION_ALLOW: [&str; 11] = [
    "coder::info",
    "coder::read-file",
    "coder::search",
    "coder::list-folder",
    "coder::tree",
    "engine::functions::info",
    "engine::functions::list",
    EVIDENCE_GET_ID,
    TRACE_GET_ID,
    LOGS_LIST_ID,
    DIAGNOSIS_RECORD_ID,
];

/// Denied outright. Deny wins over allow in the harness policy, so a function
/// added to this worker later is refused until somebody decides otherwise.
pub const INVESTIGATION_DENY: [&str; 21] = [
    "shell::*",
    "state::*",
    "queue::*",
    "worktree::*",
    "harness::*",
    "github::*",
    "configuration::*",
    "storage::*",
    "database::*",
    "engine::traces::*",
    "engine::logs::*",
    "session::*",
    "router::*",
    "sentinel::groups::*",
    "sentinel::investigate",
    "sentinel::investigations::*",
    "sentinel::on-*",
    "sentinel::ingest",
    // The agent works from the evidence it was handed and the code it can
    // read. The worker's own health surface and the list of every other
    // occurrence are the console's, not the investigation's.
    "sentinel::status",
    "sentinel::occurrences::list",
    // An investigation reasons from the evidence it was handed. Reading what
    // an earlier one concluded would anchor it to that answer, which is the
    // opposite of what a second opinion is for.
    DIAGNOSES_LIST_ID,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_only_write_an_investigation_can_make_is_its_own_diagnosis() {
        let own: Vec<&str> = INVESTIGATION_ALLOW
            .iter()
            .copied()
            .filter(|id| id.starts_with("sentinel::"))
            .collect();
        assert_eq!(
            own,
            vec![
                EVIDENCE_GET_ID,
                TRACE_GET_ID,
                LOGS_LIST_ID,
                DIAGNOSIS_RECORD_ID
            ],
            "three reads and one write — nothing else of this worker's surface"
        );
        assert!(
            !INVESTIGATION_ALLOW
                .iter()
                .any(|id| id.starts_with("shell::")),
            "an investigation reads code; it does not run it"
        );
    }

    #[test]
    fn every_registered_function_is_either_allowed_or_denied_to_an_agent() {
        // A function added later must not become silently reachable from
        // inside an investigation.
        for spec in catalog() {
            let id = spec.function_id;
            let allowed = INVESTIGATION_ALLOW.contains(&id);
            let denied = INVESTIGATION_DENY
                .iter()
                .any(|pattern| match pattern.strip_suffix('*') {
                    Some(prefix) => id.starts_with(prefix),
                    None => *pattern == id,
                });
            assert!(
                allowed || denied,
                "{id} is neither allowed nor denied to an investigation"
            );
        }
    }
}
