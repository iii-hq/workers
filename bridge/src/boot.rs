//! Boot sequence: builtin guard -> connect the remote client -> register
//! `bridge.invoke`/`bridge.invoke_async` + forward functions on the LOCAL
//! engine -> register expose functions on the REMOTE engine.
//!
//! `register_worker` connects in the background (same as the builtin,
//! mod.rs:84): an unreachable remote does not fail boot — calls fail until
//! the connection is up, exactly like the builtin behaved.

use std::collections::HashSet;
use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, Error, IIIClient, InitOptions, RegisterFunction};
use tokio::sync::RwLock;

use crate::config::BridgeConfig;
use crate::configuration::{ApplyLock, ConfigCell};
use crate::functions::{
    self, ExposeTable, ForwardTable, LocalCaller, RawValue, RemoteCaller, RemoteCell,
};

const LIST_WORKERS_FUNCTION_ID: &str = "engine::workers::list";
const BUILTIN_III_BRIDGE_WORKER_ID: &str = "iii-bridge";

/// Function ids EVER registered on a client. The SDK panics on a duplicate
/// `register_function` id and has no unregister, so hot-reload must decide
/// "still needs registering?" against this set, never against the current
/// tables (a removed-then-re-added entry is absent from the tables but still
/// registered). See `configuration::on_config_change`.
pub type RegisteredIds = Arc<RwLock<HashSet<String>>>;

pub struct BootHandle {
    pub remote: RemoteCell,
    pub forwards: ForwardTable,
    pub exposes: ExposeTable,
    pub config: ConfigCell,
    /// Ids ever registered on the LOCAL client: `bridge.invoke`,
    /// `bridge.invoke_async`, and every forward's local function.
    pub local_registered: RegisteredIds,
    /// Ids ever registered on the CURRENT remote-client generation (expose
    /// names). Reset when a `url` change swaps in a fresh client.
    pub remote_registered: RegisteredIds,
    pub apply_lock: ApplyLock,
}

impl BootHandle {
    /// Disconnect the remote client. The local connection is owned by main.
    pub async fn shutdown(&self) {
        let client = self.remote.read().await.clone();
        client.shutdown_async().await;
    }
}

pub async fn start(iii: Arc<IIIClient>, config: BridgeConfig) -> anyhow::Result<BootHandle> {
    guard_against_builtin_bridge(&iii).await?;

    let remote_client = Arc::new(register_worker(
        &config.effective_url(),
        InitOptions::default(),
    ));
    let remote: RemoteCell = Arc::new(RwLock::new(remote_client));

    let forwards: ForwardTable = Arc::new(RwLock::new(
        config
            .forward
            .iter()
            .map(|f| (f.local_function.clone(), f.clone()))
            .collect(),
    ));
    let exposes: ExposeTable = Arc::new(RwLock::new(
        config
            .expose
            .iter()
            .map(|e| (e.remote_name().to_string(), e.clone()))
            .collect(),
    ));

    register_bridge_functions(&iii, remote.clone());
    for f in &config.forward {
        register_forward_function(
            &iii,
            remote.clone(),
            forwards.clone(),
            &f.local_function,
            &f.remote_function,
        );
    }
    {
        let client = remote.read().await.clone();
        for e in &config.expose {
            register_expose_function(&client, iii.clone(), exposes.clone(), e.remote_name());
        }
    }

    // Seed the ever-registered sets with exactly what boot registered above.
    let local_registered: RegisteredIds = Arc::new(RwLock::new(
        [functions::INVOKE_FN, functions::INVOKE_ASYNC_FN]
            .into_iter()
            .map(str::to_string)
            .chain(config.forward.iter().map(|f| f.local_function.clone()))
            .collect(),
    ));
    let remote_registered: RegisteredIds = Arc::new(RwLock::new(
        config
            .expose
            .iter()
            .map(|e| e.remote_name().to_string())
            .collect(),
    ));

    Ok(BootHandle {
        remote,
        forwards,
        exposes,
        config: Arc::new(RwLock::new(Arc::new(config.normalized()))),
        local_registered,
        remote_registered,
        apply_lock: Arc::new(tokio::sync::Mutex::new(())),
    })
}

/// `bridge.invoke` + `bridge.invoke_async` on the local engine — exact
/// function ids and descriptions from the builtin (mod.rs:96-191).
///
/// Both use `new_async_with_bad_request` with the typed [`functions::
/// InvokeInput`]: the SDK auto-extracts a real request schema instead of the
/// permissive `AnyValue` a `Value` closure param would emit, while
/// [`functions::invoke_bad_request`] keeps owning the `deserialization_error`
/// / "Failed to parse invoke input: {err}" contract for malformed payloads.
pub fn register_bridge_functions(iii: &Arc<IIIClient>, remote: RemoteCell) {
    let caller = Arc::new(RemoteCaller { cell: remote });
    {
        let caller = caller.clone();
        iii.register_function(
            functions::INVOKE_FN,
            RegisterFunction::new_async_with_bad_request(
                move |req: functions::InvokeInput| {
                    let caller = caller.clone();
                    async move {
                        functions::handle_invoke_typed(caller.as_ref(), req)
                            .await
                            .map_err(Error::from)
                    }
                },
                functions::invoke_bad_request,
            )
            .description("Invoke a function on the remote III instance"),
        );
    }
    iii.register_function(
        functions::INVOKE_ASYNC_FN,
        RegisterFunction::new_async_with_bad_request(
            move |req: functions::InvokeInput| {
                let caller = caller.clone();
                async move {
                    functions::handle_invoke_async_typed(caller.as_ref(), req)
                        .await
                        .map_err(Error::from)
                }
            },
            functions::invoke_bad_request,
        )
        .description("Fire-and-forget invoke on the remote III instance"),
    );
}

