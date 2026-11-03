//! SDK-facing controlled functions and event-driven scenario observers.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::{Deliver, ObserverHandle, RegisterFunction, Source};
use serde_json::{json, Value};

use crate::client::Client;
use crate::deadline::Deadline;
use crate::types::probe::{ControlledTargetV1, LifecycleAcceptedV1, LifecycleEventV1};
use crate::types::trace::strip_engine_fields;

pub(crate) const LIFECYCLE_FUNCTION_ID: &str = "integration-probe::turn-completed";
const READY_FUNCTION_ID: &str = "integration-probe::harness-ready";
const TRACES_FUNCTION_ID: &str = "integration-probe::traces-changed";

/// The harness's turn-completion event, filtered per session.
const ON_TURN_COMPLETED_FN: &str = "harness::on-turn-completed";
/// The harness's readiness event.
const ON_READY_FN: &str = "harness::on-ready";
/// The engine's span-store change event (hosted in `default`).
const ON_SPAN_FN: &str = "observability::on-span";
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub(crate) struct CompletionObservation {
    pub event: LifecycleEventV1,
}

/// One observer the probe created, by the function it observes.
#[derive(Clone)]
struct BoundObserver {
    function_id: &'static str,
    handle: ObserverHandle,
}

pub struct ScenarioProbe {
    client: Client,
    completions: Arc<Mutex<Vec<CompletionObservation>>>,
    completion_notify: Arc<tokio::sync::Notify>,
    ready: Arc<AtomicBool>,
    ready_notify: Arc<tokio::sync::Notify>,
    trace_generation: Arc<AtomicU64>,
    trace_notify: Arc<tokio::sync::Notify>,
    bindings: Mutex<Vec<BoundObserver>>,
    /// Every invocation payload the controlled function served, in arrival
    /// order. Probe-side WHOLE-RUN evidence: the serving process sees every
    /// execution regardless of which session (or no session at all — e.g. a
    /// call-mode reaction) dispatched it, where session-scoped trace
    /// collection cannot.
    target_calls: Arc<Mutex<Vec<Value>>>,
    target_notify: Arc<tokio::sync::Notify>,
    target_response_released: Arc<AtomicBool>,
    target_response_notify: Arc<tokio::sync::Notify>,
}

impl ScenarioProbe {
    pub async fn start(ws_url: &str) -> anyhow::Result<Self> {
        let probe = Self {
            client: Client::connect(ws_url, "integration-probe"),
            completions: Arc::new(Mutex::new(Vec::new())),
            completion_notify: Arc::new(tokio::sync::Notify::new()),
            ready: Arc::new(AtomicBool::new(false)),
            ready_notify: Arc::new(tokio::sync::Notify::new()),
            trace_generation: Arc::new(AtomicU64::new(0)),
            trace_notify: Arc::new(tokio::sync::Notify::new()),
            bindings: Mutex::new(Vec::new()),
            target_calls: Arc::new(Mutex::new(Vec::new())),
            target_notify: Arc::new(tokio::sync::Notify::new()),
            target_response_released: Arc::new(AtomicBool::new(false)),
            target_response_notify: Arc::new(tokio::sync::Notify::new()),
        };
        probe.register_sinks();
        Ok(probe)
    }

    fn register_sinks(&self) {
        self.register_completion_sink();
        self.register_ready_sink();
        self.register_trace_sink();
    }

    fn register_completion_sink(&self) {
        let completions = Arc::clone(&self.completions);
        let completion_notify = Arc::clone(&self.completion_notify);
        self.client.inner().register_function(
            LIFECYCLE_FUNCTION_ID,
            RegisterFunction::new_async(move |payload: Value| {
                let completions = Arc::clone(&completions);
                let completion_notify = Arc::clone(&completion_notify);
                async move {
                    let event = parse_lifecycle_payload(payload).map_err(Error::Handler)?;
                    let observation = CompletionObservation { event };
                    completions
                        .lock()
                        .map_err(|_| Error::Handler("integration/completion_lock_poisoned".into()))?
                        .push(observation);
                    completion_notify.notify_waiters();
                    serde_json::to_value(LifecycleAcceptedV1 { accepted: true }).map_err(|error| {
                        Error::Handler(format!(
                            "integration/completion_response_serialize: {error}"
                        ))
                    })
                }
            })
            .description("Completion signal for harness integration scenarios.")
            .request_format(schema_for::<LifecycleEventV1>())
            .response_format(schema_for::<LifecycleAcceptedV1>()),
        );
    }

