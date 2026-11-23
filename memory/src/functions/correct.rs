//! `memory::transcript` and `memory::correct` — the internal seams of
//! consolidation's verify tier: read a note's source transcript (redacted
//! and cut, never raw), and replace a note that transcript contradicts.
//! Both refuse a call from inside an agent turn and have no REST route.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::MemoryError;
use crate::events::ItemEvent;
use crate::extract::source::{self, Line};
use crate::types::{Memory, Provenance, fingerprint, now_ms};

pub const TRANSCRIPT_ID: &str = "memory::transcript";
pub const TRANSCRIPT_DESC: &str = "Internal: the source transcript of one live memory, as redacted lines, a long line as its \
     head and tail (its recorded span, or the turn it was saved from). `missing` says why there \
     is none.";

pub const CORRECT_ID: &str = "memory::correct";
pub const CORRECT_DESC: &str = "Internal: replace a memory its source transcript contradicts. One append writes the new \
     record (the old provenance and statement date, plus the reason) and retires the old one \
     into it; `expected_memory` must still match the old record.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TranscriptRequest {
    /// Bank; omitted: the calling session's bank, else the configured default.
    #[serde(default)]
    pub bank: Option<String>,
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TranscriptResponse {
    pub lines: Vec<Line>,
    /// Why there are no lines: `not_found`, `edited`, `no_span`,
    /// `session_not_found`, `entry_not_found`, `off_path`, `turn_not_found`,
    /// `too_long`, `unreadable` or `empty`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing: Option<String>,
}

pub async fn transcript(
    deps: &Deps,
    req: TranscriptRequest,
) -> Result<TranscriptResponse, MemoryError> {
    crate::scope::human_only(TRANSCRIPT_ID)?;
    let bank_name = crate::scope::request_bank(deps, req.bank).await?;
    let bank = deps.store().await.bank(&bank_name).await?;
    let missing = |missing: &str| TranscriptResponse {
        lines: Vec::new(),
        missing: Some(missing.into()),
    };
    let Some(memory) = bank.get(&req.id).await.filter(Memory::is_live) else {
        return Ok(missing("not_found"));
    };
    // Its text no longer matches the span it was read from.
    if fingerprint(&memory.text) != memory.id {
        return Ok(missing("edited"));
    }
    let source = memory.source.unwrap_or_default();
    Ok(match source::read(&deps.iii, &source).await? {
        Ok(lines) => TranscriptResponse {
            lines,
            missing: None,
        },
        Err(why) => missing(why),
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CorrectRequest {
    /// Bank; omitted: the calling session's bank, else the configured default.
    #[serde(default)]
    pub bank: Option<String>,
    /// The memory to correct.
    pub id: String,
    /// Its corrected text.
    pub text: String,
    /// Why, stored on the new record and carried on the events.
    pub reason: String,
    /// The snapshot of the memory the correction was judged on, as
    /// `memory::supersede` takes it.
    pub expected_memory: serde_json::Value,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CorrectResponse {
    pub corrected: bool,
    /// The record that now holds the corrected text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<Memory>,
    /// The retired record.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded: Option<Memory>,
    /// Why nothing was written: `changed`, `pinned`, `edited`, `no_change`,
    /// `text_exists`, `invalid_text` or `flagged`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

fn skipped(why: &str) -> CorrectResponse {
    CorrectResponse {
        corrected: false,
        memory: None,
        superseded: None,
        skipped: Some(why.into()),
    }
}

pub async fn correct(deps: &Deps, req: CorrectRequest) -> Result<CorrectResponse, MemoryError> {
    crate::scope::human_only(CORRECT_ID)?;
    let bank_name = crate::scope::request_bank(deps, req.bank).await?;
    let bank = deps.store().await.bank(&bank_name).await?;
    let Some(old) = bank.get(&req.id).await.filter(Memory::is_live) else {
        return Ok(skipped("changed"));
    };
    if !crate::store::snapshot_matches(&old, &req.expected_memory)? {
        return Ok(skipped("changed"));
    }
    if old.pinned {
        return Ok(skipped("pinned"));
    }
    if fingerprint(&old.text) != old.id {
        return Ok(skipped("edited"));
    }
    let reason = super::items::clamp_reason(Some(crate::redact::redact(&req.reason).text))
        .ok_or_else(|| MemoryError::InvalidInput("reason must not be empty".into()))?;
    if crate::redact::reject_injection("reason", &reason).is_err() {
        return Ok(skipped("flagged"));
    }
    let text = crate::redact::redact(req.text.trim())
        .text
        .trim()
        .to_string();
    if text.len() < 3 || text.len() > 2_000 {
        return Ok(skipped("invalid_text"));
    }
    if crate::redact::reject_injection("text", &text).is_err() {
        return Ok(skipped("flagged"));
    }
    let id = fingerprint(&text);
    if id == old.id {
        return Ok(skipped("no_change"));
    }
    match bank.get(&id).await {
        // The right text is already a live memory (a torn earlier attempt,
        // or a fact stated elsewhere): the wrong one retires into it. Its
        // revision pins it: an edit before the append is `changed`.
        Some(twin) if twin.is_live() && fingerprint(&twin.text) == twin.id => {
            let twin_at = serde_json::json!({ "revision": twin.revision });
            let retired = match bank
                .supersede(
                    &old.id,
                    &id,
                    false,
                    (Some(&req.expected_memory), Some(&twin_at)),
                )
                .await
            {
                Ok((retired, _)) => retired,
                Err(MemoryError::InvalidInput(_)) => return Ok(skipped("changed")),
                Err(e) => return Err(e),
            };
            deps.emitter
                .item_with(ItemEvent::Superseded, &bank_name, &retired, Some(&reason))
                .await;
            return Ok(CorrectResponse {
                corrected: true,
                memory: Some(twin),
                superseded: Some(retired),
                skipped: None,
            });
        }
        // Retired once already: bringing it back would let a judge flip a
        // note back and forth. Or its text was edited away from this id: it
        // no longer holds the corrected fact.
        Some(_) => return Ok(skipped("text_exists")),
        None => {}
    }
    let now = now_ms();
    let replacement = Memory {
        id,
        text,
        entities: old.entities.clone(),
        tags: old.tags.clone(),
        confidence: old.confidence,
        // Model-written text is not promoted on the old text's evidence.
        corroboration: 0,
        pinned: false,
        source: Some(Provenance {
            correction_reason: Some(reason.clone()),
            ..old.source.clone().unwrap_or_default()
        }),
        created_at: now,
        updated_at: now,
        invalid_at: None,
        superseded_by: None,
        revision: 0,
        stated_at: Some(old.last_stated()),
    };
    let receipt = crate::extract::effect_receipt(
        &format!("correct:{}:{}:{}", old.id, old.revision, replacement.id),
        &bank_name,
        0,
    );
    let Some(Some(retired)) = bank
        .extraction_expected(&receipt, replacement.clone(), None, Some(&old))
        .await?
    else {
        return Ok(skipped("changed"));
    };
    deps.emitter
        .item_with(ItemEvent::Created, &bank_name, &replacement, Some(&reason))
        .await;
    deps.emitter
        .item_with(ItemEvent::Superseded, &bank_name, &retired, Some(&reason))
        .await;
    crate::embed_client::spawn_backfill(deps, &bank_name);
    Ok(CorrectResponse {
        corrected: true,
        memory: Some(replacement),
        superseded: Some(retired),
        skipped: None,
    })
}
