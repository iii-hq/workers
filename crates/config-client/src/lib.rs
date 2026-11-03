//! Client plumbing for the builtin `configuration` worker, shared by the
//! workers whose entries carry live console knobs (fp, web, workflow,
//! sandbox-code-runner) so the retry/NOT_FOUND/seeding/reload rules exist
//! once instead of drifting per worker (docs/sops/configuration.md).
//!
//! Split of responsibilities: the worker keeps its config type, schema,
//! parse, and what *applying* a config means; this crate owns how to talk to
//! the configuration worker — and the two rules that are easy to get subtly
//! wrong:
//!
//! - **Seeding**: `ensure` forwards the candidate (`--config` seed or
//!   built-in default) as `initial_value`; the engine's
//!   `configuration::ensure` installs it ONLY against an absent/null entry
//!   and preserves any stored operator/Compose value (even `false`/`0`/`""`)
//!   atomically. Engines without ensure use a warned, non-atomic raw get/register
//!   compatibility path; stored non-null values omit initial_value entirely.
//! - **Reload serialization**: every reload runs under one lock with the
//!   fetch INSIDE it, so overlapping `configuration::on-updated` deliveries
//!   converge on the latest authoritative value instead of racing
//!   (docs/sops/configuration.md §6).
//! - **Control-plane routing**: `configuration::*` is a worker-owned API,
//!   not an `engine::*` builtin. Its client calls therefore name `default`
//!   explicitly while registered callbacks still target the caller's
//!   project namespace.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::{CallRequest, Deliver, IIIClient, ObserverHandle, RegisterFunction, Source};
use serde_json::{json, Value};

/// The configuration worker's change event, observed by [`on_change`].
const ON_UPDATED_FN: &str = "configuration::on-updated";
const TIMEOUT_MS: u64 = 5_000;
const RETRIES: u32 = 3;
const RETRY_BACKOFF_MS: u64 = 250;

/// A worker's configuration entry: the identity + schema half of
/// `configuration::register`.
pub struct EntrySpec {
    pub id: &'static str,
    /// Stable injectable configuration-form family. Unlike `id`, this never
    /// changes when a supervisor assigns `III_CONFIG_NAME`.
    pub form_id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
    /// Installed as `initial_value` when nothing is stored yet and no
    /// explicit seed is given.
    pub default_value: Value,
}

/// Shared initialization for workers retaining independent SDK pins and RPC retries.
pub mod initialization;

#[cfg(test)]
mod initialization_tests;

/// Declare metadata and seed atomically when supported. On a proven missing ensure
/// capability, use legacy initialization with an explicit concurrency warning.
pub async fn ensure(iii: &IIIClient, spec: &EntrySpec, seed: Option<Value>) -> Result<(), String> {
    let mut payload = registration_payload(spec);
    payload["initial_value"] = seed.unwrap_or_else(|| spec.default_value.clone());
    initialization::ensure_with(payload, |function, payload| {
        call_configuration_with_retry(iii, function, payload)
    })
    .await
}

fn registration_payload(spec: &EntrySpec) -> Value {
    json!({
        "id": spec.id,
        "name": spec.name,
        "description": spec.description,
        "schema": spec.schema,
        "metadata": { "ui_form": spec.form_id },
    })
}

/// The stored value, or `None` when nothing is stored yet (a `NOT_FOUND`
/// entry, or a stored explicit `null` — the placeholder an unseeded
/// registration persists — including a null the engine refuses to serve
/// through its schema check, see [`initialization::get_value_with`]).
///
/// The missing-entry code is the configuration worker's uppercase literal
/// `NOT_FOUND` and the match is deliberately case-SENSITIVE: the engine's
/// missing-FUNCTION code is lowercase `function_not_found`, and a
/// configuration worker that is absent or unroutable must surface as an
/// error, never read as "nothing stored yet".
pub async fn fetch(iii: &IIIClient, id: &str) -> Result<Option<Value>, String> {
    let stored = initialization::get_value_with(id, |function, payload| {
        call_configuration_with_retry(iii, function, payload)
    })
    .await?;
    Ok(stored.filter(|v| !v.is_null()))
}

