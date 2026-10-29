//! The observers this worker keeps, in step with the configuration.
//!
//! Every observer lives in a slot that follows the live configuration:
//! observed when its source is enabled, disposed when it is turned off. An
//! observer whose function is not registered yet — the harness or `cron` is
//! not up — stays pending and becomes active when that worker registers it,
//! so a monitor never has to retry its way back to watching.

use iii_config_client::ObserverSlot;
use iii_sdk::{Deliver, IIIClient, Source};
use serde_json::json;

use crate::{functions, WorkerConfig};

/// The engine's own sources, hosted in the `default` namespace.
pub const ON_SPAN_FN: &str = "observability::on-span";
pub const ON_LOG_FN: &str = "observability::on-log";
/// Hosted by other workers.
pub const ON_TURN_COMPLETED_FN: &str = "harness::on-turn-completed";
pub const ON_TICK_FN: &str = "cron::on-tick";

/// OpenTelemetry `severity_number` of an ERROR record; ERROR and FATAL are at
/// or above it.
pub const ERROR_SEVERITY_NUMBER: i32 = 17;

#[derive(Clone, Default)]
pub struct Observers {
    trace: ObserverSlot,
    log: ObserverSlot,
    turn_completed: ObserverSlot,
    prune: ObserverSlot,
}

impl Observers {
    /// Reconcile every observer with the live configuration. Idempotent:
    /// called on every pass of the recovery loop, so a settings change takes
    /// effect within one pass.
    pub fn reconcile(&self, iii: &IIIClient, config: &WorkerConfig) {
        let enabled = config.enabled;

        self.trace.reconcile(
            enabled && config.sources.trace.enabled,
            || iii.observe(span_source(), functions::ON_TRACE_ACTIVITY_ID),
            "watching span activity for errors",
            "stopped watching span activity",
        );

        self.log.reconcile(
            enabled && config.sources.log.enabled,
            || iii.observe(log_source(), functions::ON_LOG_ID),
            "watching error logs",
            "stopped watching error logs",
        );

        self.turn_completed.reconcile(
            enabled,
            || iii.observe(turn_completed_source(), functions::ON_TURN_COMPLETED_ID),
            "listening for finished investigation turns",
            "stopped listening for finished turns",
        );

        self.prune.reconcile(
            enabled,
            || {
                iii.observe(
                    prune_source(&config.retention.cron),
                    functions::ON_SCHEDULE_ID,
                )
            },
            "daily prune scheduled",
            "daily prune unscheduled",
        );
    }

    /// Whether the trace and log sources are observed, for the status
    /// surface.
    pub fn observing(&self) -> (bool, bool) {
        (self.trace.is_observing(), self.log.is_observing())
    }
}

/// Every span-activity tick. The event carries trace ids only, so the
/// target reads each trace and records it only when it holds an error.
pub fn span_source() -> Source {
    Source::function(ON_SPAN_FN)
        .namespace("default")
        .deliver(Deliver::Output)
}

/// ERROR (and FATAL) log records.
pub fn log_source() -> Source {
    Source::function(ON_LOG_FN)
        .namespace("default")
        .filters(json!({ "input.severity_number": { "$gte": ERROR_SEVERITY_NUMBER } }))
        .deliver(Deliver::Output)
}

/// Every finished harness turn; the target re-reads the status of the ones
/// that belong to an investigation.
pub fn turn_completed_source() -> Source {
    Source::function(ON_TURN_COMPLETED_FN).deliver(Deliver::Output)
}

/// The retention prune's cron schedule.
pub fn prune_source(expression: &str) -> Source {
    Source::function(ON_TICK_FN)
        .args(json!({ "expression": expression }))
        .deliver(Deliver::Output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn error_logs_are_selected_by_severity() {
        assert_eq!(
            log_source(),
            Source::function("observability::on-log")
                .namespace("default")
                .filters(json!({ "input.severity_number": { "$gte": 17 } }))
                .deliver(Deliver::Output)
        );
    }

    #[test]
    fn the_prune_observes_cron_with_its_expression() {
        assert_eq!(
            prune_source("0 0 3 * * *"),
            Source::function("cron::on-tick")
                .args(json!({ "expression": "0 0 3 * * *" }))
                .deliver(Deliver::Output)
        );
    }

    #[tokio::test]
    async fn sources_follow_the_configuration() {
        // `IIIClient::new` builds local state only: observers stay pending.
        let iii = Arc::new(IIIClient::new("ws://127.0.0.1:1"));
        let observers = Observers::default();

        let mut config = WorkerConfig {
            enabled: true,
            ..WorkerConfig::default()
        };
        config.sources.log.enabled = false;
        observers.reconcile(&iii, &config);
        assert_eq!(observers.observing(), (true, false));

        config.enabled = false;
        observers.reconcile(&iii, &config);
        assert_eq!(observers.observing(), (false, false));
    }
}
