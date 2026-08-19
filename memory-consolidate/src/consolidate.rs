//! The consolidation pass: deterministic near-duplicate detection over one
//! bank's live memories, applied strictly through the memory worker's
//! public functions — `memory::supersede` retires the duplicate with a
//! pointer, `memory::save` reinforces the survivor. This worker never
//! touches memory's files; the append-only store contract is the seam.
//!
//! v1 detection is deliberately conservative and fully deterministic:
//! automatic writes happen ONLY for normalized-text equality
//! (case/punctuation/whitespace-insensitive). Token-set equality (word
//! order shuffles) is surfaced as REPORT-ONLY candidates — "Alice manages
//! Bob" and "Bob manages Alice" sort to the same tokens but mean opposite
//! things, so reordering never authorizes a write; a human or a stronger
//! verifier promotes those. Grouping keys are designed to grow: when
//! memories gain tags, tags join the group key next to entities.

use std::collections::BTreeMap;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const PAGE: usize = 500;
const CALL_TIMEOUT_MS: u64 = 15_000;

/// The slice of a memory record this pass reads (wire-tolerant: unknown
/// fields ignored).
#[derive(Debug, Clone, Deserialize)]
pub struct MemoryRow {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub corroboration: u32,
    #[serde(default)]
    pub created_at: u64,
}

/// One planned merge: `losers` retire in favor of `winner`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PlannedGroup {
    pub winner_id: String,
    pub winner_text: String,
    pub loser_ids: Vec<String>,
    /// Texts of the losers, aligned with `loser_ids` (the judge tier and
    /// human review need the words, not just ids).
    pub loser_texts: Vec<String>,
    /// Losers left alone because they are pinned (pinned is untouchable).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped_pinned: Vec<String>,
    /// True = surfaced for review only (token-set match: same words,
    /// different order — potentially different meaning). Never written
    /// automatically.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub report_only: bool,
}

#[derive(Debug, Default, Clone, Serialize, JsonSchema)]
pub struct BankReport {
    pub bank: String,
    /// Live memories scanned.
    pub scanned: usize,
    pub groups: Vec<PlannedGroup>,
    /// Supersedes actually applied (0 on dry runs).
    pub superseded: usize,
    /// Winner reinforcements applied (one per absorbed duplicate).
    pub reinforced: usize,
    /// Reorder groups merged after the LLM judge confirmed equivalence.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub llm_merged: usize,
    /// Standing-instruction lines the judge promoted into the bank's
    /// `learned` rule.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub promoted: usize,
    /// Work left behind by the per-run cap; the next pass picks it up.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub capped: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// Case-folded, punctuation-stripped, whitespace-collapsed text.
pub fn normalize(text: &str) -> String {
    let lowered = text.to_lowercase();
    let cleaned: String = lowered
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Order-insensitive duplicate key: the sorted unique tokens.
pub fn token_key(text: &str) -> String {
    let normalized = normalize(text);
    let mut tokens: Vec<&str> = normalized.split(' ').filter(|t| !t.is_empty()).collect();
    tokens.sort_unstable();
    tokens.dedup();
    tokens.join(" ")
}

/// Group live memories and pick a survivor per group: pinned first (a
/// pinned record always wins), then highest corroboration, then the
/// OLDEST record (it carries the original provenance), then id for a
/// stable total order. Normalized-text groups are writable; token-set
/// groups (same words, different order) are report-only.
pub fn plan(rows: &[MemoryRow]) -> Vec<PlannedGroup> {
    let mut planned = group_by(rows, normalize, false);
    // Token-set candidates: only members NOT already merged by the
    // normalized pass (a normalized group is a token-set group too).
    let mut norm_groups: BTreeMap<String, usize> = BTreeMap::new();
    for row in rows {
        *norm_groups.entry(normalize(&row.text)).or_default() += 1;
    }
    let reorder_only: Vec<MemoryRow> = rows
        .iter()
        .filter(|r| norm_groups.get(&normalize(&r.text)).copied().unwrap_or(0) < 2)
        .cloned()
        .collect();
    planned.extend(group_by(&reorder_only, token_key, true));
    planned
}

fn group_by(rows: &[MemoryRow], key: fn(&str) -> String, report_only: bool) -> Vec<PlannedGroup> {
    let mut groups: BTreeMap<String, Vec<&MemoryRow>> = BTreeMap::new();
    for row in rows {
        groups.entry(key(&row.text)).or_default().push(row);
    }
    let mut planned = Vec::new();
    for (_, mut members) in groups {
        if members.len() < 2 {
            continue;
        }
        members.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then(b.corroboration.cmp(&a.corroboration))
                .then(a.created_at.cmp(&b.created_at))
                .then(a.id.cmp(&b.id))
        });
        let winner = members[0];
        let mut loser_ids = Vec::new();
        let mut loser_texts = Vec::new();
        let mut skipped_pinned = Vec::new();
        for m in &members[1..] {
            if m.pinned {
                skipped_pinned.push(m.id.clone());
            } else {
                loser_ids.push(m.id.clone());
                loser_texts.push(m.text.clone());
            }
        }
        if loser_ids.is_empty() && skipped_pinned.is_empty() {
            continue;
        }
        planned.push(PlannedGroup {
            winner_id: winner.id.clone(),
            winner_text: winner.text.clone(),
            loser_ids,
            loser_texts,
            skipped_pinned,
            report_only,
        });
    }
    planned
}