    fn register_ready_sink(&self) {
        let ready = Arc::clone(&self.ready);
        let ready_notify = Arc::clone(&self.ready_notify);
        self.client.inner().register_function(
            READY_FUNCTION_ID,
            RegisterFunction::new_async(move |payload: Value| {
                let ready = Arc::clone(&ready);
                let ready_notify = Arc::clone(&ready_notify);
                async move {
                    parse_ready_payload(&payload).map_err(Error::Handler)?;
                    ready.store(true, Ordering::Release);
                    ready_notify.notify_waiters();
                    Ok::<Value, Error>(json!({ "accepted": true }))
                }
            })
            .description("Readiness signal for harness integration scenarios."),
        );
    }

    fn register_trace_sink(&self) {
        let trace_generation = Arc::clone(&self.trace_generation);
        let trace_notify = Arc::clone(&self.trace_notify);
        self.client.inner().register_function(
            TRACES_FUNCTION_ID,
            RegisterFunction::new_async(move |payload: Value| {
                let trace_generation = Arc::clone(&trace_generation);
                let trace_notify = Arc::clone(&trace_notify);
                async move {
                    parse_trace_payload(&payload).map_err(Error::Handler)?;
                    trace_generation.fetch_add(1, Ordering::AcqRel);
                    trace_notify.notify_waiters();
                    Ok::<Value, Error>(json!({ "accepted": true }))
                }
            })
            .description("Trace-store change signal for harness integration scenarios."),
        );
    }

    pub fn register_target(
        &self,
        run_id: &str,
        target: Option<&ControlledTargetV1>,
    ) -> anyhow::Result<()> {
        let Some(target) = target else {
            return Ok(());
        };
        let prefix = format!("{run_id}::");
        anyhow::ensure!(
            target.function_id.starts_with(&prefix),
            "integration/target_scope: {} must be prefixed by {prefix}",
            target.function_id
        );
        register_controlled_function(
            self.client.inner(),
            target,
            Arc::clone(&self.target_calls),
            Arc::clone(&self.target_notify),
            Arc::clone(&self.target_response_released),
            Arc::clone(&self.target_response_notify),
        );
        Ok(())
    }

    /// Snapshot of every controlled-function invocation payload so far
    /// (engine fields stripped), in arrival order.
    pub fn target_calls(&self) -> Vec<Value> {
        self.target_calls
            .lock()
            .map(|calls| calls.clone())
            .unwrap_or_default()
    }

    /// Wait until the controlled function has served at least `expected`
    /// invocations. This is how a scenario awaits work that produces no
    /// session turn — a call-mode reaction's dispatch, including the trailing
    /// coalesced fire that lands only after the rate-cap window frees.
    pub async fn wait_for_target_calls(
        &self,
        expected: usize,
        deadline: Deadline,
    ) -> anyhow::Result<Vec<Value>> {
        anyhow::ensure!(expected > 0, "expected call count must be positive");
        loop {
            let notified = self.target_notify.notified();
            let calls = self.target_calls();
            if calls.len() >= expected {
                return Ok(calls);
            }
            deadline
                .timeout("controlled-function call count", notified)
                .await?;
        }
    }

    /// Release a controlled target held at the response boundary. Idempotent
    /// so fault-error cleanup and normal cleanup can both call it safely.
    pub fn release_target_response(&self) {
        self.target_response_released.store(true, Ordering::Release);
        self.target_response_notify.notify_waiters();
    }

    /// Observers live in the engine. Once that process restarts, dispose
    /// the probe's handles (the SDK would replay them) before observing anew.
    pub async fn forget_observer_bindings(&self) -> anyhow::Result<()> {
        let bindings = std::mem::take(
            &mut *self
                .bindings
                .lock()
                .map_err(|_| anyhow::anyhow!("integration/bindings_lock_poisoned"))?,
        );
        for binding in bindings {
            let _ = binding.handle.dispose().await;
        }
        Ok(())
    }

