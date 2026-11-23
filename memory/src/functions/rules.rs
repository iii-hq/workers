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
    // Redacted as injected: these rules reach models (the judge, agents
    // following the truncation marker), and a hand edit may hold a secret.
    let rules = bank
        .list_rules()?
        .into_iter()
        .map(|mut rule| {
            rule.content = crate::redact::redact(&rule.content).text;
            rule
        })
        .collect();
    Ok(RuleListResponse {
        bank: bank_name,
        rules,
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
    /// The rule's content as the caller last read it (`memory::rule::list`).
    /// The write is refused with `rule_changed` when the rule holds anything
    /// else by then. Empty: the rule must not exist yet. Omitted: replace
    /// without checking.
    #[serde(default)]
    pub expected: Option<String>,
}

/// In the error a stale `expected` gets; callers match on this substring.
pub const RULE_CHANGED: &str = "rule_changed";

fn rule_changed(name: &str) -> MemoryError {
    MemoryError::InvalidInput(format!(
        "{RULE_CHANGED}: rule `{name}` changed since `expected` was read"
    ))
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RuleSetResponse {
    pub ok: bool,
    /// False when the empty content removed the rule.
    pub exists: bool,
}

pub async fn set(deps: &Deps, req: RuleSetRequest) -> Result<RuleSetResponse, MemoryError> {
    crate::scope::human_only(super::RULE_SET_ID)?;
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
    // Compared redacted, like what the write stores and `rule::list` returns.
    let expected = req
        .expected
        .as_deref()
        .map(|e| crate::redact::redact(e).text);
    let bank = match store.bank(&bank_name).await {
        Ok(bank) => bank,
        // A missing bank holds no rule: it reads as `""`.
        Err(_) if expected.as_deref().is_some_and(|e| !e.is_empty()) => {
            return Err(rule_changed(&req.name));
        }
        // Deleting from a bank that does not exist must not create it: an
        // empty-content set against a missing bank is a no-op, not a
        // bank-plus-events side effect.
        Err(_) if !exists => return Ok(RuleSetResponse { ok: true, exists }),
        Err(_) => {
            let (bank, created) = store.ensure_bank(&bank_name, None).await?;
            if created {
                deps.emitter.bank("created", &bank_name).await;
            }
            bank
        }
    };
    // A `learned` write waits for a running demotion or extraction merge:
    // landing between its read and its rewrite would refuse that rewrite
    // and leave the lines it already saved as memories. Released before the
    // demotion below, which takes the lock itself.
    let learned_write = if req.name == crate::extract::LEARNED_RULE {
        Some(crate::learned::LEARNED_WRITES.lock().await)
    } else {
        None
    };
    let written = match &expected {
        Some(expected) => bank
            .extraction_rule_expected(&req.name, expected, &req.content)?
            .ok_or_else(|| rule_changed(&req.name))?,
        None => {
            bank.set_rule(&req.name, &req.content)?;
            true
        }
    };
    drop(learned_write);
    if written {
        deps.emitter.bank("rules-changed", &bank_name).await;
    }
    if written
        && exists
        && req.name == crate::extract::LEARNED_RULE
        && let Err(e) = crate::learned::demote_over_cap(deps, &bank_name).await
    {
        tracing::warn!(bank = %bank_name, error = %e, "learned rule demotion failed");
    }
    Ok(RuleSetResponse { ok: true, exists })
}
