//! The observable `shell::on-*` functions (iii 0.25), one next to each
//! trigger type this worker owns: `shell::on-job-finished`,
//! `shell::on-git-changed`, `shell::on-turns-changed` and
//! `shell::on-changed`.
//!
//! Old consumers bind the trigger type with the two-step pattern; new ones
//! observe the function with the same config as args. Each function is
//! owner-delivered: its [`ObserveHandler`] files an observer in the very
//! registry the trigger type's bindings live in, so both are served by the
//! same watch and the same fan-out, and an observer receives exactly the
//! event a binding does, through [`Observer::call`]. Events come only from
//! this worker, in-process: invoking an on-* function is refused with
//! [`codes::FORBIDDEN`], so nobody can forge one.

use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, Observe, ObserveHandler, Observer, RegisterFunction};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Codes the `shell::on-*` functions refuse with.
pub mod codes {
    /// Invoking an on-* function directly.
    pub const FORBIDDEN: &str = "FORBIDDEN";
    /// An observer's args the trigger type would refuse as a config.
    pub const INVALID_ARGS: &str = "INVALID_ARGS";
}

/// One observer of a `shell::on-*` function, as the fan-out sees it.
/// Production wraps the SDK's [`Observer`]; tests record.
pub trait EventObserver: Send + Sync {
    /// The observer's id, stable across replays.
    fn id(&self) -> &str;
    /// The function its events go to, for logs and probes.
    fn target(&self) -> &str;
    /// Hand one event to the observer. Fire-and-forget like a binding's
    /// `Void` delivery: never blocks the producer of the event.
    fn deliver(&self, payload: Value);
}

impl std::fmt::Debug for dyn EventObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventObserver")
            .field("id", &self.id())
            .field("target", &self.target())
            .finish()
    }
}

/// The registry key of an observer, apart from every binding id: the
/// engine's trigger-instance ids and the observers' ids are separate spaces.
pub fn observer_key(id: &str) -> String {
    format!("observer:{id}")
}

/// How long a delivery waits for an observer the engine has not marked
/// active yet: `on_observer_added` files it before the engine acknowledges
/// it, and a replay (`shell::on-job-finished` on a finished job) or a watch
/// event may come first.
const ACTIVATION_WAIT: Duration = Duration::from_secs(5);
const ACTIVATION_POLL: Duration = Duration::from_millis(50);

impl EventObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    fn target(&self) -> &str {
        Observer::target(self)
    }

    fn deliver(&self, payload: Value) {
        let obs = self.clone();
        // Own task per delivery, so one hung observer cannot hold up the
        // next one or the pump that produced the event.
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + ACTIVATION_WAIT;
            while !obs.is_active() && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(ACTIVATION_POLL).await;
            }
            match obs.call::<Value>(payload).await {
                Ok(_) => {}
                // The observer's own filters declined this event.
                Err(Error::Remote { code, .. }) if code == "FILTERS_NOT_MATCHED" => {}
                Err(e) => tracing::warn!(
                    observer = %obs.id(),
                    function_id = %obs.function_id(),
                    target = %obs.target(),
                    error = %e,
                    "observer delivery failed"
                ),
            }
        });
    }
}

/// The error every `shell::on-*` handler returns: events come only from
/// this worker, never by invoking the function.
pub fn refuse_invocation(function_id: &str) -> Error {
    Error::Remote {
        code: codes::FORBIDDEN.to_string(),
        message: format!(
            "{function_id} is an event source: observe it \
             (iii.observe({{ function_id: '{function_id}', args }}, target)) instead of invoking \
             it; only the ide worker emits its events"
        ),
        stacktrace: None,
    }
}

/// The handler of an on-* function: takes the event `E` its observers
/// receive, so the function publishes that typed request schema (and `()`
/// as its response), never `AnyValue`, and refuses every invocation.
pub fn refusing_handler<E>(
    function_id: &'static str,
) -> impl Fn(E) -> Result<(), Error> + Send + Sync + 'static
where
    E: DeserializeOwned + JsonSchema + Send + 'static,
{
    move |_event: E| Err(refuse_invocation(function_id))
}

/// Register `function_id` owner-delivered: args `A` (the trigger config),
/// request schema `E` (the event an observer receives), `handler` filing
/// observers in the trigger type's registry. Invoking it is refused. On an
/// engine without observe it registers as a plain function and its hooks
/// never run.
pub fn register_observable<A, E, H>(
    iii: &IIIClient,
    function_id: &'static str,
    description: &str,
    handler: H,
) where
    A: JsonSchema,
    E: DeserializeOwned + JsonSchema + Send + 'static,
    H: ObserveHandler,
{
    iii.register_function(
        function_id,
        RegisterFunction::new(refusing_handler::<E>(function_id))
            .description(description)
            .observe(Observe::owner().args_format::<A>().handler(handler)),
    );
    tracing::info!(function_id, "registered observable function");
}

