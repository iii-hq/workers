//! Token-gated provider diagnostic updates.
use std::sync::Arc;

use futures::future::BoxFuture;
use iii_sdk::errors::Error;

use crate::registry::store::RegistryStore;
use crate::types::router::{ProviderStatusRequest, ProviderStatusResponse};

pub fn make_provider_status(
    registry: Arc<RegistryStore>,
) -> impl Fn(ProviderStatusRequest) -> BoxFuture<'static, Result<ProviderStatusResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |req: ProviderStatusRequest| {
        let registry = registry.clone();
        Box::pin(async move {
            let diagnostic = registry.update_diagnostic(req).await.map_err(Error::from)?;
            Ok(ProviderStatusResponse {
                ok: true,
                diagnostic,
            })
        })
    }
}