    /// Observe the span store, this session's turn completions and the
    /// harness's readiness. The span observer is engine-hosted, so awaiting it
    /// is also a barrier for the function registrations queued before it; the
    /// harness's observers stay `pending` until the harness registers its
    /// functions and activate on their own.
    pub async fn bind_observers(&self, session_id: &str, deadline: Deadline) -> anyhow::Result<()> {
        let spans = self.observe(
            Source::function(ON_SPAN_FN).namespace("default"),
            ON_SPAN_FN,
            TRACES_FUNCTION_ID,
        )?;
        await_active(&spans, deadline).await?;
        self.observe(
            Source::function(ON_TURN_COMPLETED_FN)
                .filters(json!({ "input.session_id": { "$eq": session_id } })),
            ON_TURN_COMPLETED_FN,
            LIFECYCLE_FUNCTION_ID,
        )?;
        self.observe(
            Source::function(ON_READY_FN),
            ON_READY_FN,
            READY_FUNCTION_ID,
        )?;
        Ok(())
    }

    /// Add a second completion observer for direct children of `session_id`.
    /// The root observer remains session-scoped; the parent filter captures
    /// every in-turn `harness::spawn` child without needing runtime child ids
    /// before the first generation runs. A spawned child's event names its
    /// parent in `parent.session_id`, a displayed child's in
    /// `parent_session_id`; the filter takes either.
    pub async fn bind_child_completion_observer(
        &self,
        parent_session_id: &str,
        deadline: Deadline,
    ) -> anyhow::Result<()> {
        let children = self.observe(
            Source::function(ON_TURN_COMPLETED_FN).filters(json!({ "$or": [
                { "input.parent.session_id": { "$eq": parent_session_id } },
                { "input.parent_session_id": { "$eq": parent_session_id } },
            ] })),
            ON_TURN_COMPLETED_FN,
            LIFECYCLE_FUNCTION_ID,
        )?;
        await_active(&children, deadline).await
    }

    /// Once readiness proves the harness registered its functions, wait for
    /// every completion observer to be active before any stimulus.
    pub async fn confirm_completion_binding(
        &self,
        _session_id: &str,
        deadline: Deadline,
    ) -> anyhow::Result<()> {
        let completions = self
            .bindings
            .lock()
            .map_err(|_| anyhow::anyhow!("integration/bindings_lock_poisoned"))?
            .iter()
            .filter(|binding| binding.function_id == ON_TURN_COMPLETED_FN)
            .map(|binding| binding.handle.clone())
            .collect::<Vec<_>>();
        for handle in completions {
            await_active(&handle, deadline).await?;
        }
        Ok(())
    }

    /// Observe `source` with `target`, delivering the event itself.
    fn observe(
        &self,
        source: Source,
        function_id: &'static str,
        target: &'static str,
    ) -> anyhow::Result<ObserverHandle> {
        let handle = self
            .client
            .inner()
            .observe(source.deliver(Deliver::Output), target);
        self.bindings
            .lock()
            .map_err(|_| anyhow::anyhow!("integration/bindings_lock_poisoned"))?
            .push(BoundObserver {
                function_id,
                handle: handle.clone(),
            });
        Ok(handle)
    }

    pub async fn wait_until_ready(&self, deadline: Deadline) -> anyhow::Result<()> {
        loop {
            let ready_notified = self.ready_notify.notified();
            if self.ready.load(Ordering::Acquire) {
                return Ok(());
            }
            deadline
                .timeout("harness readiness signal", ready_notified)
                .await?;
        }
    }

