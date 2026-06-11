//! Per-call operational telemetry for the shell worker.
//!
//! The worker already initializes `iii-observability` (OTel meter + tracer +
//! logs bridge) at boot, but emitted zero per-call signal: an operator could
//! not see call rate, latency, error-by-code, or how many background jobs are
//! live. This module closes that gap with a handful of OpenTelemetry metric
//! instruments built once (via `Lazy`) over the already-installed global meter,
//! plus a single `record_call` wrapper that every handler routes through.
//!
//! Design constraints honored here:
//! - Instruments are built once and reused (no per-call allocation of the
//!   instrument itself); attribute vectors are tiny (2-3 `KeyValue`s).
//! - The hot path adds an `Instant::now()`, a classification of the result, one
//!   counter `add`, and one histogram `record`. No clone of payloads/outputs.
//! - The concurrent-jobs gauge reads a plain `AtomicUsize` (see [`crate::jobs`])
//!   in a synchronous callback — never an `.await` — so it cannot deadlock.
//! - When no OTel collector is attached the instruments are silent no-ops, so
//!   nothing here can break boot or fail a call.

use std::future::Future;
use std::time::Instant;

use iii_observability::opentelemetry::metrics::{Counter, Histogram, Meter, ObservableGauge};
use iii_observability::opentelemetry::{global, KeyValue};
use iii_sdk::IIIError;
use once_cell::sync::Lazy;
use tracing::Instrument;

/// The meter name used for every shell instrument. Matches the worker's
/// OTel service identity so dashboards can group on it.
const METER_NAME: &str = "shell";

fn meter() -> Meter {
    global::meter(METER_NAME)
}

/// Per-call counter: one increment per handler invocation, tagged with the
/// function id, the coarse outcome (`ok`/`error`), and the fine-grained code.
static CALLS: Lazy<Counter<u64>> = Lazy::new(|| {
    meter()
        .u64_counter("shell.calls")
        .with_description("Count of shell worker handler calls, by function and outcome/code")
        .build()
});

/// Per-call latency histogram in milliseconds, tagged with function id and
/// outcome so an operator can see p50/p99 latency and spot timeouts.
static CALL_DURATION_MS: Lazy<Histogram<f64>> = Lazy::new(|| {
    meter()
        .f64_histogram("shell.call.duration_ms")
        .with_description("Latency of shell worker handler calls in milliseconds")
        .with_unit("ms")
        .build()
});

/// Counter incremented whenever an exec call truncates stdout or stderr at the
/// configured `max_output_bytes`. A rising rate means agents are producing more
/// output than the worker will return.
static EXEC_OUTPUT_TRUNCATED: Lazy<Counter<u64>> = Lazy::new(|| {
    meter()
        .u64_counter("shell.exec.output_truncated")
        .with_description(
            "Count of exec calls whose stdout/stderr was truncated at max_output_bytes",
        )
        .build()
});

/// Concurrent-jobs gauge. Built once and kept alive for the process lifetime by
/// leaking the handle (the gauge must outlive registration to keep reporting;
/// the worker runs until SIGTERM so this is a one-time, bounded leak). The
/// callback reads a plain atomic — no lock, no `.await` — so it is
/// deadlock-safe regardless of which task the metrics pipeline calls it from.
static JOBS_RUNNING_GAUGE: Lazy<ObservableGauge<u64>> = Lazy::new(|| {
    meter()
        .u64_observable_gauge("shell.jobs.running")
        .with_description("Number of background jobs currently in the Running state")
        .with_callback(|observer| {
            observer.observe(crate::jobs::running_gauge_value() as u64, &[]);
        })
        .build()
});

/// Force-build every instrument once at boot so the first real call does not
/// pay the `Lazy` initialization cost, and so the observable gauge is
/// registered with the meter provider immediately (its callback only fires once
/// it exists). Safe and idempotent; a no-op if OTel is unexported.
pub fn init() {
    Lazy::force(&CALLS);
    Lazy::force(&CALL_DURATION_MS);
    Lazy::force(&EXEC_OUTPUT_TRUNCATED);
    Lazy::force(&JOBS_RUNNING_GAUGE);
}

/// Coarse success/failure label for a call.
pub const OUTCOME_OK: &str = "ok";
pub const OUTCOME_ERROR: &str = "error";

