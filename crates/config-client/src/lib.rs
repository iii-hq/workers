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
//! - **Seeding**: `configuration::register` REPLACES the stored value
//!   whenever `initial_value` is supplied (engine `store.rs` — "Existing
//!   entries keep their value unless `initial_value` is supplied"), so a
//!   seed or built-in default is installed only when nothing is stored yet.
//! - **Reload serialization**: every reload runs under one lock with the
//!   fetch INSIDE it, so overlapping `configuration:updated` deliveries
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
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::trigger::Trigger;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

const TIMEOUT_MS: u64 = 5_000;
const RETRIES: u32 = 3;
const RETRY_BACKOFF_MS: u64 = 250;

/// A worker's configuration entry: the identity + schema half of
/// `configuration::register`.
pub struct EntrySpec {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
    /// Installed as `initial_value` when nothing is stored yet and no
    /// explicit seed is given.
    pub default_value: Value,
}

/// Register the entry's schema (idempotent, safe to call every boot). `seed`
/// (a `--config` value) or the built-in default becomes `initial_value` ONLY
/// when nothing is stored yet — see the module doc for why the pre-check is
/// load-bearing, not an optimization.
pub async fn register(
    iii: &IIIClient,
    spec: &EntrySpec,
    seed: Option<Value>,
) -> Result<(), String> {
    let mut payload = json!({
        "id": spec.id,
        "name": spec.name,
        "description": spec.description,
        "schema": spec.schema,
    });
    if fetch(iii, spec.id).await?.is_none() {
        payload["initial_value"] = seed.unwrap_or_else(|| spec.default_value.clone());
    }
    trigger_configuration_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

/// The stored value, or `None` when nothing is stored yet (a `NOT_FOUND`
/// entry, or a stored explicit `null` — the placeholder an unseeded
/// registration persists).
///
/// The missing-entry code is the configuration worker's uppercase literal
/// `NOT_FOUND` and the match is deliberately case-SENSITIVE: the engine's
/// missing-FUNCTION code is lowercase `function_not_found`, and a
/// configuration worker that is absent or unroutable must surface as an
/// error, never read as "nothing stored yet".
pub async fn fetch(iii: &IIIClient, id: &str) -> Result<Option<Value>, String> {
    match trigger_configuration_with_retry(iii, "configuration::get", json!({ "id": id })).await {
        Ok(resp) => Ok(resp.get("value").cloned().filter(|v| !v.is_null())),
        Err(e) if e.contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

async fn trigger_configuration_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let mut last_err = String::new();
    for attempt in 1..=RETRIES {
        match iii
            .trigger(
                TriggerRequest {
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
                // NOT_FOUND is a definitive answer (nothing stored yet, the
                // normal first-ever boot), not a transient failure — hand it
                // straight to the caller instead of retrying and warning.
                if last_err.contains("NOT_FOUND") {
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

/// Best-effort trigger binding: a transient failure must not brick boot or a
/// reload — it surfaces as a `None` handle (and a warn) and is retried on
/// the next config event.
pub fn try_bind(iii: &IIIClient, input: RegisterTriggerInput) -> Option<Trigger> {
    let (trigger_type, function_id) = (input.trigger_type.clone(), input.function_id.clone());
    match iii.register_trigger(input) {
        Ok(handle) => {
            tracing::info!(trigger_type, function_id, "trigger binding requested");
            Some(handle)
        }
        Err(e) => {
            tracing::warn!(trigger_type, function_id, error = %e, "trigger binding failed");
            None
        }
    }
}

/// A live trigger binding that follows a boolean knob (the guidance hooks).
/// The mutex serialises concurrent reconciles; it is sync and never held
/// across an await.
#[derive(Clone, Default)]
pub struct BindingSlot(Arc<Mutex<Option<Trigger>>>);

impl BindingSlot {
    /// Reconcile the live binding with `enabled`: on → `bind()` once; off →
    /// unregister and drop the handle. Idempotent under repeated config
    /// events, and a failed bind (`None`) retries on the next event.
    pub fn reconcile(
        &self,
        enabled: bool,
        bind: impl FnOnce() -> Option<Trigger>,
        on_msg: &str,
        off_msg: &str,
    ) {
        let mut slot = self.0.lock().unwrap_or_else(|p| p.into_inner());
        match (enabled, slot.is_some()) {
            (true, false) => {
                *slot = bind();
                if slot.is_some() {
                    tracing::info!("{}", on_msg);
                }
            }
            (false, true) => {
                if let Some(handle) = slot.take() {
                    handle.unregister();
                }
                tracing::info!("{}", off_msg);
            }
            _ => {}
        }
    }

    /// Whether a binding is currently held (test/introspection helper).
    pub fn is_bound(&self) -> bool {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).is_some()
    }
}

/// Trigger payload for `<worker>::on-config-change`. Advisory only: handlers
/// re-fetch the authoritative value and ignore it, so a direct call can
/// never inject config.
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
/// in iii-permissions.yaml) and bind it to `configuration:updated` for
/// `config_id`. Every delivery runs `reload` through the same serialized
/// [`Reload`].
///
/// Returns that [`Reload`]: call `.run()` once right after this registration
/// to close the boot gap — an update landing between the boot-time fetch and
/// this binding fired into nothing, and without the extra pass it would stay
/// invisible until the NEXT update or a restart.
pub fn on_change<F, Fut>(
    iii: &Arc<IIIClient>,
    config_id: &'static str,
    fn_id: &'static str,
    description: &'static str,
    reload: F,
) -> Result<Reload, Error>
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

    iii.register_trigger(RegisterTriggerInput::new(
        "configuration".to_string(),
        fn_id.to_string(),
        json!({ "configuration_id": config_id, "event_types": ["configuration:updated"] }),
    ))?;
    Ok(reload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// `IIIClient::new` only builds local state — no network — and
    /// `register_trigger` queues locally, so a real `Trigger` handle is
    /// available engine-free (the same trick the workers' own tests use).
    fn client() -> Arc<IIIClient> {
        Arc::new(IIIClient::new("ws://127.0.0.1:1"))
    }

    fn some_binding(iii: &IIIClient) -> Option<Trigger> {
        try_bind(
            iii,
            RegisterTriggerInput::new(
                "harness::hook::pre-generate".to_string(),
                "test::hook".to_string(),
                json!({ "on_error": "fail_open" }),
            ),
        )
    }

    #[test]
    fn binding_slot_reconciles_on_off_and_is_idempotent() {
        let iii = client();
        let slot = BindingSlot::default();
        let binds = AtomicUsize::new(0);

        let bind = || {
            binds.fetch_add(1, Ordering::SeqCst);
            some_binding(&iii)
        };
        slot.reconcile(true, bind, "on", "off");
        assert!(slot.is_bound());
        // Repeated `on` events must not re-bind.
        slot.reconcile(true, bind, "on", "off");
        assert_eq!(binds.load(Ordering::SeqCst), 1);

        slot.reconcile(false, bind, "on", "off");
        assert!(!slot.is_bound());
        // Repeated `off` events are a no-op too.
        slot.reconcile(false, bind, "on", "off");
        assert_eq!(binds.load(Ordering::SeqCst), 1);

        // A failed bind (None) leaves the slot empty so the next event retries.
        slot.reconcile(true, || None, "on", "off");
        assert!(!slot.is_bound());
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
        )
        .expect("registration succeeds engine-free");

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
