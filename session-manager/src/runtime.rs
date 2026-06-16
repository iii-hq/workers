//! The hot-swappable storage + event-fan-out runtime.
//!
//! Everything the worker derives from the `adapter` half of the config —
//! the [`SessionStore`], the [`SessionService`] over it, the event
//! [`EventSink`], and (bridge mode) the remote connection plus its relay —
//! lives in one [`SessionRuntime`]. [`build_runtime`] constructs it from a
//! [`WorkerConfig`]; [`configuration`](crate::configuration) keeps the live
//! one behind an `Arc<RwLock<_>>` and swaps a freshly-built one in when the
//! authoritative config changes, so an adapter change (fs <-> bridge, a new
//! `data_dir`, a bridge url/timeout) hot-reloads without a worker restart.
//!
//! Handlers read the live runtime per call (see
//! [`functions::register_all`](crate::functions::register_all)): an in-flight
//! request finishes on the runtime it started with; the next call observes
//! the swap.

use std::sync::Arc;

use iii_sdk::{register_worker, InitOptions, WorkerMetadata, III};

use crate::config::{StorageAdapter, WorkerConfig};
use crate::configuration::ConfigCell;
use crate::events::{
    attach_bridge_relay, BridgeSubscribers, Emitter, EventSink, IiiDeliverer, RemotePublisher,
    TriggerSets,
};
use crate::service::SessionService;
use crate::store::{BridgeStore, FsStore, SessionStore};

/// Which storage topology a runtime was built for. Gates the internal
/// `session::store::*` protocol: only an authoritative fs-mode instance serves
/// it (a bridge forwarding to itself would recurse).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterMode {
    /// Authoritative local fs store; serves the raw store protocol and is the
    /// single event fan-out point for any attached bridges.
    FsMain,
    /// Defers storage and event publishing to a main instance over `remote`.
    Bridge,
}

/// The live storage + event plumbing, rebuilt and swapped on an adapter change.
pub struct SessionRuntime {
    /// The config this runtime was built from (its `adapter` half is what the
    /// reload path compares to decide rebuild-vs-tune).
    pub config: Arc<WorkerConfig>,
    /// Domain logic over `store`; what the 14 `session::*` handlers call.
    pub service: Arc<SessionService>,
    /// Raw store the `session::store::*` protocol serves (fs mode only).
    pub store: Arc<dyn SessionStore>,
    /// Where a mutation's events go: the local [`Emitter`] (fs) or a
    /// [`RemotePublisher`] to the main (bridge).
    pub sink: Arc<dyn EventSink>,
    /// Local trigger fan-out. In fs mode this IS the `sink`; in bridge mode the
    /// relay feeds it and the post-reload resync replays through it.
    pub local_emitter: Arc<Emitter>,
    pub mode: AdapterMode,
    /// The remote engine connection (bridge mode only); shut down when this
    /// runtime is retired.
    pub bridge_remote: Option<Arc<III>>,
}

/// The boot-time wiring `build_runtime` reuses on every rebuild: the engine
/// handle, the six subscriber sets, the bridge relay registry, and the shared
/// config snapshot the service reads list limits from.
pub struct SessionBuildContext {
    pub iii: Arc<III>,
    /// This instance's own engine url; a bridge whose `url` equals it would
    /// defer storage to itself, so that is rejected.
    pub local_url: String,
    pub sets: TriggerSets,
    pub bridge_subscribers: BridgeSubscribers,
    pub config_cell: ConfigCell,
}

/// Worker identity advertised on both the local and (bridge mode) remote
/// connections.
pub fn worker_metadata() -> WorkerMetadata {
    WorkerMetadata {
        runtime: "rust".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: "session-manager".to_string(),
        os: std::env::consts::OS.to_string(),
        pid: Some(std::process::id()),
        telemetry: None,
        ..WorkerMetadata::default()
    }
}

