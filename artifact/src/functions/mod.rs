//! The worker's public surface: eight functions over one store and one
//! tunnel client.
//!
//! Every handler reads the live config snapshot per call, so a configuration
//! change takes effect on the next invocation (except `listen_addr`).

pub mod artifacts;
pub mod sharing;

use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::WorkerConfig;
use crate::configuration::ConfigCell;
use crate::dns::Dns;
use crate::store::Store;
use crate::tunnel::Tunnel;

/// What every handler works against.
pub struct Ctx {
    pub store: Arc<Store>,
    pub tunnel: Arc<Tunnel>,
    /// Public DNS check for fresh tunnel hostnames.
    pub dns: Arc<Dns>,
    /// Base URL of the share listener as bound, e.g. `http://127.0.0.1:3120`.
    pub local_base: String,
}

impl Ctx {
    /// An engine-free context for tests.
    pub fn for_tests(public_url: Option<&str>) -> Arc<Self> {
        Arc::new(Self {
            store: Arc::new(Store::in_memory()),
            tunnel: Arc::new(Tunnel::fake(public_url)),
            dns: Arc::new(Dns::fake(true)),
            local_base: "http://127.0.0.1:3120".to_string(),
        })
    }
}

/// One entry of the wire surface: what a caller sees for one function.
pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

/// Build a schema the same way iii-sdk does at registration.
fn schema_of<T: JsonSchema>() -> schemars::schema::RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req: JsonSchema, Resp: JsonSchema>(
    function_id: &'static str,
    description: &'static str,
) -> FunctionSpec {
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

/// The full wire-surface catalog, in registration order. Keep in lockstep
/// with [`register_all`].
pub fn catalog() -> Vec<FunctionSpec> {
    use artifacts as a;
    use sharing as s;
    vec![
        spec::<a::CreateRequest, a::CreateResponse>(a::CREATE_ID, a::CREATE_DESC),
        spec::<a::UpdateRequest, a::UpdateResponse>(a::UPDATE_ID, a::UPDATE_DESC),
        spec::<a::GetRequest, a::GetResponse>(a::GET_ID, a::GET_DESC),
        spec::<a::ListRequest, a::ListResponse>(a::LIST_ID, a::LIST_DESC),
        spec::<a::DeleteRequest, a::DeleteResponse>(a::DELETE_ID, a::DELETE_DESC),
        spec::<s::ShareRequest, s::ShareView>(s::SHARE_ID, s::SHARE_DESC),
        spec::<s::SharesRequest, s::SharesResponse>(s::SHARES_ID, s::SHARES_DESC),
        spec::<s::RevokeRequest, s::RevokeResponse>(s::REVOKE_ID, s::REVOKE_DESC),
    ]
}

/// Register one function whose handler gets the shared context and the live
/// config snapshot.
fn register<Req, Resp, F, Fut>(
    iii: &IIIClient,
    ctx: &Arc<Ctx>,
    cell: &ConfigCell,
    id: &'static str,
    desc: &'static str,
    handler: F,
) where
    Req: DeserializeOwned + JsonSchema + Send + 'static,
    Resp: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Ctx>, Arc<WorkerConfig>, Req) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<Resp, String>> + Send + 'static,
{
    let ctx = ctx.clone();
    let cell = cell.clone();
    iii.register_function(
        id,
        RegisterFunction::new_async(move |req: Req| {
            let ctx = ctx.clone();
            let cell = cell.clone();
            let handler = handler.clone();
            async move {
                let cfg = cell.read().await.clone();
                handler(ctx, cfg, req).await.map_err(Error::Handler)
            }
        })
        .description(desc),
    );
}

pub fn register_all(iii: &IIIClient, ctx: &Arc<Ctx>, cell: &ConfigCell) {
    use artifacts as a;
    use sharing as s;
    register(iii, ctx, cell, a::CREATE_ID, a::CREATE_DESC, a::create);
    register(iii, ctx, cell, a::UPDATE_ID, a::UPDATE_DESC, a::update);
    register(iii, ctx, cell, a::GET_ID, a::GET_DESC, a::get);
    register(iii, ctx, cell, a::LIST_ID, a::LIST_DESC, a::list);
    register(iii, ctx, cell, a::DELETE_ID, a::DELETE_DESC, a::delete);
    register(iii, ctx, cell, s::SHARE_ID, s::SHARE_DESC, s::share);
    register(iii, ctx, cell, s::SHARES_ID, s::SHARES_DESC, s::shares);
    register(iii, ctx, cell, s::REVOKE_ID, s::REVOKE_DESC, s::revoke);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_every_function_in_registration_order() {
        let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
        assert_eq!(
            ids,
            vec![
                "artifact::create",
                "artifact::update",
                "artifact::get",
                "artifact::list",
                "artifact::delete",
                "artifact::share",
                "artifact::shares",
                "artifact::revoke",
            ]
        );
    }

    #[test]
    fn every_function_carries_an_actionable_description() {
        for spec in catalog() {
            assert!(spec.description.len() > 40, "{}", spec.function_id);
        }
    }
}
