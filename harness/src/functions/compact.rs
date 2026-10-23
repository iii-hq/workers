//! `harness::compact` — compact a session now and wait for the outcome (the
//! console's model switch needs it before switching). The `/compact` command
//! through `harness::send` starts the same compaction without waiting; both
//! report to every client watching the session ([`crate::compaction`]).

use schemars::JsonSchema;
use serde::Deserialize;

use crate::compaction::{Outcome, Request};
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::ids;
use crate::types::turn::TurnRecord;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CompactRequest {
    pub session_id: String,
    /// The model that writes the summary; omitted, the session's last turn's.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    /// Guidance for the summariser: what to keep, drop, or emphasise. Steers
    /// this compaction only.
    #[serde(default)]
    pub instructions: Option<String>,
}

const NO_MODEL: &str =
    "harness::compact: this session has no turn yet — name the `model` that writes the summary";

pub async fn handle(deps: &Deps, req: CompactRequest) -> Result<Outcome, HarnessError> {
    let prev = compactable_session(deps, &req.session_id).await?;
    let (model, provider) = match (req.model, prev) {
        (Some(model), _) => (model, req.provider),
        (None, Some(prev)) => (prev.options.model, req.provider.or(prev.options.provider)),
        (None, None) => return Err(HarnessError::InvalidRequest(NO_MODEL.into())),
    };
    let request = Request {
        session_id: req.session_id,
        model,
        provider,
        instructions: req
            .instructions
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty()),
        entry_id: None,
        // The caller shows its own progress and needs the session idle after.
        mark_working: false,
    };
    Ok(crate::compaction::run(deps, request, ids::new_compaction_id()).await)
}

/// The checks every compaction request passes: a live, existing, writable
/// session. Returns its turn record (the default summariser model).
pub(crate) async fn compactable_session(
    deps: &Deps,
    session_id: &str,
) -> Result<Option<TurnRecord>, HarnessError> {
    super::delete_session_tree::ensure_live(deps, session_id).await?;
    let metadata = deps
        .session()
        .await
        .metadata_of(session_id)
        .await?
        .ok_or_else(|| {
            HarnessError::InvalidRequest(format!(
                "nothing to compact: session `{session_id}` does not exist"
            ))
        })?;
    super::send::ensure_writable(&metadata, session_id)?;
    let cfg = deps.cfg().await;
    crate::state::get_turn(&deps.iii, session_id, cfg.session_timeout_ms).await
}