/// `true` only when the error carries the configuration worker's standalone
/// `NOT_FOUND` entry code, identified by the outermost `remote error (<code>)` envelope code rather than a substring or token scan of the message, so
/// a compound code such as `RESOURCE_NOT_FOUND`/`STATEMENT_NOT_FOUND` or the
/// engine's lowercase missing-FUNCTION code `function_not_found` still
/// propagates as a failure instead of being read as "nothing stored yet".
fn is_not_found(error: &str) -> bool {
    // Anchor on the SDK's own rendering instead of scanning the whole string:
    // a remote failure prints as `remote error ({code}): {message}`, and this
    // client wraps a retried get as
    // `configuration::get failed after RETRIES attempts: {err}`. Peel exactly
    // that one wrapper (never a foreign one or a different attempt count) and
    // then require the NOT_FOUND envelope at the very start, so a NOT_FOUND
    // code buried in an unrelated message, a nested envelope, or a different
    // wrapper stays a real failure and propagates.
    const RETRY_WRAPPER: &str = "configuration::get failed after 3 attempts: ";
    const _: () = assert!(RETRIES == 3);
    let raw = error.trim();
    let raw = raw.strip_prefix(RETRY_WRAPPER).unwrap_or(raw);
    raw == "NOT_FOUND"
        || raw == "remote error (NOT_FOUND):"
        || raw.starts_with("remote error (NOT_FOUND): ")
}

/// Retry transient RPC failures, returning a definitive missing-entry response immediately.
async fn call_configuration_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let mut last_err = String::new();
    for attempt in 1..=RETRIES {
        match iii
            .call(
                CallRequest {
                    function_id: function_id.to_string(),
                    payload: payload.clone(),
                    action: None,
                    timeout_ms: Some(TIMEOUT_MS),
                }
                .namespace("default"),
            )
            .await
        {
            Ok(v) => return Ok(v),
            Err(e) => {
                last_err = e.to_string();
                if matches!(&e, Error::Remote { code, .. } if code == "function_not_found" || code == "NOT_FOUND")
                {
                    return Err(last_err);
                }
                // NOT_FOUND is a definitive answer (nothing stored yet, the
                // normal first-ever boot), not a transient failure — hand it
                // straight to the caller instead of retrying and warning.
                if is_not_found(&last_err) {
                    return Err(last_err);
                }
                if attempt < RETRIES {
                    tracing::warn!(
                        function_id,
                        attempt,
                        error = %last_err,
                        "configuration RPC failed; retrying"
                    );
                    tokio::time::sleep(Duration::from_millis(
                        RETRY_BACKOFF_MS * u64::from(attempt),
                    ))
                    .await;
                }
            }
        }
    }
    Err(format!(
        "{function_id} failed after {RETRIES} attempts: {last_err}"
    ))
}

/// A live observer that follows a boolean knob (the guidance hooks).
/// The mutex serialises concurrent reconciles; it is sync and never held
/// across an await.
#[derive(Clone, Default)]
pub struct ObserverSlot(Arc<Mutex<Option<ObserverHandle>>>);

impl ObserverSlot {
    /// Reconcile the live observer with `enabled`: on → `observe()` once;
    /// off → dispose the observer (on a spawned task) and drop the handle.
    /// Idempotent under repeated config events. `observe` returns at once:
    /// the observer stays `pending` until its function registers.
    pub fn reconcile(
        &self,
        enabled: bool,
        observe: impl FnOnce() -> ObserverHandle,
        on_msg: &str,
        off_msg: &str,
    ) {
        let mut slot = self.0.lock().unwrap_or_else(|p| p.into_inner());
        match (enabled, slot.is_some()) {
            (true, false) => {
                let handle = observe();
                tracing::info!(observer_id = handle.id(), "{}", on_msg);
                *slot = Some(handle);
            }
            (false, true) => {
                if let Some(handle) = slot.take() {
                    dispose_in_background(handle);
                }
                tracing::info!("{}", off_msg);
            }
            _ => {}
        }
    }

    /// Whether an observer is currently held (test/introspection helper).
    pub fn is_observing(&self) -> bool {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).is_some()
    }
}

/// `dispose` waits for the engine's confirmation, so it runs off the
/// reconcile path, which holds the slot's sync lock.
fn dispose_in_background(handle: ObserverHandle) {
    match tokio::runtime::Handle::try_current() {
        Ok(runtime) => {
            runtime.spawn(async move {
                if let Err(error) = handle.dispose().await {
                    tracing::warn!(observer_id = handle.id(), %error, "observer dispose failed");
                }
            });
        }
        Err(_) => tracing::warn!(
            observer_id = handle.id(),
            "no tokio runtime to dispose the observer on; it stays active"
        ),
    }
}

