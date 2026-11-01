//! In-process registration of the production `session-manager` surface
//! against the shared SDK handle.
//!
//! Re-uses the production entry points in the binary's fs-mode boot
//! order — the six event functions, the internal store-events feed, the
//! `session::store::*` protocol, and the `session::*` handlers — backed
//! by a real `FsStore` over a per-binary tempdir, so @engine scenarios
//! exercise identical code paths end to end and can read the JSONL
//! files back for persistence assertions.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::{CallRequest, IIIClient};
use tokio::sync::{OnceCell, RwLock};

use session_manager::config::{FsBackendConfig, StorageAdapter, WorkerConfig};
use session_manager::configuration::{AppState, ConfigCell};
use session_manager::events::register_event_functions;
use session_manager::functions::{self, store_protocol};
use session_manager::runtime::{build_runtime, SessionBuildContext};

pub struct Shared {
    /// The main (fs-mode) instance's data directory; steps read the
    /// per-session JSONL files from here.
    pub data_dir: PathBuf,
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
            register_event_functions(iii);
            let config_cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg.clone())));
            let ctx = Arc::new(SessionBuildContext {
                iii: iii.clone(),
                local_url: super::engine::ws_url(),
                config_cell,
            });
            let runtime = build_runtime(&cfg, &ctx).expect("build main fs runtime");
            let state = AppState::new(runtime, ctx);

            store_protocol::register_store_protocol(iii, state.clone());
            functions::register_all(iii, &state);
            // The caller (`engine::get_or_init`) waits for the batch to
            // land, checks who owns the ids, and only then probes them.

            Arc::new(Shared { data_dir })
        })
        .await
        .clone()
}

pub fn shared() -> Option<Arc<Shared>> {
    SHARED.get().cloned()
}

/// The last function `register_all` registers: registrations flow over
/// one connection in boot order, so this one landing means the batch did.
pub const LAST_REGISTERED: &str = "session::set-active-leaf";

/// Poll the engine until `function_id` answers, or say why it did not
/// within 10 s. An `Err` carrying a `session/` code also counts — it
/// proves the call reached the production handler. Only call this once
/// the ids are known to be the harness's own: the probe is a real call.
pub async fn wait_until_routable(iii: &IIIClient, function_id: &str) -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let res = iii
            .call(CallRequest {
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
            Ok(_) => return Ok(()),
            Err(e) if e.to_string().contains("session/") => return Ok(()),
            Err(e) if std::time::Instant::now() >= deadline => {
                return Err(format!(
                    "{function_id} did not become routable within 10s: {e}"
                ));
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}
