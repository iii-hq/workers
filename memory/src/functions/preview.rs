//! `memory::preview` — the full injection dry-run. Where `memory::recall`
//! ranks memories, preview composes the ENTIRE memory payload a turn on
//! this bank would receive: the rules section exactly as it lands in the
//! system prompt (same budget, same truncation markers, same on/off
//! switches), the recalled memories AFTER the ambient floor and token
//! budget, and the appended `<memory>` message verbatim. Built by the
//! pre-generate hook's own [`hooks::injection`] — it cannot drift.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::MemoryError;
use crate::hooks;
use crate::types::Memory;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PreviewRequest {
    /// Bank to preview; omitted: the calling session's bank, else the
    /// configured default.
    #[serde(default)]
    pub bank: Option<String>,
    /// The hypothetical user message (what someone would type in chat).
    pub query: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PreviewMemory {
    pub memory: Memory,
    /// Recall score; 0.0 for memories added by the ambient floor.
    pub score: f32,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PreviewResponse {
    pub bank: String,
    /// The memory section appended to the system prompt (ambient header +
    /// rules), exactly as injected — truncation markers included.
    pub system_prompt_section: String,
    /// Rules included, and whether any were cut by `max_rule_chars`.
    pub rules: usize,
    pub rules_truncated: bool,
    /// Memories the turn would be handed, post ambient floor and token
    /// budget, in injection order.
    pub memories: Vec<PreviewMemory>,
    /// The appended `<memory>` message body, verbatim; absent when no
    /// memory fits the budget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Retrieval mode that ran (`bm25-entity` or `bm25-entity-semantic`;
    /// `none` when memories are not injected).
    pub retrieval: String,
}

pub async fn preview(deps: &Deps, req: PreviewRequest) -> Result<PreviewResponse, MemoryError> {
    let cfg = deps.config().await;
    let bank_name = crate::scope::request_bank(deps, req.bank).await?;
    crate::store::validate_name(&bank_name)?;
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await.ok();
    // A deleted bank gives a session no memory at all (`scope`), not the
    // header an unknown one gets.
    let deleted = bank.is_none() && bank_name != cfg.default_bank && store.is_deleted(&bank_name);
    let injected = if deleted {
        hooks::Injection::none()
    } else {
        // A hypothetical message has no window: nothing was injected yet.
        let seen = std::collections::HashSet::new();
        hooks::injection(
            deps,
            &cfg,
            &bank_name,
            bank.as_deref(),
            Some(&req.query),
            &seen,
        )
        .await
    };
    Ok(PreviewResponse {
        bank: bank_name,
        system_prompt_section: injected.section,
        rules: injected.rules,
        rules_truncated: injected.rules_truncated,
        memories: injected
            .memories
            .into_iter()
            .map(|(memory, score)| PreviewMemory { memory, score })
            .collect(),
        message: injected.message,
        retrieval: injected.retrieval.into(),
    })
}
