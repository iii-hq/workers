//! The ingest pipeline: a tick becomes a group.
//!
//! Everything here runs on the durable queue, FIFO by `trace_id`, so the
//! spans and logs of one trace are handled in order and never race each
//! other. What the pipeline mostly does is decide what *not* to record:
//! its own traces, investigations, duplicates of a span that ticked twice,
//! and logs that turn out to belong to a failure already captured.

pub mod ring;

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapters::{log as log_adapter, trace as trace_adapter, ErrorEvent};
use crate::evidence::EvidenceBundleV1;
use crate::registry::{EngineRegistry, Registry};
use crate::store::{Db, OccurrenceWrite, PendingLogWrite, RecordOutcome, Store};
use crate::{
    fingerprint, ids, normalize, Counters, ErrorSourceV1, GroupChangeReasonV1, GroupStatusV1,
    Normalizer, Redactor, SentinelError, TraceStoreStateV1, WorkerConfig,
};

/// One span-coalesce window of the engine, which broadcasts a span before it
/// stores it: a read right on the tick can miss the very span that raised it.
pub const TICK_READ_DELAY: Duration = Duration::from_millis(300);

/// Most tick trace ids waiting for a read. Past it the oldest are dropped and
/// counted in `tick_backlog_dropped`: a stalled engine must not grow this worker.
pub const TICK_BACKLOG_MAX: usize = 4096;
/// Trace ids per `engine::traces::list` call.
const TICK_READ_BATCH: usize = 128;
/// Batches one tick handler reads before it leaves the rest to the next tick,
/// so a long backlog cannot hold the observer's in-flight slot for long.
const TICK_READ_BATCHES: usize = 4;
/// Wait after a failed read, doubling per consecutive failure up to the max.
const TICK_RETRY_MIN: Duration = Duration::from_secs(1);
const TICK_RETRY_MAX: Duration = Duration::from_secs(30);

/// Tick trace ids not read yet, oldest first with the time each arrived, and
/// when a failed read may run again. Ids that arrive while a read is in
/// flight, or that a failed read gives back, wait here for one later read
/// instead of a job each: a job per id was what turned a slow engine into a
/// flood of reads against it.
#[derive(Default)]
struct TickBacklog {
    ids: VecDeque<(String, tokio::time::Instant)>,
    waiting: HashSet<String>,
    failures: u32,
    retry_at: Option<tokio::time::Instant>,
}

impl TickBacklog {
    /// Adds the ids, each as just arrived; returns how many of the oldest it
    /// dropped to stay under [`TICK_BACKLOG_MAX`]. An id already waiting moves
    /// to the back with the new arrival: its newest span is broadcast before it
    /// is stored, so the read must wait one window from the latest tick.
    fn push(&mut self, trace_ids: impl IntoIterator<Item = String>) -> u64 {
        let now = tokio::time::Instant::now();
        for trace_id in trace_ids {
            if !self.waiting.insert(trace_id.clone()) {
                if let Some(at) = self.ids.iter().position(|(id, _)| id == &trace_id) {
                    self.ids.remove(at);
                }
            }
            self.ids.push_back((trace_id, now));
        }
        self.trim()
    }

    /// Gives a failed batch back, ahead of what arrived during the read: it
    /// is older, so the queue stays ordered by arrival.
    fn push_front(&mut self, batch: Vec<(String, tokio::time::Instant)>) -> u64 {
        for (trace_id, arrived) in batch.into_iter().rev() {
            if self.waiting.insert(trace_id.clone()) {
                self.ids.push_front((trace_id, arrived));
            }
        }
        self.trim()
    }

    fn trim(&mut self) -> u64 {
        let mut dropped = 0;
        while self.ids.len() > TICK_BACKLOG_MAX {
            if let Some((oldest, _)) = self.ids.pop_front() {
                self.waiting.remove(&oldest);
                dropped += 1;
            }
        }
        dropped
    }

    /// When the oldest waiting id has been stored by the engine: one span
    /// coalesce window after its tick arrived.
    fn oldest_readable_at(&self) -> Option<tokio::time::Instant> {
        self.ids
            .front()
            .map(|(_, arrived)| *arrived + TICK_READ_DELAY)
    }

