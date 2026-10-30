//! The demo counter provider: `demo::on-counter-changed` with its observer
//! hooks, and the `demo::counter::*` functions around it. `main.rs` runs it
//! as a worker; `tests/counter_provider.rs` hosts it in process.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use iii_sdk::{
    Call, CallAction, CallRequest, Error, IIIClient, Keys, Match, ObserveRejection, Observer,
    RegisterFunction,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const ON_COUNTER_CHANGED_FN: &str = "demo::on-counter-changed";
/// Bound fan-out: observers beyond this are rejected when they are added.
pub const MAX_OBSERVERS: usize = 64;

/// Observer args of `demo::on-counter-changed`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CounterArgs {
    /// Counter whose changes are delivered.
    counter: String,
}

/// The event `demo::on-counter-changed` returns: one committed bump.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct CounterChanged {
    /// Counter that changed.
    counter: String,
    /// Revision of the counter after the bump.
    revision: u64,
}

/// Request naming one counter.
#[derive(Debug, Deserialize, JsonSchema)]
struct CounterRequest {
    /// Counter name.
    counter: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct Empty {}

#[derive(Default)]
struct Provider {
    /// counter -> (value, revision); the source of truth the query reads.
    counters: Mutex<HashMap<String, (u64, u64)>>,
}

impl Provider {
    fn read(&self, counter: &str) -> Value {
        let (value, revision) = self
            .counters
            .lock()
            .unwrap()
            .get(counter)
            .copied()
            .unwrap_or_default();
        json!({ "counter": counter, "value": value, "revision": revision })
    }
}

/// Observers of `counter` among the active ones.
fn observers_of(iii: &IIIClient, counter: &str) -> usize {
    iii.observers(ON_COUNTER_CHANGED_FN)
        .iter()
        .filter(|obs| obs.args_value()["counter"].as_str() == Some(counter))
        .count()
}

/// Validate an observer's args and cap the number of observers. The SDK runs
/// a function's observer hooks one at a time and activates an accepted
/// observer before the next hook runs, so the active observers are every
/// observer accepted before this one and not removed since.
fn accept(iii: &IIIClient, obs: &Observer) -> Result<(), ObserveRejection> {
    let args: CounterArgs = obs.args()?;
    if args.counter.trim().is_empty() || args.counter.len() > 64 {
        return Err(ObserveRejection::new(
            "INVALID_ARGS",
            "counter must be 1-64 bytes",
        ));
    }
    if iii.observers(ON_COUNTER_CHANGED_FN).len() >= MAX_OBSERVERS {
        return Err(ObserveRejection::new(
            "OBSERVER_LIMIT",
            format!("{ON_COUNTER_CHANGED_FN} takes at most {MAX_OBSERVERS} observers"),
        ));
    }
    Ok(())
}

/// Register the provider's hooks and functions on `iii`.
pub fn register(iii: &Arc<IIIClient>) {
    let provider = Arc::new(Provider::default());

    // Hooks by id before registering, so they see every observer.
    let hook_iii = iii.clone();
    iii.on_observer_added_for(ON_COUNTER_CHANGED_FN, move |obs: Observer| {
        let result = accept(&hook_iii, &obs);
        async move { result }
    });
    // Each bump reaches the observers whose args name its counter.
    iii.on_observer_match_for(
        ON_COUNTER_CHANGED_FN,
        Match::keys(
            |obs| Keys::from(obs.args_value()["counter"].as_str()),
            |call| Keys::from(call.input["counter"].as_str()),
        ),
    );
    // The handler returns the event: request and response formats are
    // derived from `CounterChanged`.
    iii.register_function(
        ON_COUNTER_CHANGED_FN,
        RegisterFunction::new(|event: CounterChanged| Ok::<_, Error>(event))
            .description(
                "Demo counter changed: { counter, revision } after each committed bump. \
                 Observe it with args { counter }.",
            )
            .args_format::<CounterArgs>()
            .private(),
    );

    let get = provider.clone();
    iii.register_function(
        "demo::counter::get",
        RegisterFunction::new_async(move |request: CounterRequest| {
            let provider = get.clone();
            async move { Ok::<Value, Error>(provider.read(&request.counter)) }
        })
        .description("Read one demo counter: { counter, value, revision }.")
        .metadata(json!({ "read_only": true })),
    );

    let bump = provider.clone();
    let bump_iii = iii.clone();
    iii.register_function(
        "demo::counter::bump",
        RegisterFunction::new_async(move |request: CounterRequest| {
            let provider = bump.clone();
            let iii = bump_iii.clone();
            async move {
                let revision = {
                    let mut counters = provider.counters.lock().unwrap();
                    let entry = counters.entry(request.counter.clone()).or_default();
                    entry.0 += 1;
                    entry.1 += 1;
                    entry.1
                };
                let event = json!({ "counter": request.counter, "revision": revision });
                let call = Call {
                    input: event.clone(),
                    ..Call::default()
                };
                // After commit; a counter nobody observes costs one lookup.
                if iii.is_observed_for(ON_COUNTER_CHANGED_FN, &call) {
                    iii.call(CallRequest {
                        action: Some(CallAction::Void),
                        ..CallRequest::new(ON_COUNTER_CHANGED_FN, event)
                    })
                    .await?;
                }
                let mut current = provider.read(&request.counter);
                current["observers"] = json!(observers_of(&iii, &request.counter));
                Ok::<Value, Error>(current)
            }
        })
        .description(
            "Increment a demo counter, then call demo::on-counter-changed for its observers.",
        ),
    );

    let count_iii = iii.clone();
    iii.register_function(
        "demo::observers::count",
        RegisterFunction::new_async(move |_: Empty| {
            let iii = count_iii.clone();
            async move {
                Ok::<Value, Error>(json!({ "count": iii.observers(ON_COUNTER_CHANGED_FN).len() }))
            }
        })
        .description("Number of active demo::on-counter-changed observers.")
        .metadata(json!({ "read_only": true })),
    );
}
