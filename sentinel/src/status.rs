//! Live counters behind `sentinel::status`.
//!
//! Every number the operator needs to judge whether the monitor is working is
//! a counter incremented on the path that does the thing — there is no
//! separate bookkeeping pass to drift out of sync. Two of them are the ones
//! that matter when something is wrong: `lost_before_capture` says the
//! engine's ring dropped a trace before the job ran, and `redactions` says
//! the value redactor is actually firing.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicU8, Ordering};

use crate::{
    EngineStatusV1, GroupCountsV1, IngestStatusV1, InvestigationCountsV1, RepositoryStatusV1,
    SourcesStatusV1, StatusResponseV1, TraceStoreStateV1, WorkerConfig,
};

const TRACE_STORE_UNKNOWN: u8 = 0;
const TRACE_STORE_MEMORY: u8 = 1;
const TRACE_STORE_DISABLED: u8 = 2;

/// Shared, lock-free worker counters.
#[derive(Debug)]
pub struct Counters {
    ready: AtomicBool,
    trace_store: AtomicU8,
    engine_logs: AtomicBool,
    queued: AtomicU64,
    processed_total: AtomicU64,
    deduped: AtomicU64,
    dropped_own_trace: AtomicU64,
    dropped_investigation: AtomicU64,
    dropped_ignored_service: AtomicU64,
    phantom_ticks_dropped: AtomicU64,
    logs_pending_join: AtomicU64,
    logs_unattributed: AtomicU64,
    redactions: AtomicU64,
    lost_before_capture: AtomicU64,
    dropped_not_ready: AtomicU64,
    dropped_failed: AtomicU64,
    paused_until_ms: AtomicI64,
}

impl Default for Counters {
    fn default() -> Self {
        Self {
            ready: AtomicBool::new(false),
            trace_store: AtomicU8::new(TRACE_STORE_UNKNOWN),
            // The engine ships with its log store on; a failed
            // `engine::logs::list` corrects this.
            engine_logs: AtomicBool::new(true),
            queued: AtomicU64::new(0),
            processed_total: AtomicU64::new(0),
            deduped: AtomicU64::new(0),
            dropped_own_trace: AtomicU64::new(0),
            dropped_investigation: AtomicU64::new(0),
            dropped_ignored_service: AtomicU64::new(0),
            phantom_ticks_dropped: AtomicU64::new(0),
            logs_pending_join: AtomicU64::new(0),
            logs_unattributed: AtomicU64::new(0),
            redactions: AtomicU64::new(0),
            lost_before_capture: AtomicU64::new(0),
            dropped_not_ready: AtomicU64::new(0),
            dropped_failed: AtomicU64::new(0),
            paused_until_ms: AtomicI64::new(0),
        }
    }
}

macro_rules! counter {
    ($( $field:ident => $add:ident ),+ $(,)?) => {
        impl Counters {
            $(
                pub fn $add(&self, by: u64) {
                    self.$field.fetch_add(by, Ordering::Relaxed);
                }
            )+
        }
    };
}

counter! {
    processed_total => add_processed,
    deduped => add_deduped,
    dropped_own_trace => add_dropped_own_trace,
    dropped_investigation => add_dropped_investigation,
    dropped_ignored_service => add_dropped_ignored_service,
    phantom_ticks_dropped => add_phantom_ticks_dropped,
    logs_unattributed => add_logs_unattributed,
    redactions => add_redactions,
    lost_before_capture => add_lost_before_capture,
    dropped_not_ready => add_dropped_not_ready,
    dropped_failed => add_dropped_failed,
}

