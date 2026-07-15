//! `memory::rule::*` — the always-injected markdown documents. Plain
//! files under `<bank>/rules/`; editing them by hand is equivalent.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::MemoryError;
use crate::types::Rule;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RuleListRequest {
    /// Bank whose rules to list; the configured default when omitted.
    #[serde(default)]
    pub bank: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RuleListResponse {
    pub bank: String,
    pub rules: Vec<Rule>,
}

pub async fn list(deps: &Deps, req: RuleListRequest) -> Result<RuleListResponse, MemoryError> {
    let cfg = deps.config().await;
    let bank_name = req.bank.unwrap_or_else(|| cfg.default_bank.clone());
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    Ok(RuleListResponse {
        bank: bank_name,
        rules: bank.list_rules()?,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RuleSetRequest {
    /// Bank to write into; the configured default when omitted. Created
    /// on first use.
    #[serde(default)]
    pub bank: Option<String>,
    /// Rule name, `[a-z0-9][a-z0-9_-]{0,63}` (it becomes `<name>.md`).
    pub name: String,
    /// Markdown content. Empty removes the rule.
    pub content: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RuleSetResponse {
    pub ok: bool,
    /// False when the empty content removed the rule.
    pub exists: bool,
}

pub async fn set(deps: &Deps, req: RuleSetRequest) -> Result<RuleSetResponse, MemoryError> {
    let cfg = deps.config().await;
    let bank_name = req.bank.unwrap_or_else(|| cfg.default_bank.clone());
    let store = deps.store().await;
    let (bank, created) = store.ensure_bank(&bank_name, None).await?;
    if created {
        deps.emitter.bank("created", &bank_name).await;
    }
    let exists = !req.content.trim().is_empty();
    bank.set_rule(&req.name, &req.content)?;
    deps.emitter.bank("rules-changed", &bank_name).await;
    Ok(RuleSetResponse { ok: true, exists })
}
