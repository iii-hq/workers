//! In-process registration of the production `session-manager` surface
//! against the shared SDK handle.
//!
//! Re-uses the production entry points in the binary's fs-mode boot
//! order — six public trigger types, the internal store-events feed,
//! the `session::store::*` protocol, and the 14 handlers — backed by a
//! real `FsStore` over a per-binary tempdir, so @engine scenarios
//! exercise identical code paths end to end and can read the JSONL
//! files back for persistence assertions.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use tokio::sync::{OnceCell, RwLock};

use session_manager::config::{FsBackendConfig, StorageAdapter, WorkerConfig};
use session_manager::configuration::{AppState, ConfigCell};
use session_manager::events::{
    register_store_events_type, register_trigger_types, BridgeSubscribers,
};
use session_manager::functions::{self, store_protocol};
use session_manager::runtime::{build_runtime, SessionBuildContext};

pub struct Shared {
    /// The main (fs-mode) instance's data directory; steps read the
    /// per-session JSONL files from here.
    pub data_dir: PathBuf,
    /// The main emitter's live relay registry (`session::store::events`
    /// subscribers); bridge stacks poll it to confirm their relay
    /// binding round-tripped (see `common::bridge`).
    pub bridges: BridgeSubscribers,
}

static SHARED: OnceCell<Arc<Shared>> = OnceCell::const_new();

/// Idempotent: the first caller registers; subsequent callers reuse.
pub async fn register_all(iii: &Arc<IIIClient>) -> Arc<Shared> {
    SHARED
        .get_or_init(|| async {
            // Leaked tempdir that lives for the test binary lifetime.
            let tmp = tempfile::tempdir().expect("create main data_dir tempdir");
            let data_dir = tmp.keep();

            let cfg = WorkerConfig {
                adapter: StorageAdapter::Fs(FsBackendConfig {
                    data_dir: data_dir.display().to_string(),
                }),
                ..WorkerConfig::default()
            };

            // Same boot order as src/main.rs in fs mode.
            let sets = register_trigger_types(iii);
            let bridges = register_store_events_type(iii);
            let config_cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg.clone())));
            let ctx = Arc::new(SessionBuildContext {
                iii: iii.clone(),
                local_url: super::engine::ws_url(),
                sets,
                bridge_subscribers: bridges.clone(),
                config_cell,
            });
            let runtime = build_runtime(&cfg, &ctx).expect("build main fs runtime");
            let state = AppState::new(runtime, ctx);

            store_protocol::register_store_protocol(iii, state.clone());
            functions::register_all(iii, &state);

            // Block until the engine can route to the surface registered
            // above: registrations flow over one connection in boot
            // order, so the *last* one being routable means the batch
            // landed. No fixed sleep — slow runners just poll longer.
            wait_until_routable(iii, "session::set-active-leaf").await;

            Arc::new(Shared { data_dir, bridges })
        })
        .await
        .clone()
}

pub fn shared() -> Option<Arc<Shared>> {
    SHARED.get().cloned()
}

/// Poll the engine until `function_id` is routable, panicking after a
/// deadline. An `Err` carrying a `session/` code also counts as
/// routable — it proves the call reached the production handler.
async fn wait_until_routable(iii: &Arc<IIIClient>, function_id: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let res = iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload: serde_json::json!({
                    "session_id": "bdd-readiness-probe",
                    "entry_id": "none",
                }),
                action: None,
                timeout_ms: Some(1_000),
            })
            .await;
        match res {
            Ok(_) => return,
            Err(e) if e.to_string().contains("session/") => return,
            Err(e) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "{function_id} did not become routable within 10s: {e}"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}
