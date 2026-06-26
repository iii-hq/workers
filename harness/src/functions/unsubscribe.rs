//! `harness::unsubscribe` — tear down a subscription created by
//! `harness::subscribe`. Owner-checked and idempotent.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UnsubscribeRequest {
    /// Owning session; injected by the harness from the calling turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub subscription_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UnsubscribeResponse {
    /// True when a subscription existed and was removed.
    pub removed: bool,
}

pub async fn handle(
    deps: &Deps,
    req: UnsubscribeRequest,
) -> Result<UnsubscribeResponse, HarnessError> {
    // Owner check: only the owning session may tear down its subscription.
    if let (Some(entry), Some(caller)) = (
        deps.subscriptions.get(&req.subscription_id),
        req.session_id.as_ref(),
    ) {
        if &entry.session_id != caller {
            return Err(HarnessError::InvalidRequest(
                "subscription belongs to a different session".to_string(),
            ));
        }
    }
    let removed = deps.subscriptions.remove(&req.subscription_id);
    Ok(UnsubscribeResponse { removed })
}
