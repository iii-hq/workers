//! `harness::teardown` — remove trigger bindings owned by a complete harness
//! session tree. This is trusted control-plane cleanup for evals and other
//! lifecycle owners; it is not exposed to models.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TeardownRequestV1 {
    pub root_session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TeardownResponseV1 {
    pub removed: u64,
}

pub async fn handle(
    deps: &Deps,
    req: TeardownRequestV1,
) -> Result<TeardownResponseV1, HarnessError> {
    let tree = super::session_tree::collect(deps, &req.root_session_id).await?;
    let mut removed = 0u64;
    for node in tree.sessions {
        removed = removed.saturating_add(cleanup_session(deps, &node.session_id).await);
    }
    Ok(TeardownResponseV1 { removed })
}

pub(crate) async fn cleanup_session(deps: &Deps, session_id: &str) -> u64 {
    // The binding store is the single authority: one durable sweep tears the
    // session's bindings down engine-side and deletes their records.
    let removed = crate::bindings::gc::sweep_owner(deps, session_id).await as u64;
    if removed > 0 {
        tracing::info!(
            session_id,
            removed,
            "harness session trigger bindings removed"
        );
    }
    removed
}