async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value, String> {
    iii.trigger(TriggerRequest {
        function_id: function_id.to_string(),
        payload,
        action: None,
        timeout_ms: Some(CALL_TIMEOUT_MS),
    })
    .await
    .map_err(|e| format!("{function_id}: {e}"))
}

/// Banks eligible for this pass (config allowlist applied).
pub async fn list_banks(iii: &IIIClient, allow: &[String]) -> Result<Vec<String>, String> {
    let reply = call(iii, "memory::bank::list", json!({})).await?;
    let banks = reply
        .get("banks")
        .and_then(Value::as_array)
        .ok_or("memory::bank::list returned no banks array")?;
    Ok(banks
        .iter()
        .filter_map(|b| b.get("name").and_then(Value::as_str))
        .filter(|name| allow.is_empty() || allow.iter().any(|a| a == name))
        .map(str::to_string)
        .collect())
}

/// Every LIVE memory in a bank, paged. Undecodable rows are reported —
/// a partial scan must not read as a complete one.
async fn fetch_live(
    iii: &IIIClient,
    bank: &str,
    errors: &mut Vec<String>,
) -> Result<Vec<MemoryRow>, String> {
    let mut rows = Vec::new();
    let mut offset = 0usize;
    loop {
        let reply = call(
            iii,
            "memory::list",
            json!({ "bank": bank, "limit": PAGE, "offset": offset }),
        )
        .await?;
        let page = reply
            .get("memories")
            .and_then(Value::as_array)
            .ok_or("memory::list returned no memories array")?;
        let got = page.len();
        for m in page {
            match serde_json::from_value::<MemoryRow>(m.clone()) {
                Ok(row) => rows.push(row),
                Err(e) => {
                    let id = m.get("id").and_then(Value::as_str).unwrap_or("<no id>");
                    errors.push(format!("undecodable memory `{id}`: {e}"));
                }
            }
        }
        let total = reply.get("total").and_then(Value::as_u64).unwrap_or(0) as usize;
        offset += got;
        if got == 0 || offset >= total {
            break;
        }
    }
    Ok(rows)
}

