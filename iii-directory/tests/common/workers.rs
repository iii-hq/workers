//! In-process registration of the `skills` surface against the shared
//! SDK handle.
//!
//! Re-uses the production entry point `iii_skills::functions::register_all`
//! so the BDD scenarios exercise identical code paths. Registration is
//! idempotent (OnceCell) so a second BDD scenario reuses the same
//! handlers; we wipe the per-test `skills_folder` between scenarios so
//! each starts from a clean slate.
//!
//! A wiremock `MockServer` is also booted once-per-binary and the
//! worker is configured with its URL as `registry_url`. Each
//! scenario's download steps register their own routes against this
//! server so the engine path is exercised end-to-end without a real
//! HTTP call.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use iii_sdk::III;
use tokio::sync::OnceCell;
use wiremock::MockServer;

use iii_directory::{
    config::SkillsConfig,
    functions,
    trigger_types::{self, RegisteredTriggerTypes},
};

pub struct Shared {
    pub cfg: Arc<SkillsConfig>,
    /// Kept so step defs needing to assert on subscriber-set state can
    /// probe it directly (the registered trigger types live here).
    #[allow(dead_code)]
    pub triggers: Arc<RegisteredTriggerTypes>,
    /// The skills_folder the configured worker reads + writes against.
    pub skills_folder: PathBuf,
    /// MockServer the worker's `registry_url` points at. Scenarios add
    /// per-test mocks via `mock_server.register(Mock::given(...))`.
    pub mock_server: Arc<MockServer>,
}

static SHARED: OnceCell<Arc<Shared>> = OnceCell::const_new();

/// Idempotent: the first caller registers; subsequent callers reuse.
pub async fn register_all(iii: &Arc<III>) -> Result<Arc<Shared>> {
    if let Some(s) = SHARED.get() {
        return Ok(s.clone());
    }

    // Build a leaked tempdir that lives for the test binary lifetime.
    let tmp = tempfile::tempdir()?;
    let skills_folder = tmp.keep();
    std::fs::create_dir_all(&skills_folder)?;

    // Boot the mock registry server before registering handlers so the
    // captured cfg already points at the right URL.
    let mock_server = Arc::new(MockServer::start().await);

    let cfg = Arc::new(SkillsConfig {
        skills_folder: skills_folder.to_string_lossy().into_owned(),
        registry_url: mock_server.uri(),
        ..SkillsConfig::default()
    });
    let registered = trigger_types::register_all(iii);
    // `register_all` reads the live (hot-reloadable) handle; wrap a snapshot.
    // `Shared.cfg` keeps the plain `Arc<SkillsConfig>` the step defs assert on.
    let cfg_handle = (*cfg).clone().into_shared();
    functions::register_all(iii, &cfg_handle, &registered);

    // Give the SDK a beat to publish the function registrations before
    // scenarios start triggering them.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let shared = Arc::new(Shared {
        cfg,
        triggers: Arc::new(registered),
        skills_folder,
        mock_server,
    });
    let _ = SHARED.set(shared.clone());
    Ok(shared)
}

pub fn shared() -> Option<Arc<Shared>> {
    SHARED.get().cloned()
}

/// Wipe the per-test skills_folder between scenarios so leftover files
/// from one scenario don't pollute the next.
pub fn reset_fs(skills_folder: &Path) {
    let _ = std::fs::remove_dir_all(skills_folder);
    let _ = std::fs::create_dir_all(skills_folder);
}

/// Reset the wiremock server's registered routes between scenarios so
/// one scenario's mocks don't leak into the next.
pub async fn reset_mocks(server: &MockServer) {
    server.reset().await;
}
