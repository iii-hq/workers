//! A second fs-mode session-manager runtime for @engine adapter hot-reload
//! scenarios.
//!
//! It runs on the shared engine connection, so its emitter calls the event
//! functions the shared stack registered, over a scenario directory, and
//! its adapter changes through the production `apply_runtime`. The shared
//! main stack is never reloaded.

use std::path::Path;
use std::sync::Arc;

use iii_sdk::IIIClient;
use serde_json::{json, Value};
use tokio::sync::RwLock;

use session_manager::config::{FsBackendConfig, StorageAdapter, WorkerConfig};
use session_manager::configuration::{apply_runtime, AppState, ConfigCell};
use session_manager::runtime::{build_runtime, SessionBuildContext};
use session_manager::types::SessionMeta;

pub struct ReloadableInstance {
    pub state: AppState,
    /// Every data directory it has run over, removed when the scenario ends.
    dirs: Vec<tempfile::TempDir>,
}

fn fs_config(data_dir: &Path) -> WorkerConfig {
    WorkerConfig {
        adapter: StorageAdapter::Fs(FsBackendConfig {
            data_dir: data_dir.display().to_string(),
        }),
        ..WorkerConfig::default()
    }
}

impl ReloadableInstance {
    /// An fs-mode runtime over a fresh directory, on `engine`.
    pub fn start(engine: &Arc<IIIClient>) -> Self {
        let dir = tempfile::tempdir().expect("create reloadable data_dir");
        let cfg = fs_config(dir.path());
        let config_cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg.clone())));
        let ctx = Arc::new(SessionBuildContext {
            iii: engine.clone(),
            local_url: super::engine::ws_url(),
            config_cell,
        });
        let runtime = build_runtime(&cfg, &ctx).expect("build reloadable fs runtime");
        Self {
            state: AppState::new(runtime, ctx),
            dirs: vec![dir],
        }
    }

    /// Store a session with `metadata` directly in the current store, so no
    /// event function is called for it.
    pub async fn store_session(&self, session_id: &str, metadata: Value) {
        let meta: SessionMeta = serde_json::from_value(json!({
            "session_id": session_id,
            "title": "reload",
            "description": "",
            "status": "idle",
            "metadata": metadata,
            "created_at": 1,
            "updated_at": 1,
            "message_count": 0,
        }))
        .expect("session meta");
        let store = self.state.runtime.read().await.store.clone();
        store.put_meta(&meta).await.expect("store the session");
    }

    /// Change the adapter to a fresh, empty directory.
    pub async fn change_to_empty_dir(&mut self) {
        let dir = tempfile::tempdir().expect("create the next data_dir");
        apply_runtime(&self.state, fs_config(dir.path()))
            .await
            .expect("adapter reload");
        self.dirs.push(dir);
    }
}