    /// Up to `count` of the oldest ids whose window has passed. An id that
    /// joined a read in flight is too young for it and waits for the next.
    fn take(&mut self, count: usize) -> Vec<(String, tokio::time::Instant)> {
        let now = tokio::time::Instant::now();
        let mut batch = Vec::new();
        while batch.len() < count
            && self
                .ids
                .front()
                .is_some_and(|(_, arrived)| *arrived + TICK_READ_DELAY <= now)
        {
            if let Some((trace_id, arrived)) = self.ids.pop_front() {
                self.waiting.remove(&trace_id);
                batch.push((trace_id, arrived));
            }
        }
        batch
    }

    fn ready(&self) -> bool {
        !self.ids.is_empty()
            && self
                .retry_at
                .is_none_or(|at| tokio::time::Instant::now() >= at)
    }

    fn succeeded(&mut self) {
        self.failures = 0;
        self.retry_at = None;
    }

    /// Records a failed read; returns the wait before the next one.
    fn failed(&mut self) -> Duration {
        self.failures = self.failures.saturating_add(1);
        let wait = TICK_RETRY_MIN
            .saturating_mul(1u32 << self.failures.saturating_sub(1).min(5))
            .min(TICK_RETRY_MAX);
        self.retry_at = Some(tokio::time::Instant::now() + wait);
        wait
    }

    fn clear(&mut self) {
        self.ids.clear();
        self.waiting.clear();
    }
}

/// One unit of ingest work. `trace_id` is a field of every variant because
/// the queue groups by it: the span job and the log job of one trace are
/// serialized behind each other rather than racing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IngestJob {
    /// A trace ticked: read it and record what failed.
    Trace { trace_id: String },
    /// An ERROR log arrived.
    Log {
        trace_id: String,
        #[serde(default)]
        log: Value,
    },
    /// Re-read a trace once, to catch ancestors that were open at capture.
    Settle { trace_id: String },
    /// A parked log's wait ran out.
    Promote {
        trace_id: String,
        occurrence_id: String,
    },
}

impl IngestJob {
    pub fn trace_id(&self) -> &str {
        match self {
            Self::Trace { trace_id }
            | Self::Log { trace_id, .. }
            | Self::Settle { trace_id }
            | Self::Promote { trace_id, .. } => trace_id,
        }
    }
}

/// What one job did, for the caller to report and count.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IngestReport {
    pub recorded: u64,
    pub deduped: u64,
    pub events: Vec<GroupChanged>,
    /// Scheduled follow-up work, enqueued by the caller.
    pub follow_up: Vec<IngestJob>,
}

/// A group changed and the console (and any sibling worker) should know.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupChanged {
    pub group_id: String,
    pub status: GroupStatusV1,
    pub reason: Option<GroupChangeReasonV1>,
    pub created: bool,
}

/// What the pipeline needs from the engine's telemetry.
#[async_trait]
pub trait Telemetry: Send + Sync {
    /// Summaries for the given trace ids. An empty answer means the trace has
    /// already left the engine's ring.
    async fn traces(&self, trace_ids: &[String]) -> Result<Vec<TraceSummary>, SentinelError>;
    /// [`Self::traces`] for an ingest job, which waits out an engine that
    /// answers busy instead of spending the queue's retries on it.
    async fn traces_patiently(
        &self,
        trace_ids: &[String],
    ) -> Result<Vec<TraceSummary>, SentinelError> {
        self.traces(trace_ids).await
    }
    /// The full span tree, as `roots`.
    async fn tree(&self, trace_id: &str) -> Result<Vec<Value>, SentinelError>;
    async fn logs(&self, trace_id: &str) -> Result<Vec<Value>, SentinelError>;
}

/// The parts of `TraceSummary` this worker uses.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TraceSummary {
    pub trace_id: String,
    pub status: String,
    pub error_count: u64,
    pub service_name: Option<String>,
    pub function_id: Option<String>,
    pub name: Option<String>,
    pub trace_tags: BTreeMap<String, String>,
    /// Spans in the whole trace. More than the tree read returns means the
    /// trace was too big to read whole — see `iii_runtime::bounded_tree`.
    pub span_count: u64,
}

impl TraceSummary {
    fn session_id(&self) -> Option<&str> {
        self.trace_tags.get("iii.session.id").map(String::as_str)
    }

    fn holds_error(&self) -> bool {
        self.error_count > 0 || self.status.eq_ignore_ascii_case("error")
    }
}

/// Where a worker's source lives, for the version fallback.
#[async_trait]
pub trait CheckoutVersions: Send + Sync {
    /// The commit a mapped repository is on, when the worker's own version is
    /// too indistinct to detect a regression.
    async fn version_for(&self, worker: &str) -> Option<String>;
}