/// The `configuration::on-updated` event, delivered as output to
/// `<worker>::on-config-change`. Advisory only: handlers read `id` and
/// re-fetch the authoritative value, so a direct call can never inject
/// config.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Configuration id that changed.
    #[serde(default)]
    pub id: Option<String>,
}

/// Ack returned by the internal `<worker>::on-config-change` handler.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

type BoxFut = Pin<Box<dyn Future<Output = ()> + Send>>;

/// A serialized reload: `run` executes the worker's fetch→parse→apply under
/// one shared lock, with the fetch INSIDE it — whichever reload applies
/// later also fetched later, so a slow, older `configuration::get` response
/// can never overwrite a newer state.
#[derive(Clone)]
pub struct Reload {
    f: Arc<dyn Fn() -> BoxFut + Send + Sync>,
    lock: Arc<tokio::sync::Mutex<()>>,
}

impl Reload {
    pub async fn run(&self) {
        let _serialized = self.lock.lock().await;
        (self.f)().await;
    }
}

/// Register the internal `fn_id` reload handler (typed, `internal`-tagged —
/// keep it out of the callable catalog agents browse, and denied to agents
/// in iii-permissions.yaml) and observe `configuration::on-updated` for
/// `config_id` with it: filters on `input.id` and on
/// `input.event_type == "configuration:updated"`, the event delivered as
/// output. Every delivery runs `reload` through the same serialized
/// [`Reload`]. The observer stays `pending` while the configuration source
/// is not registered.
///
/// Returns that [`Reload`]: call `.run()` once right after this registration
/// to close the boot gap — an update landing between the boot-time fetch and
/// the observer becoming active reached no one, and without the extra pass
/// it would stay invisible until the NEXT update or a restart.
pub fn on_change<F, Fut>(
    iii: &Arc<IIIClient>,
    config_id: &'static str,
    fn_id: &'static str,
    description: &'static str,
    reload: F,
) -> Reload
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let reload = Reload {
        f: Arc::new(move || Box::pin(reload()) as BoxFut),
        lock: Arc::new(tokio::sync::Mutex::new(())),
    };

    let for_handler = reload.clone();
    iii.register_function(
        fn_id,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let reload = for_handler.clone();
            async move {
                reload.run().await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(description)
        .metadata(json!({ "internal": true })),
    );

    // The observer lives as long as the worker's connection; dropping the
    // handle does not dispose it.
    iii.observe(
        Source::function(ON_UPDATED_FN)
            .filters(update_filters(config_id))
            .deliver(Deliver::Output),
        fn_id,
    );
    reload
}

/// Selects the `configuration:updated` events of one entry.
fn update_filters(config_id: &str) -> Value {
    json!({
        "input.id": { "$eq": config_id },
        "input.event_type": { "$eq": "configuration:updated" },
    })
}

