//! `memory::rule::*` — the always-injected markdown documents. Plain
//! files under `<bank>/rules/`; editing them by hand is equivalent.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::MemoryError;
use crate::types::Rule;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RuleListRequest {
    /// Bank whose rules to list; omitted: the calling session's bank, else the configured default.
    #[serde(default)]
    pub bank: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RuleListResponse {
    pub bank: String,
    pub rules: Vec<Rule>,
}

pub async fn list(deps: &Deps, req: RuleListRequest) -> Result<RuleListResponse, MemoryError> {
    let bank_name = crate::scope::request_bank(deps, req.bank.clone()).await?;
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    Ok(RuleListResponse {
        bank: bank_name,
        rules: bank.list_rules()?,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RuleSetRequest {
    /// Bank to write into; omitted: the calling session's bank, else the configured default.
    /// Created on first use.
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
    let bank_name = crate::scope::request_bank(deps, req.bank.clone()).await?;
    let store = deps.store().await;
    // A shape within a line the stored rule already holds is spared: one
    // written before the screen, or by hand, must not block every later
    // write of the document (memory-consolidate rewrites `learned` whole).
    // The whole content is read, so a new line cannot finish a held line's
    // phrase.
    let held = store
        .bank(&bank_name)
        .await
        .ok()
        .and_then(|bank| bank.list_rules().ok())
        .and_then(|rules| rules.into_iter().find(|r| r.name == req.name))
        .map(|r| r.content)
        .unwrap_or_default();
    crate::redact::reject_injection_beside("content", &req.content, &held)?;
    // What `set_rule` keeps: invisible-only content removes the rule.
    let exists = !crate::redact::redact(&req.content).text.trim().is_empty();
    // Deleting from a bank that does not exist must not create it: an
    // empty-content set against a missing bank is a no-op, not a
    // bank-plus-events side effect.
    if !exists {
        let Ok(bank) = store.bank(&bank_name).await else {
            return Ok(RuleSetResponse { ok: true, exists });
        };
        bank.set_rule(&req.name, &req.content)?;
        deps.emitter.bank("rules-changed", &bank_name).await;
        return Ok(RuleSetResponse { ok: true, exists });
    }
    let (bank, created) = store.ensure_bank(&bank_name, None).await?;
    if created {
        deps.emitter.bank("created", &bank_name).await;
    }
    bank.set_rule(&req.name, &req.content)?;
    deps.emitter.bank("rules-changed", &bank_name).await;
    Ok(RuleSetResponse { ok: true, exists })
}