/// The pipeline.
pub struct Ingest<D: Db, E: EngineRegistry> {
    store: Arc<Store<D>>,
    telemetry: Arc<dyn Telemetry>,
    registry: Arc<Registry<E>>,
    checkouts: Arc<dyn CheckoutVersions>,
    counters: Arc<Counters>,
    ring: Arc<ring::PhantomRing>,
    tick_backlog: Mutex<TickBacklog>,
    /// Held by the one tick handler that is reading the backlog.
    tick_reader: tokio::sync::Mutex<()>,
    /// A later read of the backlog is scheduled; see [`Ingest::claim_tick_drain`].
    tick_drain_scheduled: std::sync::atomic::AtomicBool,
}

impl<D: Db, E: EngineRegistry> Ingest<D, E> {
    pub fn new(
        store: Arc<Store<D>>,
        telemetry: Arc<dyn Telemetry>,
        registry: Arc<Registry<E>>,
        checkouts: Arc<dyn CheckoutVersions>,
        counters: Arc<Counters>,
        ring: Arc<ring::PhantomRing>,
    ) -> Self {
        Self {
            store,
            telemetry,
            registry,
            checkouts,
            counters,
            ring,
            tick_backlog: Mutex::default(),
            tick_reader: tokio::sync::Mutex::new(()),
            tick_drain_scheduled: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub async fn handle(
        &self,
        job: IngestJob,
        config: &WorkerConfig,
    ) -> Result<IngestReport, SentinelError> {
        match job {
            IngestJob::Trace { trace_id } => self.handle_trace(&trace_id, config).await,
            IngestJob::Log { log, .. } => self.handle_log(&log, config).await,
            IngestJob::Settle { trace_id } => self.handle_settle(&trace_id, config).await,
            IngestJob::Promote { occurrence_id, .. } => {
                self.handle_promote(&occurrence_id, config).await
            }
        }
    }

    async fn handle_trace(
        &self,
        trace_id: &str,
        config: &WorkerConfig,
    ) -> Result<IngestReport, SentinelError> {
        let mut report = IngestReport::default();
        let Some(summary) = self.summary(trace_id).await? else {
            // The ring dropped it before the job ran. Measured, because a
            // number that climbs is the signal to raise the engine's span
            // budget rather than a bug in a group.
            self.counters.add_lost_before_capture(1);
            return Ok(report);
        };
        if !summary.holds_error() {
            return Ok(report);
        }

        let roots = self.telemetry.tree(trace_id).await?;
        let spans = trace_adapter::flatten(&roots);
        if spans.is_empty() {
            self.counters.add_lost_before_capture(1);
            return Ok(report);
        }

        let context = self.context(trace_id, &summary.trace_tags, config);
        if let Some(exclusion) = trace_adapter::exclusion(&spans, &context) {
            self.count_exclusion(exclusion);
            return Ok(report);
        }

        let logs = self.telemetry.logs(trace_id).await.unwrap_or_default();
        let redactor = Redactor::new(&config.redaction.patterns).unwrap_or_default();
        let normalizer = Normalizer::new(&config.fingerprint.identity_numbers);
        let captured_at_ms = ids::now_ms();

        for leaf in trace_adapter::leaf_errors(&spans) {
            // Somebody else's trace can still fail inside one of ours — the
            // console timing out on `sentinel::groups::list`, a tab's handler
            // gone. That failure is the monitor's own, not the caller's.
            if leaf.function_id().is_some_and(|function| {
                trace_adapter::is_own_function(&function, crate::WORKER_NAME)
            }) {
                self.counters.add_dropped_own_trace(1);
                continue;
            }
            let mut redactions = 0;
            let mut event = trace_adapter::event_for(leaf, &context, &redactor, &mut redactions);
            let mut bundle = trace_adapter::bundle_for(
                leaf,
                &spans,
                &logs,
                &context,
                &redactor,
                captured_at_ms,
                &mut redactions,
            );
            self.attribute(&mut event, &mut bundle, config).await;
            self.counters.add_redactions(redactions);
            // A trace too big to read whole arrives as its error spans only;
            // the rest is counted as left out rather than silently missing.
            bundle.truncated.spans +=
                u32::try_from(summary.span_count.saturating_sub(spans.len() as u64))
                    .unwrap_or(u32::MAX);
            bundle.fit_within(config.evidence.max_bytes as usize);

            let outcome = self
                .record(&event, Some(&bundle), &normalizer, None)
                .await?;
            self.tally(&mut report, outcome);
        }

        if !report.events.is_empty() || report.recorded > 0 {
            report.follow_up.push(IngestJob::Settle {
                trace_id: trace_id.to_string(),
            });
        }
        Ok(report)
    }

    async fn handle_log(
        &self,
        payload: &Value,
        config: &WorkerConfig,
    ) -> Result<IngestReport, SentinelError> {
        let mut report = IngestReport::default();
        let Some(log) = log_adapter::LogRecord::from_payload(payload) else {
            return Ok(report);
        };
        if !log.is_error() {
            return Ok(report);
        }
        // Checked before the trace is looked up: the engine's logs about a
        // delivery to a closed tab carry no trace at all.
        if log_adapter::is_own(&log, config) {
            self.counters.add_dropped_own_trace(1);
            return Ok(report);
        }

        let redactor = Redactor::new(&config.redaction.patterns).unwrap_or_default();
        let mut redactions = 0;

        // A log carries no session, so the trace is what says whether this
        // belongs to one of the worker's own investigations.
        let (session_id, session_unknown, excluded) = match log.trace_id.as_deref() {
            Some(trace_id) => match self.summary(trace_id).await? {
                Some(summary) => (
                    summary.session_id().map(str::to_string),
                    false,
                    self.summary_exclusion(&summary, config),
                ),
                None => (None, true, None),
            },
            None => (None, false, None),
        };
        if let Some(exclusion) = excluded {
            self.count_exclusion(exclusion);
            return Ok(report);
        }
        if session_unknown {
            self.counters.add_logs_unattributed(1);
        }

        // The span that explains this log may already be recorded: then the
        // log is evidence on that occurrence, not a group of its own.
        if let Some(trace_id) = log.trace_id.as_deref() {
            if let Some((occurrence_id, _, evidence)) =
                self.store.trace_occurrence(trace_id).await?
            {
                self.fold_into(&occurrence_id, evidence, &log, config, &redactor)
                    .await?;
                return Ok(report);
            }
        }

        let event = log_adapter::event_for(&log, session_id.clone(), &redactor, &mut redactions);
        let mut bundle = log_adapter::bundle_for(
            &log,
            &config.attribute_allowlist,
            &redactor,
            ids::now_ms(),
            &mut redactions,
        );
        bundle.fit_within(config.evidence.max_bytes as usize);
        self.counters.add_redactions(redactions);

        let pending = PendingLogWrite {
            dedupe_key: event.dedupe_key.clone(),
            at_ms: event.at_ms,
            trace_id: log.trace_id.clone(),
            span_id: log.span_id.clone(),
            session_id,
            worker_version: None,
            message: event.message.clone(),
            evidence: serde_json::to_string(&bundle).ok(),
            // From arrival: `at_ms` is the emitter's clock, and only the sweep ends this wait.
            join_deadline_ms: ids::now_ms() + config.sources.log.join_window_ms as i64,
            session_unknown,
        };
        // The sweep promotes it when the wait runs out; queued from here, the
        // promotion would run ahead of the span job this wait is for.
        if self.store.insert_pending_log(&pending).await?.is_none() {
            self.counters.add_deduped(1);
            report.deduped += 1;
        }
        self.refresh_pending_gauge().await;
        Ok(report)
    }

    /// The wait ran out: the log becomes a group of its own.
    async fn handle_promote(
        &self,
        occurrence_id: &str,
        config: &WorkerConfig,
    ) -> Result<IngestReport, SentinelError> {
        let mut report = IngestReport::default();
        let Some(pending) = self.store.pending_log(occurrence_id).await? else {
            // The span arrived first and folded it in.
            return Ok(report);
        };

        let bundle: Option<EvidenceBundleV1> = pending
            .evidence
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok());
        let service_name = bundle
            .as_ref()
            .map(|bundle| bundle.worker.service_name.clone())
            .unwrap_or_default();
        let call_site = bundle
            .as_ref()
            .and_then(|bundle| bundle.logs.first())
            .and_then(|log| {
                log.attributes
                    .get("code.function")
                    .or_else(|| log.attributes.get("target"))
                    // The same order as `LogRecord::call_site`.
                    .or_else(|| log.attributes.get("function_id"))
                    .cloned()
            });

        let mut event = ErrorEvent {
            source: ErrorSourceV1::Log,
            dedupe_key: pending.dedupe_key.clone(),
            at_ms: pending.at_ms,
            service_name,
            namespace: String::new(),
            function_id: call_site,
            span_name: None,
            exception_type: None,
            message: pending.message.clone(),
            stacktrace: None,
            trace_id: pending.trace_id.clone(),
            span_id: pending.span_id.clone(),
            session_id: pending.session_id.clone(),
            turn_id: None,
            worker_version: pending.worker_version.clone(),
            namespace_ambiguous: false,
            evidence: None,
        };
        let mut bundle = bundle.unwrap_or_else(|| EvidenceBundleV1 {
            version: crate::evidence::EVIDENCE_VERSION,
            captured_at_ms: pending.at_ms,
            settled: true,
            trace_id: pending.trace_id.clone().unwrap_or_default(),
            origin_span_id: pending.span_id.clone().unwrap_or_default(),
            propagated_through: Vec::new(),
            trace_tags: BTreeMap::new(),
            spans: Vec::new(),
            logs: Vec::new(),
            worker: crate::evidence::EvidenceWorkerV1 {
                service_name: event.service_name.clone(),
                version: None,
            },
            truncated: Default::default(),
        });
        self.attribute(&mut event, &mut bundle, config).await;

        let normalizer = Normalizer::new(&config.fingerprint.identity_numbers);
        let outcome = self
            .record(&event, Some(&bundle), &normalizer, Some(occurrence_id))
            .await?;
        self.tally(&mut report, outcome);
        self.refresh_pending_gauge().await;
        Ok(report)
    }

    /// One re-read, to replace a snapshot taken while ancestors were open.
    async fn handle_settle(
        &self,
        trace_id: &str,
        config: &WorkerConfig,
    ) -> Result<IngestReport, SentinelError> {
        let report = IngestReport::default();
        let Some((occurrence_id, _, previous)) = self.store.trace_occurrence(trace_id).await?
        else {
            return Ok(report);
        };
        let Some(summary) = self.summary(trace_id).await? else {
            // Gone from the ring already: the snapshot taken at capture is
            // what this group will ever have, and it says so.
            return Ok(report);
        };
        let roots = self.telemetry.tree(trace_id).await?;
        let spans = trace_adapter::flatten(&roots);
        if spans.is_empty() {
            return Ok(report);
        }
        let context = self.context(trace_id, &summary.trace_tags, config);
        let logs = self.telemetry.logs(trace_id).await.unwrap_or_default();
        let redactor = Redactor::new(&config.redaction.patterns).unwrap_or_default();
        let Some(leaf) = trace_adapter::leaf_errors(&spans).into_iter().next() else {
            return Ok(report);
        };
        let mut redactions = 0;
        let mut bundle = trace_adapter::bundle_for(
            leaf,
            &spans,
            &logs,
            &context,
            &redactor,
            ids::now_ms(),
            &mut redactions,
        );
        bundle.settled = true;
        // The rebuild reads the span again, and a span names the process that
        // emitted it rather than the worker that owns the failing function.
        // Carrying the attribution over keeps the settle pass from quietly
        // re-filing the evidence under the wrong worker.
        if let Some(worker) = previous
            .as_deref()
            .and_then(|json| serde_json::from_str::<EvidenceBundleV1>(json).ok())
            .map(|previous| previous.worker)
        {
            bundle.worker = worker;
        }
        bundle.fit_within(config.evidence.max_bytes as usize);
        self.counters.add_redactions(redactions);
        if let Ok(json) = serde_json::to_string(&bundle) {
            self.store
                .replace_evidence(&occurrence_id, &json, true)
                .await?;
        }
        Ok(report)
    }

    /// Append a late log to the evidence of the failure it belongs to.
    async fn fold_into(
        &self,
        occurrence_id: &str,
        evidence: Option<String>,
        log: &log_adapter::LogRecord,
        config: &WorkerConfig,
        redactor: &Redactor,
    ) -> Result<(), SentinelError> {
        let Some(mut bundle) = evidence
            .as_deref()
            .and_then(|json| serde_json::from_str::<EvidenceBundleV1>(json).ok())
        else {
            return Ok(());
        };
        let mut redactions = 0;
        let mut addition = log_adapter::bundle_for(
            log,
            &config.attribute_allowlist,
            redactor,
            bundle.captured_at_ms,
            &mut redactions,
        );
        self.counters.add_redactions(redactions);
        if let Some(entry) = addition.logs.pop() {
            if !bundle.logs.iter().any(|existing| {
                existing.timestamp_unix_nano == entry.timestamp_unix_nano
                    && existing.body == entry.body
            }) {
                bundle.logs.push(entry);
                bundle.fit_within(config.evidence.max_bytes as usize);
                if let Ok(json) = serde_json::to_string(&bundle) {
                    self.store
                        .replace_evidence(occurrence_id, &json, bundle.settled)
                        .await?;
                }
            }
        }
        Ok(())
    }

    /// Fill in the owning worker, its namespace and the version that was
    /// running. This is what keeps a failure filed under the worker whose
    /// code is wrong rather than the one whose span happened to carry it.
    async fn attribute(
        &self,
        event: &mut ErrorEvent,
        bundle: &mut EvidenceBundleV1,
        config: &WorkerConfig,
    ) {
        let span_service = config
            .resolve_service_alias(&event.service_name)
            .to_string();
        let owner = self
            .registry
            .resolve(None, event.function_id.as_deref(), &span_service)
            .await;

        let mut version = owner.version.clone();
        if crate::registry::version_is_indistinct(version.as_deref()) {
            // A development build reports the same string for weeks, so the
            // commit of the mapped checkout is what makes a regression
            // detectable at all.
            version = self.checkouts.version_for(&owner.worker).await.or(version);
        }

        event.service_name = owner.worker.clone();
        event.namespace = owner.namespace.clone();
        event.namespace_ambiguous = owner.ambiguous;
        event.worker_version = version.clone();
        bundle.worker.service_name = owner.worker;
        bundle.worker.version = version;
    }

    async fn record(
        &self,
        event: &ErrorEvent,
        bundle: Option<&EvidenceBundleV1>,
        normalizer: &Normalizer,
        pending_occurrence_id: Option<&str>,
    ) -> Result<RecordOutcome, SentinelError> {
        let normalized = normalizer.normalize(&event.message);
        let identity = event
            .function_id
            .clone()
            .or_else(|| event.span_name.clone())
            .unwrap_or_default();
        let fingerprint = match event.source {
            ErrorSourceV1::Log => fingerprint::log_fingerprint(
                &event.namespace,
                &event.service_name,
                &identity,
                &normalized,
            ),
            _ => fingerprint::trace_fingerprint(
                &event.namespace,
                &event.service_name,
                &identity,
                event.exception_type.as_deref(),
                &normalized,
            ),
        };

        let write = OccurrenceWrite {
            fingerprint,
            source: event.source,
            dedupe_key: event.dedupe_key.clone(),
            at_ms: event.at_ms,
            namespace: event.namespace.clone(),
            service_name: event.service_name.clone(),
            function_id: event.function_id.clone(),
            exception_type: event.exception_type.clone(),
            title: normalize::title(
                event.exception_type.as_deref(),
                event.function_id.as_deref(),
                &normalized,
            ),
            message: event.message.clone(),
            trace_id: event.trace_id.clone(),
            span_id: event.span_id.clone(),
            session_id: event.session_id.clone(),
            turn_id: event.turn_id.clone(),
            worker_version: event.worker_version.clone(),
            evidence: bundle.and_then(|bundle| serde_json::to_string(bundle).ok()),
            namespace_ambiguous: event.namespace_ambiguous,
            pending_occurrence_id: pending_occurrence_id.map(str::to_string),
        };
        self.store.record_occurrence(&write).await
    }

    fn tally(&self, report: &mut IngestReport, outcome: RecordOutcome) {
        match outcome {
            RecordOutcome::Created { group_id } => {
                self.counters.add_processed(1);
                report.recorded += 1;
                report.events.push(GroupChanged {
                    group_id,
                    status: GroupStatusV1::New,
                    reason: None,
                    created: true,
                });
            }
            RecordOutcome::Counted {
                group_id,
                status,
                changed,
            } => {
                self.counters.add_processed(1);
                report.recorded += 1;
                report.events.push(GroupChanged {
                    group_id,
                    status,
                    reason: changed.then_some(reason_for(status)),
                    created: false,
                });
            }
            RecordOutcome::Deduped { .. } => {
                self.counters.add_deduped(1);
                report.deduped += 1;
            }
        }
    }

    fn context<'a>(
        &self,
        trace_id: &'a str,
        trace_tags: &'a BTreeMap<String, String>,
        config: &'a WorkerConfig,
    ) -> trace_adapter::TraceContext<'a> {
        trace_adapter::TraceContext {
            trace_id,
            trace_tags,
            own_service: crate::WORKER_NAME,
            ignore_services: &config.ignore_services,
            attribute_allowlist: &config.attribute_allowlist,
            aliases: &config.service_aliases,
        }
    }

    /// The same exclusions, judged from a summary when there is no tree —
    /// which is the only thing a log has to go on.
    fn summary_exclusion(
        &self,
        summary: &TraceSummary,
        config: &WorkerConfig,
    ) -> Option<trace_adapter::Exclusion> {
        if summary
            .session_id()
            .is_some_and(ids::is_investigation_session)
        {
            return Some(trace_adapter::Exclusion::Investigation);
        }
        let service = summary
            .service_name
            .as_deref()
            .map(|name| config.resolve_service_alias(name).to_string())
            .unwrap_or_default();
        let function = summary.function_id.clone().unwrap_or_default();
        let name = summary.name.clone().unwrap_or_default();
        if service == crate::WORKER_NAME
            || trace_adapter::is_own_function(&function, crate::WORKER_NAME)
            || name.starts_with(&format!("fn_queue {}-", crate::WORKER_NAME))
        {
            return Some(trace_adapter::Exclusion::OwnTrace);
        }
        if config.ignore_services.contains(&service) {
            return Some(trace_adapter::Exclusion::IgnoredService);
        }
        None
    }

    fn count_exclusion(&self, exclusion: trace_adapter::Exclusion) {
        match exclusion {
            trace_adapter::Exclusion::OwnTrace => self.counters.add_dropped_own_trace(1),
            trace_adapter::Exclusion::Investigation => self.counters.add_dropped_investigation(1),
            trace_adapter::Exclusion::IgnoredService => {
                self.counters.add_dropped_ignored_service(1)
            }
        }
    }

    async fn summary(&self, trace_id: &str) -> Result<Option<TraceSummary>, SentinelError> {
        match self
            .telemetry
            .traces_patiently(&[trace_id.to_string()])
            .await
        {
            Ok(summaries) => {
                self.counters.set_trace_store(TraceStoreStateV1::Memory);
                Ok(summaries.into_iter().next())
            }
            Err(error) if is_store_disabled(&error) => {
                self.counters.set_trace_store(TraceStoreStateV1::Disabled);
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    async fn refresh_pending_gauge(&self) {
        if let Ok(pending) = self.store.pending_log_count().await {
            self.counters.set_logs_pending_join(pending);
        }
    }

    /// The trace ids of a tick that are worth queueing, and how many were this
    /// worker's own.
    pub async fn filter_tick(&self, trace_ids: &[String]) -> (Vec<String>, u64) {
        let (kept, dropped) = self.ring.filter(trace_ids.iter());
        self.counters.add_phantom_ticks_dropped(dropped);
        (self.errored_traces(kept).await, dropped)
    }

    /// A tick whose engine already named the traces that recorded an error:
    /// queue those, with no read. Reading every active trace on each ~300 ms
    /// tick made the engine decode whole traces several times a second
    /// (MOT-5648).
    pub fn filter_errored_tick(&self, error_trace_ids: &[String]) -> (Vec<String>, u64) {
        let (kept, dropped) = self.ring.filter(error_trace_ids.iter());
        self.counters.add_phantom_ticks_dropped(dropped);
        (kept, dropped)
    }

    /// The traces that hold an error, read in batches so a clean trace costs
    /// no job. One handler reads at a time and takes the ids every other tick
    /// added meanwhile. A failed read gives its ids back to the backlog and
    /// waits before the next try, so a slow engine sees one read, not one per
    /// tick plus one per trace.
    async fn errored_traces(&self, trace_ids: Vec<String>) -> Vec<String> {
        let dropped = self.tick_backlog().push(trace_ids);
        self.counters.add_tick_backlog_dropped(dropped);
        let Ok(_reader) = self.tick_reader.try_lock() else {
            return Vec::new();
        };
        if !self.tick_backlog().ready() {
            return Vec::new();
        }

        let mut errored = Vec::new();
        for _ in 0..TICK_READ_BATCHES {
            let Some(readable_at) = self.tick_backlog().oldest_readable_at() else {
                break;
            };
            tokio::time::sleep_until(readable_at).await;
            let (batch, arrivals): (Vec<String>, Vec<_>) = self
                .tick_backlog()
                .take(TICK_READ_BATCH)
                .into_iter()
                .unzip();
            if batch.is_empty() {
                // A re-tick moved the oldest id back while this slept: wait
                // for its new window rather than leave it to a later tick.
                continue;
            }
            match self.telemetry.traces(&batch).await {
                Ok(summaries) => {
                    self.tick_backlog().succeeded();
                    self.counters.set_trace_store(TraceStoreStateV1::Memory);
                    self.counters
                        .add_lost_before_capture(batch.len().saturating_sub(summaries.len()) as u64);
                    errored.extend(
                        summaries
                            .into_iter()
                            .filter(TraceSummary::holds_error)
                            .map(|summary| summary.trace_id),
                    );
                }
                Err(error) if is_store_disabled(&error) => {
                    self.counters.set_trace_store(TraceStoreStateV1::Disabled);
                    self.tick_backlog().clear();
                    break;
                }
                Err(error) => {
                    let mut backlog = self.tick_backlog();
                    let dropped = backlog.push_front(batch.into_iter().zip(arrivals).collect());
                    let retry_in = backlog.failed();
                    drop(backlog);
                    self.counters.add_tick_backlog_dropped(dropped);
                    tracing::warn!(
                        %error,
                        retry_in_ms = retry_in.as_millis() as u64,
                        "could not read a tick's traces; keeping them for one later read"
                    );
                    break;
                }
            }
        }
        errored
    }

    /// When the tick backlog is next worth a read, if it holds any id: the
    /// later of the backoff after a failure and its oldest id's read window.
    /// Ids a read leaves behind otherwise wait for the next tick, which may
    /// not come once the traces that failed have gone quiet.
    pub fn tick_backlog_due(&self) -> Option<tokio::time::Instant> {
        let backlog = self.tick_backlog();
        let readable = backlog.oldest_readable_at()?;
        Some(
            backlog
                .retry_at
                .map_or(readable, |retry| retry.max(readable)),
        )
    }

    /// Claims the one scheduled read of the backlog; false if one is pending.
    pub fn claim_tick_drain(&self) -> bool {
        !self
            .tick_drain_scheduled
            .swap(true, std::sync::atomic::Ordering::AcqRel)
    }

    pub fn release_tick_drain(&self) {
        self.tick_drain_scheduled
            .store(false, std::sync::atomic::Ordering::Release);
    }

    fn tick_backlog(&self) -> MutexGuard<'_, TickBacklog> {
        self.tick_backlog
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn reason_for(status: GroupStatusV1) -> GroupChangeReasonV1 {
    match status {
        GroupStatusV1::Regressed => GroupChangeReasonV1::Regression,
        GroupStatusV1::New => GroupChangeReasonV1::IgnoreExpired,
        GroupStatusV1::Diagnosed => GroupChangeReasonV1::Diagnosed,
        GroupStatusV1::Resolved => GroupChangeReasonV1::Resolved,
        GroupStatusV1::Ignored => GroupChangeReasonV1::Ignored,
        GroupStatusV1::Investigating => GroupChangeReasonV1::Investigating,
    }
}

/// The engine reports a disabled span store as an error on every read. It is
/// a state, not a failure: the worker keeps running on logs alone.
fn is_store_disabled(error: &SentinelError) -> bool {
    error.to_string().contains("memory_exporter_not_enabled")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_job_carries_the_trace_it_belongs_to() {
        // The queue groups by this field: without it on every variant, a
        // log job and its span job could be handled at the same time.
        let jobs = [
            IngestJob::Trace {
                trace_id: "t1".into(),
            },
            IngestJob::Log {
                trace_id: "t1".into(),
                log: Value::Null,
            },
            IngestJob::Settle {
                trace_id: "t1".into(),
            },
            IngestJob::Promote {
                trace_id: "t1".into(),
                occurrence_id: "occ_1".into(),
            },
        ];
        for job in jobs {
            assert_eq!(job.trace_id(), "t1");
            let encoded = serde_json::to_value(&job).expect("serializes");
            assert_eq!(
                encoded["trace_id"], "t1",
                "the queue reads this field off the top level: {encoded}"
            );
            assert!(encoded["kind"].is_string());
        }
    }

    #[test]
    fn a_disabled_span_store_reads_as_a_state_not_a_failure() {
        let answer = iii_sdk::errors::Error::Remote {
            code: "memory_exporter_not_enabled".into(),
            message: "In-memory span storage is not available.".into(),
            stacktrace: None,
        };
        assert!(is_store_disabled(&SentinelError::dependency(format!(
            "engine::traces::list: {answer}"
        ))));
        assert!(!is_store_disabled(&SentinelError::dependency(
            "connection refused"
        )));
    }
}