/// Follow another worker's configuration entry. Its id comes from that
/// worker's `<worker>::configuration-id` (retried every few seconds until the
/// worker answers, so start order does not matter); its value (`None` while
/// the entry does not exist) is published on the returned channel and
/// refreshed on every `configuration::on-updated` update of the entry through
/// `fn_id`, this worker's internal handler. `Err` when the worker answers
/// without an id.
pub async fn follow(
    iii: &Arc<IIIClient>,
    worker: &str,
    fn_id: &'static str,
    description: &'static str,
) -> Result<tokio::sync::watch::Receiver<Option<Value>>, String> {
    let identity = format!("{worker}::configuration-id");
    let id = loop {
        let reply = iii
            .call(CallRequest {
                function_id: identity.clone(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(5_000),
            })
            .await;
        match reply {
            Ok(reply) => match reply.get("id").and_then(Value::as_str) {
                Some(id) if !id.trim().is_empty() => break id.to_owned(),
                _ => return Err(format!("{identity} answered without an id")),
            },
            Err(_) => tokio::time::sleep(Duration::from_secs(5)).await,
        }
    };
    // One followed entry per process: its id lives as long as the observer.
    let id: &'static str = Box::leak(id.into_boxed_str());
    let (tx, rx) = tokio::sync::watch::channel(fetch(iii, id).await.ok().flatten());
    let tx = Arc::new(tx);
    let engine = iii.clone();
    let reload = on_change(iii, id, fn_id, description, move || {
        let (engine, tx) = (engine.clone(), tx.clone());
        async move {
            if let Ok(value) = fetch(&engine, id).await {
                tx.send_replace(value);
            }
        }
    });
    reload.run().await;
    Ok(rx)
}

#[cfg(test)]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/support/is_not_found_cases.rs"
    ));

    use super::*;
    use iii_sdk::ObserverState;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The shared client's missing-entry classifier: a real `Error::Remote`
    /// NOT_FOUND envelope (and its retry-wrapped form) is the only signal that
    /// seeds a default; anything else propagates so a service failure never
    /// clobbers a stored value.
    #[test]
    fn is_not_found_matches_only_the_envelope_code() {
        assert_missing_entry_contract(is_not_found);
    }

    /// `IIIClient::new` only builds local state — no network — and `observe`
    /// files the observer locally until a connection sends it, so a real
    /// `ObserverHandle` is available engine-free.
    fn client() -> Arc<IIIClient> {
        Arc::new(IIIClient::new("ws://127.0.0.1:1"))
    }

    #[test]
    fn registration_keeps_runtime_id_separate_from_form_family() {
        let spec = EntrySpec {
            id: "web-team-a",
            form_id: "web",
            name: "Web",
            description: "Web settings",
            schema: json!({ "type": "object" }),
            default_value: json!({}),
        };

        let payload = registration_payload(&spec);
        assert_eq!(payload["id"], "web-team-a");
        assert_eq!(payload["metadata"]["ui_form"], "web");
    }

    fn some_observer(iii: &IIIClient) -> ObserverHandle {
        iii.observe(
            Source::function("harness::hook::on-pre-generate")
                .args(json!({ "on_error": "fail_open" })),
            "test::hook",
        )
    }

    #[tokio::test]
    async fn observer_slot_reconciles_on_off_and_is_idempotent() {
        let iii = client();
        let slot = ObserverSlot::default();
        let observes = AtomicUsize::new(0);
        let observed = Mutex::new(None::<ObserverHandle>);

        let observe = || {
            observes.fetch_add(1, Ordering::SeqCst);
            let handle = some_observer(&iii);
            *observed.lock().unwrap() = Some(handle.clone());
            handle
        };
        slot.reconcile(true, observe, "on", "off");
        assert!(slot.is_observing());
        // Repeated `on` events must not observe again.
        slot.reconcile(true, observe, "on", "off");
        assert_eq!(observes.load(Ordering::SeqCst), 1);

        slot.reconcile(false, observe, "on", "off");
        assert!(!slot.is_observing());
        // Repeated `off` events are a no-op too.
        slot.reconcile(false, observe, "on", "off");
        assert_eq!(observes.load(Ordering::SeqCst), 1);

        // `off` disposes the dropped observer on a spawned task.
        let handle = observed.lock().unwrap().take().expect("observed once");
        tokio::time::timeout(Duration::from_secs(5), async {
            while handle.state() != ObserverState::Disposed {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the observer is disposed");

        // The next `on` observes afresh.
        slot.reconcile(true, observe, "on", "off");
        assert!(slot.is_observing());
        assert_eq!(observes.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn update_filters_select_one_entrys_updates() {
        assert_eq!(
            update_filters("fp"),
            json!({
                "input.id": { "$eq": "fp" },
                "input.event_type": { "$eq": "configuration:updated" },
            })
        );
    }

    #[tokio::test]
    async fn reload_serializes_and_runs_every_call() {
        let ran = Arc::new(AtomicUsize::new(0));
        let counted = ran.clone();
        let iii = client();
        let reload = on_change(
            &iii,
            "test",
            "test::on-config-change",
            "test reload",
            move || {
                let ran = counted.clone();
                async move {
                    ran.fetch_add(1, Ordering::SeqCst);
                }
            },
        );

        reload.run().await;
        reload.run().await;
        assert_eq!(ran.load(Ordering::SeqCst), 2);
    }

    /// The event payload is advisory and lenient: `{}`, a full `{id}`, and
    /// junk fields must all deserialize (the handler re-fetches anyway).
    #[test]
    fn on_config_change_event_is_lenient() {
        let empty: OnConfigChangeEvent = serde_json::from_value(json!({})).unwrap();
        assert!(empty.id.is_none());
        let full: OnConfigChangeEvent =
            serde_json::from_value(json!({ "id": "fp", "extra": 1 })).unwrap();
        assert_eq!(full.id.as_deref(), Some("fp"));
    }
}