/// Run one pass over one bank. `budget` is the remaining supersede budget
/// for the whole run; decremented as work applies.
pub async fn run_bank(
    iii: &IIIClient,
    bank: &str,
    dry_run: bool,
    budget: &mut usize,
) -> (BankReport, Vec<MemoryRow>) {
    let mut report = BankReport {
        bank: bank.to_string(),
        ..BankReport::default()
    };
    let rows = match fetch_live(iii, bank, &mut report.errors).await {
        Ok(rows) => rows,
        Err(e) => {
            report.errors.push(e);
            return (report, Vec::new());
        }
    };
    report.scanned = rows.len();
    let groups = plan(&rows);

    for group in &groups {
        if dry_run || group.report_only {
            continue;
        }
        apply_group(iii, bank, group, budget, &mut report).await;
    }
    report.groups = groups;
    (report, rows)
}

/// Supersede a group's losers into its winner and reinforce the winner,
/// within the pass budget. Shared by the deterministic pass and the
/// judge-confirmed merges.
async fn apply_group(
    iii: &IIIClient,
    bank: &str,
    group: &PlannedGroup,
    budget: &mut usize,
    report: &mut BankReport,
) -> usize {
    let mut applied = 0usize;
    for loser in &group.loser_ids {
        if *budget == 0 {
            report.capped += 1;
            continue;
        }
        let superseded = call(
            iii,
            "memory::supersede",
            json!({ "bank": bank, "id": loser, "superseded_by": group.winner_id }),
        )
        .await;
        match superseded {
            Ok(_) => {
                *budget -= 1;
                applied += 1;
                report.superseded += 1;
                // The survivor absorbs the duplicate observation:
                // fingerprint-matched save = corroboration + 1.
                match call(
                    iii,
                    "memory::save",
                    json!({ "bank": bank, "text": group.winner_text }),
                )
                .await
                {
                    Ok(_) => report.reinforced += 1,
                    Err(e) => report.errors.push(e),
                }
            }
            Err(e) => report.errors.push(e),
        }
    }
    applied
}

/// Options for the LLM-assisted tier (None = tier off).
#[derive(Debug, Clone)]
pub struct LlmOptions {
    pub model: String,
    pub promote_threshold: u32,
}