/// One local proxy function per `forward` entry (mod.rs:193-237).
///
/// Request and response are [`RawValue`]: a `forward` entry's shape is
/// whatever the user-configured remote function expects/returns, so the only
/// schema that can be published here is the permissive-but-typed union —
/// `#[serde(transparent)]` means deserialization can never fail.
pub fn register_forward_function(
    iii: &Arc<IIIClient>,
    remote: RemoteCell,
    forwards: ForwardTable,
    local_function: &str,
    remote_function: &str,
) {
    let caller = Arc::new(RemoteCaller { cell: remote });
    let local = local_function.to_string();
    iii.register_function(
        local_function,
        RegisterFunction::new_async(move |input: RawValue| {
            let caller = caller.clone();
            let local = local.clone();
            let forwards = forwards.clone();
            async move {
                functions::handle_forward(caller.as_ref(), &forwards, &local, input.into())
                    .await
                    .map(RawValue)
                    .map_err(Error::from)
            }
        })
        .description(format!("Forward to remote function {remote_function}")),
    );
}

/// One function per `expose` entry, registered ON THE REMOTE engine and
/// backed by a local call (mod.rs:240-270).
///
/// Request and response are [`RawValue`] for the same reason as
/// [`register_forward_function`]: an `expose` entry's shape is whatever the
/// user-configured local function expects/returns.
pub fn register_expose_function(
    remote_client: &Arc<IIIClient>,
    iii: Arc<IIIClient>,
    exposes: ExposeTable,
    remote_name: &str,
) {
    let local = Arc::new(LocalCaller { iii });
    let name = remote_name.to_string();
    remote_client.register_function(
        remote_name,
        RegisterFunction::new_async(move |input: RawValue| {
            let local = local.clone();
            let name = name.clone();
            let exposes = exposes.clone();
            async move {
                functions::handle_expose(local.as_ref(), &exposes, &name, input.into())
                    .await
                    .map(RawValue)
                    .map_err(Error::from)
            }
        }),
    );
}

/// Query the engine for connected workers and bail out if the built-in
/// `iii-bridge` worker is active — it registers the same function ids
/// (`bridge.invoke`, `bridge.invoke_async`, its forward/expose names), and
/// duplicate function registration silently collides (last-write-wins).
async fn guard_against_builtin_bridge(iii: &Arc<IIIClient>) -> anyhow::Result<()> {
    let workers_list = iii
        .trigger(TriggerRequest {
            function_id: LIST_WORKERS_FUNCTION_ID.to_string(),
            payload: serde_json::json!({}),
            action: None,
            timeout_ms: Some(5000),
        })
        .await
        .map_err(|e| anyhow::anyhow!("failed to query {LIST_WORKERS_FUNCTION_ID}: {e}"))?;

    if builtin_iii_bridge_active(&workers_list) {
        anyhow::bail!(
            "cannot start the bridge worker: the built-in iii-bridge worker is active and \
             already registers bridge.invoke / bridge.invoke_async (plus its forward and \
             expose functions) on this engine — duplicate function registrations silently \
             collide (last-write-wins). Remove iii-bridge from the engine config (a \
             config.yaml that doesn't list it won't run it), then start this worker."
        );
    }
    Ok(())
}

fn builtin_iii_bridge_active(workers_list: &serde_json::Value) -> bool {
    workers_list
        .get("workers")
        .and_then(|w| w.as_array())
        .is_some_and(|workers| {
            workers.iter().any(|worker| {
                worker.get("id").and_then(|v| v.as_str()) == Some(BUILTIN_III_BRIDGE_WORKER_ID)
                    || worker.get("name").and_then(|v| v.as_str())
                        == Some(BUILTIN_III_BRIDGE_WORKER_ID)
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_builtin_by_id() {
        let v = serde_json::json!({"workers": [{"id": "iii-bridge"}]});
        assert!(builtin_iii_bridge_active(&v));
    }

    #[test]
    fn detects_builtin_by_name() {
        let v = serde_json::json!({"workers": [{"id": "x", "name": "iii-bridge"}]});
        assert!(builtin_iii_bridge_active(&v));
    }

    #[test]
    fn absent_builtin_passes() {
        let v = serde_json::json!({"workers": [{"id": "iii-http"}]});
        assert!(!builtin_iii_bridge_active(&v));
    }

    #[test]
    fn empty_list_passes() {
        assert!(!builtin_iii_bridge_active(
            &serde_json::json!({"workers": []})
        ));
    }

    #[test]
    fn missing_key_passes() {
        assert!(!builtin_iii_bridge_active(&serde_json::json!({})));
    }
}