    pub async fn wait_for_completion(
        &self,
        deadline: Deadline,
    ) -> anyhow::Result<CompletionObservation> {
        let observations = self.wait_for_completion_turns(1, deadline).await?;
        latest_terminal_observation(&observations)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("completion wait returned no events"))
    }

    pub async fn wait_for_completion_turns(
        &self,
        expected: usize,
        deadline: Deadline,
    ) -> anyhow::Result<Vec<CompletionObservation>> {
        anyhow::ensure!(expected > 0, "expected completion count must be positive");
        loop {
            let notified = self.completion_notify.notified();
            let observations = self
                .completions
                .lock()
                .map_err(|_| anyhow::anyhow!("completion observer lock poisoned"))?
                .clone();
            let turns = observations
                .iter()
                .filter(|observation| observation.event.terminal)
                .map(|observation| observation.event.turn_id.as_str())
                .collect::<BTreeSet<_>>();
            if turns.len() >= expected {
                return Ok(observations);
            }
            deadline
                .timeout("terminal turn completion", notified)
                .await?;
        }
    }

    pub async fn wait_for_trace_change(
        &self,
        after_generation: u64,
        deadline: Deadline,
    ) -> anyhow::Result<u64> {
        wait_for_trace_generation(
            &self.trace_generation,
            &self.trace_notify,
            after_generation,
            deadline,
        )
        .await
    }

    /// Snapshot the trace notification counter before a scenario can emit any
    /// subject traces. Collection later waits for a generation newer than this
    /// baseline, independent of lifecycle callback ordering.
    pub fn current_trace_generation(&self) -> u64 {
        self.trace_generation.load(Ordering::Acquire)
    }

    pub async fn shutdown(&self) {
        self.release_target_response();
        let bindings = self
            .bindings
            .lock()
            .map(|mut bindings| std::mem::take(&mut *bindings))
            .unwrap_or_default();
        let deadline = Deadline::after(SHUTDOWN_TIMEOUT);
        for binding in bindings.into_iter().rev() {
            match deadline
                .timeout("observer cleanup", binding.handle.dispose())
                .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::warn!(
                    function_id = binding.function_id,
                    %error,
                    "integration observer cleanup failed"
                ),
                Err(error) => tracing::warn!(
                    function_id = binding.function_id,
                    %error,
                    "integration observer cleanup timed out"
                ),
            }
        }
        self.client.shutdown().await;
    }
}

/// Wait until `handle` is active; fails with the observer's reason when the
/// engine or the function's worker refused it.
async fn await_active(handle: &ObserverHandle, deadline: Deadline) -> anyhow::Result<()> {
    deadline
        .timeout("observer activation", handle.ready())
        .await?
        .map_err(|error| anyhow::anyhow!("observer {} failed: {error}", handle.id()))
}

async fn wait_for_trace_generation(
    trace_generation: &AtomicU64,
    trace_notify: &tokio::sync::Notify,
    after_generation: u64,
    deadline: Deadline,
) -> anyhow::Result<u64> {
    loop {
        let notified = trace_notify.notified();
        let generation = trace_generation.load(Ordering::Acquire);
        if generation > after_generation {
            return Ok(generation);
        }
        deadline.timeout("trace-store change", notified).await?;
    }
}

pub(crate) fn latest_terminal_observation(
    observations: &[CompletionObservation],
) -> Option<&CompletionObservation> {
    observations
        .iter()
        .filter(|observation| observation.event.terminal)
        .max_by(|left, right| {
            left.event
                .timestamp
                .cmp(&right.event.timestamp)
                .then_with(|| left.event.turn_id.cmp(&right.event.turn_id))
        })
}

fn parse_lifecycle_payload(mut payload: Value) -> Result<LifecycleEventV1, String> {
    strip_engine_fields(&mut payload);
    serde_json::from_value(payload)
        .map_err(|error| format!("integration/lifecycle_contract: {error}"))
}

fn parse_ready_payload(payload: &Value) -> Result<(), String> {
    if payload.get("status").and_then(Value::as_str) == Some("ready") {
        Ok(())
    } else {
        Err("integration/ready_contract: expected status=ready".to_string())
    }
}

