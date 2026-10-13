//! Turn-record compaction (MOT-5166): rewrites finished records still holding
//! their frozen prompt texts inline (written before `harness_prompt` refs) and
//! deletes the prompt bodies no record references. Runs in the pending sweep
//! and once after boot, over the records of a full read it is handed.

use std::collections::BTreeSet;

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::state::{prompt_digest, TurnListing};
use crate::types::turn::TurnRecord;

/// How long an unreferenced body is kept: a send stores its body before it
/// writes the record that references it.
pub(crate) const PROMPT_BODY_GRACE_MS: i64 = 86_400_000;

#[derive(Debug, Default)]
pub struct CompactReport {
    /// Finished records rewritten with refs (and slimmed).
    pub converted: u64,
    /// Prompt bodies deleted.
    pub prompts_collected: u64,
}

/// Convert, then collect. Never fails: what fails is warned and left for the
/// next pass.
pub async fn compact(deps: &Deps, listing: &TurnListing, now: i64) -> CompactReport {
    let timeout = deps.cfg().await.session_timeout_ms;
    let records = &listing.records;
    let mut report = CompactReport::default();
    for record in records.iter().filter(|r| holds_inline_prompt(r)) {
        match convert(deps, record, timeout).await {
            Ok(true) => report.converted += 1,
            Ok(false) => {}
            Err(e) => tracing::warn!(
                session_id = %record.session_id,
                error = %e,
                "could not convert a turn record to prompt refs"
            ),
        }
    }
    // An inline text counts by its digest: converted, its record references it.
    let mut referenced: BTreeSet<String> = records
        .iter()
        .flat_map(prompt_texts)
        .filter_map(|(text, slot)| slot.clone().or_else(|| text.as_deref().map(prompt_digest)))
        .collect();
    referenced.extend(listing.unparsed_refs.iter().cloned());
    let cutoff = now - PROMPT_BODY_GRACE_MS;
    match crate::state::collect_prompt_bodies(&deps.iii, &referenced, cutoff, timeout).await {
        Ok(n) => report.prompts_collected = n,
        Err(e) => tracing::warn!(error = %e, "prompt body collection failed"),
    }
    report
}

/// Each frozen text with its `harness_prompt` ref.
fn prompt_texts(record: &TurnRecord) -> Vec<(&Option<String>, &Option<String>)> {
    let options = &record.options;
    let mut texts = vec![(&options.system_prompt, &options.system_prompt_ref)];
    if let Some(context) = &options.skill_context {
        texts.push((&context.baseline, &context.baseline_ref));
    }
    texts
}

/// A finished record with a text inline and no ref (hydrated records carry
/// both, listed ones only the ref).
fn holds_inline_prompt(record: &TurnRecord) -> bool {
    record.status.is_terminal()
        && prompt_texts(record)
            .into_iter()
            .any(|(text, slot)| text.is_some() && slot.is_none())
}

/// Rewrite one listed record, under the session's guards and only if it is
/// still the record that was listed. `put_turn` moves the texts to refs.
async fn convert(deps: &Deps, listed: &TurnRecord, timeout: u64) -> Result<bool, HarnessError> {
    let session_id = &listed.session_id;
    // A step enters `inflight` before it takes `turn_activity`, and holds that
    // for its whole run: skip it rather than wait it out (the boot pass runs
    // in the redrive loop). Checked again under the guards.
    if deps.inflight.contains(session_id) {
        return Ok(false);
    }
    let _activity = deps.turn_activity.guard(session_id).await;
    let _lock = deps.locks.guard(session_id).await;
    if deps.inflight.contains(session_id) {
        return Ok(false);
    }
    let Some(mut record) = crate::state::get_turn(&deps.iii, session_id, timeout).await? else {
        return Ok(false);
    };
    if record.turn_id != listed.turn_id
        || record.updated_at != listed.updated_at
        || !holds_inline_prompt(&record)
    {
        return Ok(false);
    }
    record.slim_finished();
    crate::state::put_turn(&deps.iii, &record, timeout).await?;
    Ok(true)
}