/// Fine-grained code label used when a call fails without a coded remote error
/// (e.g. an argv-parse or allowlist rejection surfaced as `IIIError::Handler`).
pub const CODE_INVOCATION_FAILED: &str = "invocation_failed";

/// Pure classification of a handler result into the `(outcome, code)` pair used
/// as metric attributes. Kept free of any OTel or IO dependency so it can be
/// unit-tested without a meter or a live collector.
///
/// - `Ok(_)`                       -> (`ok`,    `ok`)
/// - `Err(Remote { code, .. })`    -> (`error`, the S-code verbatim)
/// - `Err(any other variant)`      -> (`error`, `invocation_failed`)
pub fn classify<T>(result: &Result<T, IIIError>) -> (&'static str, String) {
    match result {
        Ok(_) => (OUTCOME_OK, OUTCOME_OK.to_string()),
        Err(IIIError::Remote { code, .. }) => (OUTCOME_ERROR, code.clone()),
        Err(_) => (OUTCOME_ERROR, CODE_INVOCATION_FAILED.to_string()),
    }
}

/// Time `fut`, classify its result, emit the per-call counter + latency
/// histogram tagged with `function_id`, and return the result unchanged.
///
/// This is the single observation point wrapped around every handler. It does
/// not alter the handler's value or error in any way — it only reads the
/// `Result` to derive labels. The future is awaited inside an `info_span!`
/// carrying `function_id` so logs/traces emitted by the handler correlate with
/// the same call.
pub async fn record_call<T, F>(function_id: &'static str, fut: F) -> Result<T, IIIError>
where
    F: Future<Output = Result<T, IIIError>>,
{
    // `.instrument()` (not `span.enter()`) attaches the span to the future
    // across await points without holding a `!Send` `Entered` guard, keeping
    // this future `Send` for tokio's multi-threaded runtime.
    let span = tracing::info_span!("shell.call", function_id = function_id);

    let start = Instant::now();
    let result = fut.instrument(span).await;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

    let (outcome, code) = classify(&result);

    CALLS.add(
        1,
        &[
            KeyValue::new("function_id", function_id),
            KeyValue::new("outcome", outcome),
            KeyValue::new("code", code),
        ],
    );
    CALL_DURATION_MS.record(
        elapsed_ms,
        &[
            KeyValue::new("function_id", function_id),
            KeyValue::new("outcome", outcome),
        ],
    );

    result
}

/// Record that an exec call truncated `stdout` and/or `stderr`. Called by the
/// foreground exec handler after it has the typed response in hand (the generic
/// `record_call` wrapper cannot see truncation flags on an opaque `T`). Emits at
/// most one increment per call even when both streams truncate, so the counter
/// reads as "calls that truncated", not "streams truncated".
pub fn record_output_truncated(function_id: &'static str, stdout: bool, stderr: bool) {
    if stdout || stderr {
        EXEC_OUTPUT_TRUNCATED.add(1, &[KeyValue::new("function_id", function_id)]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_ok_is_ok_ok() {
        let result: Result<u8, IIIError> = Ok(7);
        let (outcome, code) = classify(&result);
        assert_eq!(outcome, OUTCOME_OK);
        assert_eq!(code, OUTCOME_OK);
    }

    #[test]
    fn classify_remote_derives_outcome_error_and_the_scode() {
        let result: Result<u8, IIIError> = Err(IIIError::Remote {
            code: "S215".to_string(),
            message: "jail/denylist".to_string(),
            stacktrace: None,
        });
        let (outcome, code) = classify(&result);
        assert_eq!(outcome, OUTCOME_ERROR);
        assert_eq!(code, "S215");
    }

    #[test]
    fn classify_non_coded_error_falls_back_to_invocation_failed() {
        // An argv-parse / allowlist rejection surfaces as Handler (no S-code).
        let result: Result<u8, IIIError> =
            Err(IIIError::Handler("argv: command not allowed".to_string()));
        let (outcome, code) = classify(&result);
        assert_eq!(outcome, OUTCOME_ERROR);
        assert_eq!(code, CODE_INVOCATION_FAILED);

        // A timeout (distinct non-Remote variant) classifies the same way.
        let timed_out: Result<u8, IIIError> = Err(IIIError::Timeout);
        let (outcome, code) = classify(&timed_out);
        assert_eq!(outcome, OUTCOME_ERROR);
        assert_eq!(code, CODE_INVOCATION_FAILED);
    }
}
