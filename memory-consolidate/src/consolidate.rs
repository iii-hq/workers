//! The consolidation pass: deterministic near-duplicate detection over one
//! bank's live memories, applied strictly through the memory worker's
//! public functions — `memory::supersede` retires the duplicate with a
//! pointer, `memory::save` reinforces the survivor. This worker never
//! touches memory's files; the append-only store contract is the seam.
//!
//! v1 detection is deliberately conservative and fully deterministic: two
//! memories are duplicates only when their normalized text matches
//! (case/punctuation/whitespace-insensitive) or their token SETS are equal
//! (word order shuffles). Semantic near-duplicate merging is a later,
//! LLM-assisted tier. Grouping keys are designed to grow: when memories
//! gain tags, tags join the group key next to entities.

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
    /// Losers left alone because they are pinned (pinned is untouchable).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped_pinned: Vec<String>,
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

/// Group live memories by duplicate key and pick a survivor per group:
/// pinned first (a pinned record always wins), then highest corroboration,
/// then the OLDEST record (it carries the original provenance), then id
/// for a stable total order.
pub fn plan(rows: &[MemoryRow]) -> Vec<PlannedGroup> {
    let mut groups: BTreeMap<String, Vec<&MemoryRow>> = BTreeMap::new();
    for row in rows {
        groups.entry(token_key(&row.text)).or_default().push(row);
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
        let mut skipped_pinned = Vec::new();
        for m in &members[1..] {
            if m.pinned {
                skipped_pinned.push(m.id.clone());
            } else {
                loser_ids.push(m.id.clone());
            }
        }
        if loser_ids.is_empty() && skipped_pinned.is_empty() {
            continue;
        }
        planned.push(PlannedGroup {
            winner_id: winner.id.clone(),
            winner_text: winner.text.clone(),
            loser_ids,
            skipped_pinned,
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

/// Every LIVE memory in a bank, paged.
async fn fetch_live(iii: &IIIClient, bank: &str) -> Result<Vec<MemoryRow>, String> {
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
            if let Ok(row) = serde_json::from_value::<MemoryRow>(m.clone()) {
                rows.push(row);
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
) -> BankReport {
    let mut report = BankReport {
        bank: bank.to_string(),
        ..BankReport::default()
    };
    let rows = match fetch_live(iii, bank).await {
        Ok(rows) => rows,
        Err(e) => {
            report.errors.push(e);
            return report;
        }
    };
    report.scanned = rows.len();
    let groups = plan(&rows);

    for group in &groups {
        for loser in &group.loser_ids {
            if dry_run {
                continue;
            }
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
    }
    report.groups = groups;
    report
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
    fn exact_and_word_order_duplicates_group() {
        let rows = vec![
            row("fp1", "User publishes on Tuesday mornings.", false, 2, 100),
            row("fp2", "user publishes on tuesday mornings", false, 0, 200),
            row("fp3", "On Tuesday mornings user publishes", false, 0, 300),
            row("fp4", "Something entirely different", false, 0, 400),
        ];
        let groups = plan(&rows);
        assert_eq!(groups.len(), 1);
        // Highest corroboration wins; the rest retire.
        assert_eq!(groups[0].winner_id, "fp1");
        assert_eq!(groups[0].loser_ids, vec!["fp2", "fp3"]);
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
    fn oldest_wins_on_ties() {
        let rows = vec![
            row("fpb", "same text here", false, 0, 200),
            row("fpa", "same text here!", false, 0, 100),
        ];
        let groups = plan(&rows);
        assert_eq!(groups[0].winner_id, "fpa");
    }
}