/// Build the storage + event runtime for `cfg`'s adapter. Pure of any global
/// state, so the reload path can build a candidate and only swap it in on
/// success (a failed build leaves the running runtime untouched).
pub fn build_runtime(
    cfg: &WorkerConfig,
    ctx: &SessionBuildContext,
) -> Result<SessionRuntime, String> {
    match cfg.resolve_adapter() {
        StorageAdapter::Fs(fs_cfg) => {
            let data_dir = fs_cfg.resolved_data_dir();
            let store: Arc<dyn SessionStore> = Arc::new(
                FsStore::new(&data_dir)
                    .map_err(|e| format!("open data_dir {}: {e}", data_dir.display()))?,
            );

            // Main mode: the local emitter fans out to subscribers AND to every
            // attached bridge, and doubles as the mutation sink.
            let emitter = Arc::new(Emitter::with_bridges(
                ctx.sets.clone(),
                Arc::new(IiiDeliverer::new(ctx.iii.clone())),
                ctx.bridge_subscribers.clone(),
            ));
            let service = Arc::new(SessionService::with_config_cell(
                store.clone(),
                ctx.config_cell.clone(),
            ));
            let sink: Arc<dyn EventSink> = emitter.clone();
            tracing::info!(data_dir = %data_dir.display(), "adapter: fs (main)");
            Ok(SessionRuntime {
                config: Arc::new(cfg.clone()),
                service,
                store,
                sink,
                local_emitter: emitter,
                mode: AdapterMode::FsMain,
                bridge_remote: None,
            })
        }
        StorageAdapter::Bridge(bridge_cfg) => {
            // Exact match is unambiguous misconfiguration: the bridge would
            // defer storage to itself and hang on the first call. (Aliased URLs
            // of the same engine can still slip through — best-effort guard.)
            if bridge_cfg.url == ctx.local_url {
                return Err(format!(
                    "bridge url {} equals the local engine url {} — this instance would defer \
                     to itself; fix the bridge adapter `config.url`",
                    bridge_cfg.url, ctx.local_url
                ));
            }
            let remote = Arc::new(register_worker(
                &bridge_cfg.url,
                InitOptions {
                    metadata: Some(worker_metadata()),
                    ..InitOptions::default()
                },
            ));

            let store: Arc<dyn SessionStore> =
                Arc::new(BridgeStore::new(remote.clone(), bridge_cfg.timeout_ms));

            // Local emitter serves local subscribers; it is fed by the relay
            // (events coming back from the main), never directly by the handlers.
            let local_emitter = Arc::new(Emitter::new(
                ctx.sets.clone(),
                Arc::new(IiiDeliverer::new(ctx.iii.clone())),
            ));
            let relay = attach_bridge_relay(&remote, local_emitter.clone());

            let service = Arc::new(SessionService::with_config_cell(
                store.clone(),
                ctx.config_cell.clone(),
            ));
            let sink: Arc<dyn EventSink> =
                Arc::new(RemotePublisher::new(remote.clone(), bridge_cfg.timeout_ms));
            tracing::info!(main = %bridge_cfg.url, relay = %relay, "adapter: bridge");
            Ok(SessionRuntime {
                config: Arc::new(cfg.clone()),
                service,
                store,
                sink,
                local_emitter,
                mode: AdapterMode::Bridge,
                bridge_remote: Some(remote),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BridgeBackendConfig, FsBackendConfig};

    // register_worker returns immediately without connecting (a background
    // thread retries silently), so build_runtime is unit-testable without a
    // live engine.
    fn ctx(local_url: &str) -> SessionBuildContext {
        let iii = Arc::new(register_worker(local_url, InitOptions::default()));
        let config_cell = Arc::new(tokio::sync::RwLock::new(Arc::new(WorkerConfig::default())));
        SessionBuildContext {
            iii,
            local_url: local_url.to_string(),
            sets: TriggerSets::new(),
            bridge_subscribers: BridgeSubscribers::new(),
            config_cell,
        }
    }

    #[test]
    fn build_fs_runtime_opens_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = WorkerConfig {
            adapter: StorageAdapter::Fs(FsBackendConfig {
                data_dir: tmp.path().display().to_string(),
            }),
            ..WorkerConfig::default()
        };
        let rt = build_runtime(&cfg, &ctx("ws://127.0.0.1:59621")).expect("fs runtime builds");
        assert_eq!(rt.mode, AdapterMode::FsMain);
        assert!(rt.bridge_remote.is_none());
    }

    #[test]
    fn build_bridge_runtime_rejects_self_reference() {
        let cfg = WorkerConfig {
            adapter: StorageAdapter::Bridge(BridgeBackendConfig {
                url: "ws://127.0.0.1:59622".to_string(),
                timeout_ms: 5000,
            }),
            ..WorkerConfig::default()
        };
        // A bridge whose url equals the local engine would defer to itself.
        let err = build_runtime(&cfg, &ctx("ws://127.0.0.1:59622"))
            .err()
            .expect("self-referential bridge is rejected");
        assert!(err.contains("defer to itself"), "unexpected error: {err}");
    }

    #[test]
    fn build_bridge_runtime_to_distinct_url() {
        let cfg = WorkerConfig {
            adapter: StorageAdapter::Bridge(BridgeBackendConfig {
                url: "ws://127.0.0.1:59624".to_string(),
                timeout_ms: 1234,
            }),
            ..WorkerConfig::default()
        };
        let rt = build_runtime(&cfg, &ctx("ws://127.0.0.1:59623")).expect("bridge runtime builds");
        assert_eq!(rt.mode, AdapterMode::Bridge);
        assert!(rt.bridge_remote.is_some());
    }
}
