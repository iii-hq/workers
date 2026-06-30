//! OpenTelemetry metrics for the workflow worker.
//!
//! Instruments bind to the global meter on FIRST EMIT (after the worker is
//! connected and the provider installed by `OtelConfig` in `main`), which
//! avoids the boot-time race where forcing before the provider is installed
//! would bind to the no-op meter. They are silent no-ops when no collector is
//! attached, so nothing here can break boot or fail a request. Orchestration
//! code calls the `record_*` helpers; OTel types never leak out of this module.
//!
//! Cardinality: only bounded labels (status/action/outcome). Never tag a
//! metric with run_id / node_uid / session_id.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;

use iii_helpers::observability::opentelemetry::metrics::{Counter, Histogram, ObservableGauge};
use iii_helpers::observability::opentelemetry::{global, KeyValue};

use crate::types::RunStatus;

const METER_NAME: &str = "workflow";

static RUNS_STARTED: LazyLock<Counter<u64>> = LazyLock::new(|| {
    global::meter(METER_NAME)
        .u64_counter("workflow.runs.started")
        .with_description("Workflow runs accepted by start (after validation/idempotency)")
        .build()
});

static RUNS_TERMINAL: LazyLock<Counter<u64>> = LazyLock::new(|| {
    global::meter(METER_NAME)
        .u64_counter("workflow.runs.terminal")
        .with_description("Workflow runs reaching a terminal state, by status")
        .build()
});

static RUN_DURATION_MS: LazyLock<Histogram<f64>> = LazyLock::new(|| {
    global::meter(METER_NAME)
        .f64_histogram("workflow.run.duration_ms")
        .with_description("Wall-clock duration of a run from creation to terminal")
        .with_unit("ms")
        .build()
});

static NODE_TIMEOUTS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    global::meter(METER_NAME)
        .u64_counter("workflow.nodes.timeouts")
        .with_description("Node timeout actions taken by the sweep, by action")
        .build()
});

static NODE_DURATION_MS: LazyLock<Histogram<f64>> = LazyLock::new(|| {
    global::meter(METER_NAME)
        .f64_histogram("workflow.node.duration_ms")
        .with_description("Duration a node spent Running until it reached a terminal state")
        .with_unit("ms")
        .build()
});

/// Backing value for the active-runs gauge; refreshed by the sweep each cycle.
static ACTIVE_RUNS: AtomicU64 = AtomicU64::new(0);

static RUNS_ACTIVE_GAUGE: LazyLock<ObservableGauge<u64>> = LazyLock::new(|| {
    global::meter(METER_NAME)
        .u64_observable_gauge("workflow.runs.active")
        .with_description("Non-terminal workflow runs (refreshed each cron sweep)")
        .with_callback(|observer| observer.observe(ACTIVE_RUNS.load(Ordering::Relaxed), &[]))
        .build()
});

// --- pure label mappers (unit-tested; no OTel/IO dependency) ---

pub fn status_label(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Running => "running",
        RunStatus::AwaitingNodes => "awaiting_nodes",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
        RunStatus::Cancelled => "cancelled",
    }
}

pub fn action_label(refire: bool) -> &'static str {
    if refire {
        "refire"
    } else {
        "failout"
    }
}

pub fn outcome_label(done: bool) -> &'static str {
    if done {
        "done"
    } else {
        "failed"
    }
}

// --- emit helpers (called by handlers) ---

/// Counts only newly-accepted runs (not idempotency hits, and not crash-recovery
/// re-drives, which can reach terminal with no `started` in this process). So
/// `runs.started` and `runs.terminal` are NOT a closed funnel — for in-flight
/// count use the `workflow.runs.active` gauge, not `started - terminal`.
pub fn record_run_started() {
    RUNS_STARTED.add(1, &[]);
}

pub fn record_run_terminal(status: RunStatus, duration_ms: f64) {
    let s = status_label(status);
    RUNS_TERMINAL.add(1, &[KeyValue::new("status", s)]);
    RUN_DURATION_MS.record(duration_ms, &[KeyValue::new("status", s)]);
}

pub fn record_node_terminal(done: bool, duration_ms: f64) {
    NODE_DURATION_MS.record(
        duration_ms,
        &[KeyValue::new("outcome", outcome_label(done))],
    );
}

pub fn record_timeout(refire: bool) {
    NODE_TIMEOUTS.add(1, &[KeyValue::new("action", action_label(refire))]);
}

pub fn set_active_runs(n: u64) {
    ACTIVE_RUNS.store(n, Ordering::Relaxed);
    // Force-register the observable gauge on the first sweep (well after boot)
    // so its callback binds to the installed meter provider, not the no-op meter
    // a boot-time force could capture.
    LazyLock::force(&RUNS_ACTIVE_GAUGE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_label_covers_every_variant() {
        assert_eq!(status_label(RunStatus::Running), "running");
        assert_eq!(status_label(RunStatus::AwaitingNodes), "awaiting_nodes");
        assert_eq!(status_label(RunStatus::Completed), "completed");
        assert_eq!(status_label(RunStatus::Failed), "failed");
        assert_eq!(status_label(RunStatus::Cancelled), "cancelled");
    }

    #[test]
    fn action_and_outcome_labels() {
        assert_eq!(action_label(true), "refire");
        assert_eq!(action_label(false), "failout");
        assert_eq!(outcome_label(true), "done");
        assert_eq!(outcome_label(false), "failed");
    }
}
