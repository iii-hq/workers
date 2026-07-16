//! `memory::preview` — the full injection dry-run. Where `memory::recall`
//! ranks memories, preview composes the ENTIRE memory payload a turn on
//! this bank would receive: the rules section exactly as it lands in the
//! system prompt (same budget, same truncation markers), the recalled
//! memories AFTER the ambient floor and token budget, and the appended
//! `<memory>` message verbatim. Runs the same code the pre-generate hook
//! runs — this cannot drift from production behavior.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::MemoryError;
use crate::hooks;
use crate::types::Memory;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PreviewRequest {
    /// Bank to preview; the configured default when omitted.
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
    /// Retrieval mode that ran (`bm25-entity` or `bm25-entity-semantic`).
    pub retrieval: String,
}

pub async fn preview(deps: &Deps, req: PreviewRequest) -> Result<PreviewResponse, MemoryError> {
    let cfg = deps.config().await;
    let bank_name = req.bank.unwrap_or_else(|| cfg.default_bank.clone());
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;

    let mut section = hooks::ambient_header(&bank_name);
    let rules = bank.list_rules().unwrap_or_default();
    let (rules_part, rules_truncated) = hooks::build_rules_section(&rules, cfg.max_rule_chars);
    if cfg.inject_rules {
        section.push_str(&rules_part);
    }

    let query_vec = crate::embed_client::query_vector(deps, &req.query).await;
    let retrieval = if query_vec.is_some() {
        "bm25-entity-semantic"
    } else {
        "bm25-entity"
    };
    let hits = bank
        .recall(
            &req.query,
            query_vec.as_deref(),
            cfg.recall_limit,
            cfg.decay_half_life_days,
            false,
        )
        .await;
    let mut scores: std::collections::HashMap<String, f32> =
        hits.iter().map(|(m, s)| (m.id.clone(), *s)).collect();
    let hit_memories: Vec<Memory> = hits.into_iter().map(|(m, _)| m).collect();
    let top = bank.top_memories(cfg.recall_limit).await;
    let memories = hooks::apply_ambient_floor(hit_memories, top, cfg.recall_limit);

    let (message, included) =
        match hooks::memories_message(&bank_name, &memories, cfg.recall_budget_tokens) {
            Some((body, ids)) => (Some(body), ids),
            None => (None, Vec::new()),
        };
    let included: std::collections::HashSet<String> = included.into_iter().collect();

    Ok(PreviewResponse {
        bank: bank_name,
        system_prompt_section: section,
        rules: rules.len(),
        rules_truncated,
        memories: memories
            .into_iter()
            .filter(|m| included.contains(&m.id))
            .map(|m| {
                let score = scores.remove(&m.id).unwrap_or(0.0);
                PreviewMemory { memory: m, score }
            })
            .collect(),
        message: if cfg.inject_memories { message } else { None },
        retrieval: retrieval.into(),
    })
}
