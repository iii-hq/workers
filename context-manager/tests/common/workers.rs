//! In-process registration of the production `context-manager` surface
//! against the shared SDK handle.
//!
//! Re-uses the production entry points in the binary's boot order —
//! production adapters (`RouterModelResolver`, `RouterSummarizer`,
//! `FsLeaseStore`, `SystemClock`) and `functions::register_all` — so
//! @engine scenarios exercise identical code paths end to end,
//! including real filesystem lease writes (under a per-process temp
//! dir). `llm-router` is absent on the test engine, which is exactly
//! the degraded mode the spec defines (fallback limits; compact ->
//! overflow).

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use tokio::sync::{OnceCell, RwLock};

use context_manager::adapters::fs_lease::FsLeaseStore;
use context_manager::adapters::router::{RouterModelResolver, RouterSummarizer};
use context_manager::config::WorkerConfig;
use context_manager::functions;
use context_manager::ports::{lease_cell, Deps, SystemClock};

static REGISTERED: OnceCell<()> = OnceCell::const_new();

/// Idempotent: the first caller registers; subsequent callers reuse.
pub async fn register_all(iii: &Arc<IIIClient>) {
    REGISTERED
        .get_or_init(|| async {
            let cfg = WorkerConfig::default();
            let cell = Arc::new(RwLock::new(Arc::new(cfg)));
            let leases = lease_cell(Arc::new(
                FsLeaseStore::new(
                    std::env::temp_dir()
                        .join(format!("context-manager-bdd-leases-{}", std::process::id())),
                )
                .expect("create bdd lease dir"),
            ));
            let deps = Arc::new(Deps {
                resolver: Arc::new(RouterModelResolver::new(iii.clone())),
                // The summariser reads its timeout from the live snapshot per
                // call; router::chat is absent on the test engine, so only the
                // trigger error path runs anyway.
                summarizer: Arc::new(RouterSummarizer::new(iii.clone(), cell.clone())),
                leases,
                clock: Arc::new(SystemClock),
                config: cell,
            });
            functions::register_all(iii, &deps);

            // Registrations flow over one connection in order, so the
            // last one being routable means the batch landed.
            wait_until_routable(iii, "context::count-tokens").await;
        })
        .await;
}

/// Poll the engine until `function_id` is routable, panicking after a
/// deadline. An `Err` carrying a `context/` code also counts as
/// routable — it proves the call reached the production handler.
async fn wait_until_routable(iii: &Arc<IIIClient>, function_id: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let res = iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload: serde_json::json!({ "model": { "id": "bdd-readiness-probe" } }),
                action: None,
                timeout_ms: Some(1_000),
            })
            .await;
        match res {
            Ok(_) => return,
            Err(e) if e.to_string().contains("context/") => return,
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
