//! Function registrations against the engine over the **control connection**.
//!
//! `rbac-proxy` is a pure boundary: it registers exactly one public function
//! ([`status`]) for health probes. The internal config-reload handler
//! (`rbac-proxy::on-config-change`) is registered separately in
//! [`crate::configuration`] (off this public catalog), mirroring
//! approval-gate / context-manager.

pub mod status;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};

use crate::redact_url;
use crate::server::ProxyState;
use status::{StatusInput, StatusOutput, STATUS_DESC, STATUS_ID};

/// Register every public `rbac-proxy::*` function. Called once from `main`
/// after the control connection and the proxy state are built.
pub fn register_all(iii: &Arc<IIIClient>, state: &ProxyState) {
    let state = state.clone();
    iii.register_function(
        STATUS_ID,
        RegisterFunction::new_async(move |_: StatusInput| {
            let state = state.clone();
            async move {
                let cfg = state.config.read().await.clone();
                let (host, port) = state.bound.read().await.clone();
                Ok::<StatusOutput, Error>(StatusOutput {
                    host,
                    port,
                    engine_url: redact_url(&cfg.engine_url),
                    rbac_enabled: cfg.rbac_enabled(),
                    active_connections: state.active.load(Ordering::Relaxed),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                })
            }
        })
        .description(STATUS_DESC),
    );
    tracing::info!("registered {STATUS_ID}");
}

// ---------------------------------------------------------------------------
// Wire-surface catalog — golden-tested in tests/schemas.rs. Keep in lockstep
// with register_all (the one public function).
// ---------------------------------------------------------------------------

/// One function's complete agent-facing wire surface.
pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

/// Schema generation MUST mirror iii-sdk's internal generator
/// (`SchemaSettings::draft07()`), so a catalog snapshot equals what
/// registration emits.
fn schema_of<T: schemars::JsonSchema>() -> schemars::schema::RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req, Resp>(function_id: &'static str, description: &'static str) -> FunctionSpec
where
    Req: schemars::JsonSchema,
    Resp: schemars::JsonSchema,
{
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

/// The full public wire-surface catalog, in registration order.
pub fn catalog() -> Vec<FunctionSpec> {
    vec![spec::<StatusInput, StatusOutput>(STATUS_ID, STATUS_DESC)]
}