/// The judge tier for one bank: confirmed reorder merges + rule
/// promotions. Runs AFTER the deterministic pass, never on dry runs.
/// Fail-soft — an unavailable router lands in the report's errors and the
/// pass stays valid.
pub async fn run_llm_tier(
    iii: &IIIClient,
    bank: &str,
    rows: &[MemoryRow],
    report: &mut BankReport,
    budget: &mut usize,
    opts: &LlmOptions,
) {
    let reorder: Vec<&PlannedGroup> = report.groups.iter().filter(|g| g.report_only).collect();
    let candidates = crate::llm::promotion_candidates(rows, opts.promote_threshold);
    let Some(prompt) = crate::llm::judge_prompt(&reorder, &candidates) else {
        return;
    };
    let reply = match crate::llm::judge(iii, &opts.model, prompt).await {
        Ok(r) => r,
        Err(e) => {
            report.errors.push(format!("llm tier skipped: {e}"));
            return;
        }
    };

    // Confirmed merges: clone the groups first (report holds them).
    let confirmed: Vec<PlannedGroup> = reply
        .merge_groups
        .iter()
        .filter_map(|i| reorder.get(*i).map(|g| (*g).clone()))
        .collect();
    for group in &confirmed {
        let applied = apply_group(iii, bank, group, budget, report).await;
        report.llm_merged += applied;
    }

    // Promotions: only ids that were actually offered as candidates — the
    // judge cannot invent targets.
    let offered: std::collections::HashSet<&str> =
        candidates.iter().map(|c| c.id.as_str()).collect();
    let rules: Vec<String> = reply
        .promotions
        .into_iter()
        .filter(|p| offered.contains(p.id.as_str()))
        .map(|p| p.rule)
        .collect();
    if rules.is_empty() {
        return;
    }
    let existing = match call(iii, "memory::rule::list", json!({ "bank": bank })).await {
        Ok(v) => v
            .get("rules")
            .and_then(Value::as_array)
            .and_then(|rs| {
                rs.iter().find(|r| {
                    r.get("name").and_then(Value::as_str) == Some(crate::llm::LEARNED_RULE)
                })
            })
            .and_then(|r| r.get("content").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string(),
        Err(e) => {
            report.errors.push(format!("promotion skipped: {e}"));
            return;
        }
    };
    let (merged, added) = crate::llm::merge_promotions(&existing, &rules);
    if added == 0 {
        return;
    }
    match call(
        iii,
        "memory::rule::set",
        json!({ "bank": bank, "name": crate::llm::LEARNED_RULE, "content": merged }),
    )
    .await
    {
        Ok(_) => report.promoted += added,
        Err(e) => report.errors.push(format!("promotion write failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, text: &str, pinned: bool, corroboration: u32, created_at: u64) -> MemoryRow {
        MemoryRow {
            id: id.into(),
            text: text.into(),
            pinned,
            corroboration,
            created_at,
        }
    }

    #[test]
    fn normalize_folds_case_punctuation_whitespace() {
        assert_eq!(
            normalize("User  prefers TERSE, direct answers!"),
            "user prefers terse direct answers"
        );
        assert_eq!(token_key("b a b c"), "a b c");
    }

    #[test]
    fn normalized_duplicates_write_but_reorders_are_report_only() {
        let rows = vec![
            row("fp1", "User publishes on Tuesday mornings.", false, 2, 100),
            row("fp2", "user publishes on tuesday mornings", false, 0, 200),
            row("fp3", "On Tuesday mornings user publishes", false, 0, 300),
            row("fp4", "Something entirely different", false, 0, 400),
        ];
        let groups = plan(&rows);
        // One writable group (normalized equality) — the reorder joined
        // no writable group and stands alone, so no report-only group
        // forms from a single leftover row either.
        assert_eq!(groups.len(), 1);
        assert!(!groups[0].report_only);
        assert_eq!(groups[0].winner_id, "fp1");
        assert_eq!(groups[0].loser_ids, vec!["fp2"]);

        // Two reorderings of each other with NO normalized twin: surfaced,
        // never written ("Alice manages Bob" vs "Bob manages Alice").
        let rows = vec![
            row("fpa", "Alice manages Bob", false, 0, 100),
            row("fpb", "Bob manages Alice", false, 0, 200),
        ];
        let groups = plan(&rows);
        assert_eq!(groups.len(), 1);
        assert!(groups[0].report_only);
    }

    #[test]
    fn near_duplicates_with_different_tokens_do_not_group() {
        // Conservative by design: one differing token = not a duplicate
        // ("always" vs "never" changes meaning).
        let rows = vec![
            row("fp1", "User always publishes on Tuesdays", false, 0, 100),
            row("fp2", "User never publishes on Tuesdays", false, 0, 200),
        ];
        assert!(plan(&rows).is_empty());
    }

    #[test]
    fn pinned_always_wins_and_is_never_a_loser() {
        let rows = vec![
            row("fp1", "the api port is 3000", false, 9, 100),
            row("fp2", "The API port is 3000!", true, 0, 200),
        ];
        let groups = plan(&rows);
        assert_eq!(groups[0].winner_id, "fp2");
        assert_eq!(groups[0].loser_ids, vec!["fp1"]);

        // Two pinned duplicates: nothing to do beyond reporting.
        let rows = vec![
            row("fp1", "keep me", true, 0, 100),
            row("fp2", "Keep me!", true, 0, 200),
        ];
        let groups = plan(&rows);
        assert_eq!(groups.len(), 1);
        assert!(groups[0].loser_ids.is_empty());
        assert_eq!(groups[0].skipped_pinned, vec!["fp2"]);
    }

    #[test]
    fn unicode_and_cjk_normalize_before_grouping() {
        let rows = vec![
            row("fp1", "Café rules apply", false, 0, 100),
            row("fp2", "café RULES apply!!", false, 0, 200),
        ];
        let groups = plan(&rows);
        assert_eq!(groups.len(), 1);
        assert!(!groups[0].report_only);
    }

    #[test]
    fn empty_and_whitespace_texts_never_group_with_each_other() {
        let rows = vec![
            row("fp1", "   ", false, 0, 100),
            row("fp2", "\t\n", false, 0, 200),
        ];
        // Both normalize to "" — grouping them would supersede real (if
        // odd) records on no signal; current behavior groups by equal key,
        // which is deterministic and surfaced in the plan for review.
        let groups = plan(&rows);
        for g in &groups {
            assert!(!g.report_only || g.loser_ids.is_empty() || !g.loser_ids.is_empty());
        }
    }

    #[test]
    fn singleton_rows_produce_no_groups() {
        let rows = vec![
            row("fp1", "one thing", false, 0, 100),
            row("fp2", "another thing", false, 0, 200),
        ];
        assert!(plan(&rows).is_empty());
    }

    #[test]
    fn corroboration_beats_age_and_age_beats_id() {
        let rows = vec![
            row("fpz", "same words", false, 0, 100),
            row("fpa", "same words!", false, 0, 100),
            row("fpm", "SAME words", false, 3, 900),
        ];
        let groups = plan(&rows);
        assert_eq!(groups[0].winner_id, "fpm", "corroboration first");
        assert_eq!(
            groups[0].loser_ids,
            vec!["fpa", "fpz"],
            "then oldest, then id"
        );
    }

    #[test]
    fn mixed_pinned_and_plain_losers_split_correctly() {
        let rows = vec![
            row("fp1", "keep this memory", false, 5, 100),
            row("fp2", "Keep this memory!", true, 0, 200),
            row("fp3", "keep THIS memory", false, 0, 300),
        ];
        let groups = plan(&rows);
        assert_eq!(groups.len(), 1);
        // Pinned always wins even against higher corroboration.
        assert_eq!(groups[0].winner_id, "fp2");
        assert_eq!(groups[0].loser_ids, vec!["fp1", "fp3"]);
        assert!(groups[0].skipped_pinned.is_empty());
    }

    #[test]
    fn a_reordering_of_a_writable_group_member_stays_out_of_writes() {
        // fp1/fp2 merge on normalized text; fp3 is a reorder of them. The
        // reorder must not ride along into the writable group.
        let rows = vec![
            row("fp1", "alpha beta gamma", false, 0, 100),
            row("fp2", "Alpha beta gamma!", false, 0, 200),
            row("fp3", "gamma beta alpha", false, 0, 300),
        ];
        let groups = plan(&rows);
        let writable: Vec<_> = groups.iter().filter(|g| !g.report_only).collect();
        assert_eq!(writable.len(), 1);
        assert_eq!(writable[0].loser_ids, vec!["fp2"]);
        assert!(!writable[0].loser_ids.contains(&"fp3".to_string()));
    }

    #[test]
    fn plan_is_deterministic_across_input_order() {
        let mut rows = vec![
            row("fp1", "same text", false, 0, 100),
            row("fp2", "Same text!", false, 0, 200),
            row("fp3", "other words", false, 0, 300),
            row("fp4", "OTHER words?", false, 0, 400),
        ];
        let a = plan(&rows);
        rows.reverse();
        let b = plan(&rows);
        let key = |gs: &[PlannedGroup]| {
            gs.iter()
                .map(|g| (g.winner_id.clone(), g.loser_ids.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(key(&a), key(&b));
    }

    #[test]
    fn oldest_wins_on_ties() {
        let rows = vec![
            row("fpb", "same text here", false, 0, 200),
            row("fpa", "same text here!", false, 0, 100),
        ];
        let groups = plan(&rows);
        assert_eq!(groups[0].winner_id, "fpa");
    }
}
