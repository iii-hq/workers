//! Registration-token persistence, scoped to this provider's worker id
//! (shared logic in `llm_router::provider_scaffold::state`).
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::provider_scaffold::state as scaffold;

pub const STATE_SCOPE: &str = "provider-openai";

pub async fn load_token(iii: &IIIClient) -> Option<String> {
    scaffold::load_token(iii, STATE_SCOPE).await
}

pub async fn store_token(iii: &IIIClient, token: &str) -> Result<(), Error> {
    scaffold::store_token(iii, STATE_SCOPE, token).await
}