fn parse_trace_payload(payload: &Value) -> Result<(), String> {
    let trace_ids = payload
        .get("trace_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| "integration/trace_contract: trace_ids must be an array".to_string())?;
    if trace_ids.iter().all(|trace_id| trace_id.as_str().is_some()) {
        Ok(())
    } else {
        Err("integration/trace_contract: trace_ids must contain strings".to_string())
    }
}

fn register_controlled_function(
    iii: &iii_sdk::IIIClient,
    target: &ControlledTargetV1,
    calls: Arc<Mutex<Vec<Value>>>,
    notify: Arc<tokio::sync::Notify>,
    response_released: Arc<AtomicBool>,
    response_notify: Arc<tokio::sync::Notify>,
) {
    let response = target.response.clone();
    let hold_response = target.hold_response;
    iii.register_function(
        &target.function_id,
        RegisterFunction::new_async(move |mut payload: Value| {
            let response = response.clone();
            let calls = Arc::clone(&calls);
            let notify = Arc::clone(&notify);
            let response_released = Arc::clone(&response_released);
            let response_notify = Arc::clone(&response_notify);
            async move {
                strip_engine_fields(&mut payload);
                calls
                    .lock()
                    .map_err(|_| Error::Handler("integration/target_calls_lock_poisoned".into()))?
                    .push(payload);
                notify.notify_waiters();
                while hold_response && !response_released.load(Ordering::Acquire) {
                    let released = response_notify.notified();
                    if response_released.load(Ordering::Acquire) {
                        break;
                    }
                    released.await;
                }
                Ok::<Value, Error>(response)
            }
        })
        .description(target.description.clone())
        .request_format(Value::Object(target.request_schema.clone()))
        .response_format(const_response_schema(&target.response)),
    );
}

fn const_response_schema(response: &Value) -> Value {
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "const": response
    })
}

fn schema_for<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("JSON schema serializes")
}

#[cfg(test)]
mod tests {
    use crate::types::probe::{LifecycleEventV1, LifecycleStatusV1};

    use super::*;

    #[test]
    fn lifecycle_parser_removes_engine_fields_and_keeps_strict_shape() {
        let event = parse_lifecycle_payload(json!({
            "session_id": "session-1",
            "turn_id": "turn-1",
            "status": "completed",
            "terminal": true,
            "timestamp": 1,
            "_caller_worker_id": "engine"
        }))
        .unwrap();
        assert_eq!(event.status, LifecycleStatusV1::Completed);
        assert!(event.terminal);

        let error = parse_lifecycle_payload(json!({
            "session_id": "session-1",
            "turn_id": "turn-1",
            "status": "completed",
            "terminal": true,
            "timestamp": 1,
            "unexpected": true
        }))
        .unwrap_err();
        assert!(error.contains("unknown field"), "{error}");
    }

    #[test]
    fn observer_payload_parsers_reject_malformed_events() {
        assert!(parse_ready_payload(&json!({ "status": "ready" })).is_ok());
        assert!(parse_ready_payload(&json!({ "status": "booting" })).is_err());

        assert!(parse_trace_payload(&json!({ "trace_ids": ["abc"] })).is_ok());
        assert!(parse_trace_payload(&json!({ "trace_ids": [1] })).is_err());
    }

    #[test]
    fn latest_terminal_completion_uses_event_time_not_delivery_order() {
        let observation = |turn_id: &str, timestamp: i64, terminal: bool| CompletionObservation {
            event: LifecycleEventV1 {
                session_id: "session-1".into(),
                turn_id: turn_id.into(),
                status: LifecycleStatusV1::Completed,
                terminal,
                timestamp,
                result: None,
                result_error: None,
                reason: None,
                parent: None,
                parent_session_id: None,
                reactive_depth: None,
                context: None,
            },
        };
        let observations = vec![
            observation("turn-new", 20, true),
            observation("turn-old", 10, true),
            observation("turn-progress", 30, false),
        ];

        let latest = latest_terminal_observation(&observations).unwrap();
        assert_eq!(latest.event.turn_id, "turn-new");
    }

    #[tokio::test]
    async fn trace_received_before_completion_is_visible_from_the_pre_stimulus_baseline() {
        let generation = AtomicU64::new(0);
        let notify = tokio::sync::Notify::new();
        let pre_stimulus_baseline = generation.load(Ordering::Acquire);

        // The trace callback wins the race and runs before lifecycle delivery.
        generation.fetch_add(1, Ordering::AcqRel);
        notify.notify_waiters();

        let observed = wait_for_trace_generation(
            &generation,
            &notify,
            pre_stimulus_baseline,
            Deadline::after(Duration::from_millis(20)),
        )
        .await
        .unwrap();
        assert_eq!(observed, 1);
    }
}