/// A recording [`EventObserver`] for the unit tests of each `shell::on-*`
/// registration.
#[cfg(test)]
pub(crate) mod testing {
    use std::sync::{Arc, Mutex};

    use serde_json::Value;

    use super::EventObserver;

    pub(crate) struct Recorder {
        id: String,
        target: String,
        seen: Mutex<Vec<Value>>,
    }

    impl Recorder {
        pub(crate) fn new(id: &str) -> Arc<Self> {
            Arc::new(Self {
                id: id.to_string(),
                target: format!("observer-target::{id}"),
                seen: Mutex::new(Vec::new()),
            })
        }

        pub(crate) fn seen(&self) -> Vec<Value> {
            self.seen.lock().unwrap().clone()
        }
    }

    /// Assert the schemas iii-sdk publishes for `handler` (an on-* refusing
    /// handler) are typed: the interface boot smoke fails `AnyValue` or an
    /// empty schema.
    pub(crate) fn assert_typed_schemas<F, M>(function_id: &str, _handler: &F)
    where
        F: iii_sdk::iii::IntoSyncHandler<M>,
    {
        const TYPED: [&str; 7] = [
            "type",
            "properties",
            "$ref",
            "allOf",
            "anyOf",
            "oneOf",
            "enum",
        ];
        for (side, schema) in [
            ("request", F::request_format()),
            ("response", F::response_format()),
        ] {
            let schema = schema.unwrap_or_else(|| panic!("{function_id}: no {side} schema"));
            assert!(
                TYPED.iter().any(|key| schema.get(*key).is_some()),
                "{function_id}: untyped {side} schema {schema}"
            );
            assert_ne!(schema["title"], "AnyValue", "{function_id}: {side} schema");
        }
    }

    impl EventObserver for Recorder {
        fn id(&self) -> &str {
            &self.id
        }

        fn target(&self) -> &str {
            &self.target
        }

        fn deliver(&self, payload: Value) {
            self.seen.lock().unwrap().push(payload);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_on_function_refuses_direct_invocation() {
        // shell::on-turns-changed lives in the binary only: turn_events.rs
        // checks it.
        let pairs = [
            (
                crate::job_events::JOB_FINISHED,
                crate::job_events::ON_JOB_FINISHED,
            ),
            (
                crate::git_events::GIT_CHANGED,
                crate::git_events::ON_GIT_CHANGED,
            ),
            (crate::events::CHANGED, crate::events::ON_CHANGED),
        ];
        assert_eq!(
            pairs.map(|(_, function_id)| function_id),
            [
                "shell::on-job-finished",
                "shell::on-git-changed",
                "shell::on-changed",
            ]
        );
        for (trigger_type, function_id) in pairs {
            assert!(trigger_type.starts_with("shell::"), "{trigger_type}");
            let err = refuse_invocation(function_id);
            assert!(
                matches!(&err, Error::Remote { code, message, .. }
                    if code == codes::FORBIDDEN && message.contains(function_id)),
                "{err:?}"
            );
        }
    }

    #[test]
    fn every_on_function_publishes_typed_schemas() {
        use crate::events::{ChangedEvent, ON_CHANGED};
        use crate::git_events::{GitChangedEvent, ON_GIT_CHANGED};
        use crate::job_events::{JobFinishedEvent, ON_JOB_FINISHED};
        use testing::assert_typed_schemas;
        // shell::on-turns-changed: checked in turn_events.rs.
        assert_typed_schemas(
            ON_JOB_FINISHED,
            &refusing_handler::<JobFinishedEvent>(ON_JOB_FINISHED),
        );
        assert_typed_schemas(
            ON_GIT_CHANGED,
            &refusing_handler::<GitChangedEvent>(ON_GIT_CHANGED),
        );
        assert_typed_schemas(ON_CHANGED, &refusing_handler::<ChangedEvent>(ON_CHANGED));
        // The typed handler still refuses a well-formed (forged) event.
        let forged = ChangedEvent {
            path: "a.txt".into(),
            kind: "modified".into(),
            root: "/tmp".into(),
            dir: false,
            ignored: false,
        };
        let err = refusing_handler::<ChangedEvent>(ON_CHANGED)(forged).unwrap_err();
        assert!(
            matches!(&err, Error::Remote { code, .. } if code == codes::FORBIDDEN),
            "{err:?}"
        );
    }

    #[test]
    fn observer_keys_never_collide_with_binding_ids() {
        assert_eq!(observer_key("o1"), "observer:o1");
        assert_ne!(observer_key("t1"), "t1");
    }
}