impl Counters {
    /// The durable dependencies are claimed; ingest may run.
    pub fn mark_ready(&self) {
        self.ready.store(true, Ordering::Release);
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    pub fn set_trace_store(&self, state: TraceStoreStateV1) {
        let encoded = match state {
            TraceStoreStateV1::Unknown => TRACE_STORE_UNKNOWN,
            TraceStoreStateV1::Memory => TRACE_STORE_MEMORY,
            TraceStoreStateV1::Disabled => TRACE_STORE_DISABLED,
        };
        self.trace_store.store(encoded, Ordering::Relaxed);
    }

    pub fn trace_store(&self) -> TraceStoreStateV1 {
        match self.trace_store.load(Ordering::Relaxed) {
            TRACE_STORE_MEMORY => TraceStoreStateV1::Memory,
            TRACE_STORE_DISABLED => TraceStoreStateV1::Disabled,
            _ => TraceStoreStateV1::Unknown,
        }
    }

    pub fn set_engine_logs(&self, available: bool) {
        self.engine_logs.store(available, Ordering::Relaxed);
    }

    /// Jobs in flight: a gauge, so it goes both ways.
    pub fn add_queued(&self, by: u64) {
        self.queued.fetch_add(by, Ordering::Relaxed);
    }

    pub fn sub_queued(&self, by: u64) {
        let _ = self
            .queued
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_sub(by))
            });
    }

    /// ERROR logs currently holding for the span of their own trace.
    pub fn set_logs_pending_join(&self, pending: u64) {
        self.logs_pending_join.store(pending, Ordering::Relaxed);
    }

    /// Open the ingest breaker until `until_ms`.
    pub fn pause_until(&self, until_ms: i64) {
        self.paused_until_ms.store(until_ms, Ordering::Relaxed);
    }

    /// When the breaker reopens, if it is open.
    pub fn paused_until(&self, now_ms: i64) -> Option<i64> {
        let until = self.paused_until_ms.load(Ordering::Relaxed);
        (until > now_ms).then_some(until)
    }

    pub fn ingest_snapshot(&self, now_ms: i64) -> IngestStatusV1 {
        IngestStatusV1 {
            queued: self.queued.load(Ordering::Relaxed),
            processed_total: self.processed_total.load(Ordering::Relaxed),
            deduped: self.deduped.load(Ordering::Relaxed),
            dropped_own_trace: self.dropped_own_trace.load(Ordering::Relaxed),
            dropped_investigation: self.dropped_investigation.load(Ordering::Relaxed),
            dropped_ignored_service: self.dropped_ignored_service.load(Ordering::Relaxed),
            phantom_ticks_dropped: self.phantom_ticks_dropped.load(Ordering::Relaxed),
            logs_pending_join: self.logs_pending_join.load(Ordering::Relaxed),
            logs_unattributed: self.logs_unattributed.load(Ordering::Relaxed),
            redactions: self.redactions.load(Ordering::Relaxed),
            lost_before_capture: self.lost_before_capture.load(Ordering::Relaxed),
            dropped_not_ready: self.dropped_not_ready.load(Ordering::Relaxed),
            dropped_failed: self.dropped_failed.load(Ordering::Relaxed),
            paused_until: self.paused_until(now_ms),
        }
    }

    /// The whole status surface. Group and investigation counts come from the
    /// caller because they are database aggregates, not counters.
    pub fn snapshot(
        &self,
        config: &WorkerConfig,
        now_ms: i64,
        groups: GroupCountsV1,
        investigations: InvestigationCountsV1,
        config_error: Option<String>,
    ) -> StatusResponseV1 {
        StatusResponseV1 {
            enabled: config.enabled && self.is_ready() && config_error.is_none(),
            engine: EngineStatusV1 {
                trace_store: self.trace_store(),
                logs: self.engine_logs.load(Ordering::Relaxed),
            },
            sources: SourcesStatusV1 {
                trace: config.sources.trace.enabled,
                log: config.sources.log.enabled,
            },
            ingest: self.ingest_snapshot(now_ms),
            groups,
            investigations,
            repositories: config
                .repositories
                .iter()
                .map(|repository| RepositoryStatusV1 {
                    id: repository.id.clone(),
                    path: repository.path.clone(),
                    exists: Path::new(&repository.path).is_dir(),
                    workers: repository.workers.clone(),
                })
                .collect(),
            config_error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RepositoryConfigV1;

    #[test]
    fn a_worker_without_its_dependencies_reports_itself_disabled() {
        let counters = Counters::default();
        let config = WorkerConfig::default();
        let status = counters.snapshot(
            &config,
            1_000,
            GroupCountsV1::default(),
            InvestigationCountsV1::default(),
            None,
        );
        assert!(!status.enabled, "ingest is closed until the store is ready");
        assert_eq!(status.engine.trace_store, TraceStoreStateV1::Unknown);

        counters.mark_ready();
        let status = counters.snapshot(
            &config,
            1_000,
            GroupCountsV1::default(),
            InvestigationCountsV1::default(),
            None,
        );
        assert!(status.enabled);
    }

    #[test]
    fn an_operator_disabled_worker_stays_disabled_once_ready() {
        let counters = Counters::default();
        counters.mark_ready();
        let config = WorkerConfig {
            enabled: false,
            ..WorkerConfig::default()
        };
        let status = counters.snapshot(
            &config,
            1_000,
            GroupCountsV1::default(),
            InvestigationCountsV1::default(),
            None,
        );
        assert!(!status.enabled);
    }

    #[test]
    fn a_refused_configuration_disables_the_worker_and_says_so() {
        let counters = Counters::default();
        counters.mark_ready();
        let status = counters.snapshot(
            &WorkerConfig::default(),
            0,
            GroupCountsV1::default(),
            InvestigationCountsV1::default(),
            Some("workers_ttl_ms must be at least 1000".into()),
        );
        assert!(!status.enabled);
        assert_eq!(
            status.config_error.as_deref(),
            Some("workers_ttl_ms must be at least 1000")
        );
    }

    #[test]
    fn the_breaker_window_closes_on_its_own() {
        let counters = Counters::default();
        assert_eq!(counters.paused_until(1_000), None);
        counters.pause_until(31_000);
        assert_eq!(counters.paused_until(1_000), Some(31_000));
        assert_eq!(counters.paused_until(31_000), None, "the window is over");
    }

    #[test]
    fn the_queue_gauge_never_goes_negative() {
        let counters = Counters::default();
        counters.add_queued(2);
        counters.sub_queued(5);
        assert_eq!(counters.ingest_snapshot(0).queued, 0);
    }

    #[test]
    fn repository_status_reports_whether_the_checkout_is_here() {
        let counters = Counters::default();
        let config = WorkerConfig {
            repositories: vec![
                RepositoryConfigV1 {
                    id: "here".into(),
                    path: env!("CARGO_MANIFEST_DIR").into(),
                    workers: vec!["sentinel".into()],
                },
                RepositoryConfigV1 {
                    id: "gone".into(),
                    path: "/nonexistent/checkout".into(),
                    workers: vec!["ghost".into()],
                },
            ],
            ..WorkerConfig::default()
        };
        let status = counters.snapshot(
            &config,
            0,
            GroupCountsV1::default(),
            InvestigationCountsV1::default(),
            None,
        );
        assert!(status.repositories[0].exists);
        assert!(!status.repositories[1].exists);
    }
}
