//! The LLM-assisted tier: judge what determinism cannot.
//!
//! Two candidate kinds feed ONE `router::complete` call per pass:
//! - REORDER groups (same token set, different order) — the deterministic
//!   pass surfaces them report-only because word order can flip meaning;
//!   the model judges semantic equivalence, and only a confirmed group is
//!   merged through the same supersede + reinforce seam.
//! - PROMOTION candidates (live memories re-observed at least
//!   `promote_corroboration_threshold` times) — the model decides whether
//!   one is a standing instruction and phrases the one-line rule; accepted
//!   lines append to the bank's auto-managed `learned` rule,
//!   fingerprint-deduped, never touching hand-authored rules. The judge sees
//!   the learned lines already in force, and ids promoted or covered by them
//!   are remembered in state, so a candidate is not judged again once its
//!   line is in the rule.
//!
//! Fail-soft: no router, a malformed reply, or `llm_assist_enabled: false`
//! means this tier simply does not run — the deterministic pass already
//! completed. The tier never deletes: merges supersede (pointer kept) and
//! promotions only append, each write guarded by the rule as read right
//! before it (`expected`).

use std::collections::BTreeSet;

use iii_sdk::protocol::CallRequest;
use iii_sdk::IIIClient;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::consolidate::{MemoryRow, PlannedGroup};

const JUDGE_SYSTEM: &str = "You judge memory-bank hygiene candidates. Reply ONLY with a JSON \
object (no prose, no code fences):\n\
{\"merge_groups\": [int], \"promotions\": [{\"id\": string, \"rule\": string}], \
\"covered\": [string]}\n\
Rules:\n\
- merge_groups: indices of REORDER groups whose texts state the SAME fact with the same \
meaning. Word-order changes that swap roles (\"Alice manages Bob\" vs \"Bob manages Alice\") \
are NOT the same — when unsure, leave the group out.\n\
- promotions: candidates that are standing instructions about how an assistant should behave \
(style, format, workflow). Phrase `rule` as ONE imperative line under 200 characters. Facts \
about people or the world are never promoted — omit them.\n\
- learned_lines are the rules already in force. A candidate that one of them already states, \
in the same or other words, is never promoted: list its id in covered instead.\n\
- Empty lists are the normal answer.";

const LLM_TIMEOUT_MS: u64 = 60_000;
/// Name of the auto-managed rule promotions append to (the same file the
/// memory worker's extraction writes).
pub(crate) const LEARNED_RULE: &str = "learned";
const LEARNED_HEADER: &str = "# Learned\nStanding instructions captured automatically from \
conversations. Edit or delete lines freely — extraction appends new ones and rewrites a line \
the user corrects.\n";

#[derive(Debug, Default, Deserialize)]
pub struct JudgeReply {
    #[serde(default)]
    pub merge_groups: Vec<usize>,
    #[serde(default)]
    pub promotions: Vec<Promotion>,
    /// Candidate ids a learned line already states (never promoted). Kept
    /// as a `Value` so a null or misshapen list cannot void the whole reply;
    /// the caller reads only its string items.
    #[serde(default)]
    pub covered: Value,
}

#[derive(Debug, Deserialize)]
pub struct Promotion {
    pub id: String,
    pub rule: String,
}

/// Live, unpinned-or-pinned memories reinforced often enough to be worth a
/// promotion judgment. Pinned records are eligible: promotion copies a
/// line into the learned rule and never modifies the memory itself. Ids in
/// `promoted` (the ledger) are dropped before the cap, so they never hold
/// the top slots.
// shortcut: the ledger holds promoted and covered ids only, a candidate the judge rejects is judged again each pass; record judged ids once a reply can be told from a malformed one.
pub fn promotion_candidates<'a>(
    rows: &'a [MemoryRow],
    threshold: u32,
    promoted: &BTreeSet<String>,
) -> Vec<&'a MemoryRow> {
    if threshold == 0 {
        return Vec::new();
    }
    let mut out: Vec<&MemoryRow> = rows
        .iter()
        .filter(|r| r.corroboration >= threshold && !promoted.contains(&r.id))
        .collect();
    out.sort_by(|a, b| b.corroboration.cmp(&a.corroboration).then(a.id.cmp(&b.id)));
    out.truncate(20);
    out
}

/// Render the judge prompt for one bank's candidates. `None` when there is
/// nothing to judge (skip the call entirely). The `learned` rule's lines go
/// along only with promotion candidates, so a reorder-only prompt is
/// unchanged.
pub fn judge_prompt(
    reorder_groups: &[&PlannedGroup],
    promotions: &[&MemoryRow],
    learned: &str,
) -> Option<String> {
    if reorder_groups.is_empty() && promotions.is_empty() {
        return None;
    }
    let groups: Vec<Value> = reorder_groups
        .iter()
        .enumerate()
        .map(|(i, g)| {
            json!({
                "index": i,
                "texts": std::iter::once(g.winner_text.clone())
                    .chain(g.loser_texts.iter().cloned())
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    let candidates: Vec<Value> = promotions
        .iter()
        .map(|m| json!({ "id": m.id, "text": m.text, "times_observed": m.corroboration + 1 }))
        .collect();
    let mut prompt = json!({ "reorder_groups": groups, "promotion_candidates": candidates });
    if !promotions.is_empty() {
        let lines: Vec<&str> = learned
            .lines()
            .filter_map(|l| l.strip_prefix("- "))
            .collect();
        prompt["learned_lines"] = json!(lines);
    }
    Some(prompt.to_string())
}

/// Tolerant reply parse: fences and prose stripped the same way the memory
/// worker's extraction parser does; a reply with neither field is empty.
pub fn parse_reply(raw: &str) -> JudgeReply {
    let trimmed = raw.trim();
    let start = trimmed.find('{');
    let end = trimmed.rfind('}');
    let json_str = match (start, end) {
        (Some(s), Some(e)) if e > s => &trimmed[s..=e],
        _ => return JudgeReply::default(),
    };
    serde_json::from_str(json_str).unwrap_or_default()
}

/// Append accepted rule lines to the learned rule, fingerprint-deduped
/// against existing lines AND within the batch. Returns the merged
/// content, how many lines were added, and the ids whose line is now in the
/// rule (added or already there; a line rejected for length is in neither).
pub fn merge_promotions(existing: &str, promotions: &[Promotion]) -> (String, usize, Vec<String>) {
    let mut seen: std::collections::HashSet<String> = existing
        .lines()
        .filter_map(|l| l.strip_prefix("- "))
        .map(crate::consolidate::normalize)
        .collect();
    let mut out = if existing.trim().is_empty() {
        LEARNED_HEADER.to_string()
    } else {
        existing.trim_end().to_string() + "\n"
    };
    let mut added = 0usize;
    let mut landed = Vec::new();
    for promotion in promotions {
        let line = promotion.rule.trim();
        if line.len() < 3 || line.len() > 300 {
            continue;
        }
        landed.push(promotion.id.clone());
        if !seen.insert(crate::consolidate::normalize(line)) {
            continue;
        }
        out.push_str("- ");
        out.push_str(line);
        out.push('\n');
        added += 1;
    }
    (out, added, landed)
}

/// The extraction model: the configured pin, else the first catalog entry.
pub(crate) async fn resolve_model(iii: &IIIClient, configured: &str) -> Result<String, String> {
    if !configured.is_empty() {
        return Ok(configured.to_string());
    }
    let reply = iii
        .call(CallRequest {
            function_id: "router::models::list".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .map_err(|e| format!("router::models::list: {e}"))?;
    reply
        .get("models")
        .and_then(Value::as_array)
        .and_then(|m| m.first())
        .and_then(|m| m.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .ok_or_else(|| "no models in the router catalog".to_string())
}

/// One judge call for one bank, with the router's `usage` for the call.
/// Fail-soft: any error returns the reason for the report; the
/// deterministic pass already ran.
pub async fn judge(
    iii: &IIIClient,
    configured_model: &str,
    prompt: String,
) -> Result<(JudgeReply, Value), String> {
    let reply = complete(iii, configured_model, JUDGE_SYSTEM, prompt, 1_024).await?;
    Ok((parse_reply(&reply_text(&reply)), reply["usage"].clone()))
}

/// One `router::complete` call with a static system prompt (cache-friendly)
/// and one user message; the raw reply, so callers can classify provider
/// errors (`message.stop_reason == "error"`) before reading text.
pub(crate) async fn complete(
    iii: &IIIClient,
    configured_model: &str,
    system_prompt: &str,
    prompt: String,
    max_output_tokens: u32,
) -> Result<Value, String> {
    let model = resolve_model(iii, configured_model).await?;
    iii.call(CallRequest {
        function_id: "router::complete".into(),
        payload: json!({
            "model": model,
            "system_prompt": system_prompt,
            "messages": [{
                "role": "user",
                "content": [{ "type": "text", "text": prompt }],
                "timestamp": now_ms() as i64,
            }],
            "max_output_tokens": max_output_tokens,
        }),
        action: None,
        timeout_ms: Some(LLM_TIMEOUT_MS),
    })
    .await
    .map_err(|e| format!("router::complete: {e}"))
}

/// The assistant text of a `router::complete` reply.
pub(crate) fn reply_text(reply: &Value) -> String {
    reply
        .pointer("/message/content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, text: &str, corroboration: u32) -> MemoryRow {
        MemoryRow {
            id: id.into(),
            text: text.into(),
            pinned: false,
            corroboration,
            created_at: 1,
            updated_at: 1,
            stated_at: None,
            snapshot: Default::default(),
        }
    }

    #[test]
    fn promotion_candidates_filter_sort_and_cap() {
        let mut rows: Vec<MemoryRow> = (0..30)
            .map(|i| row(&format!("fp{i:02}"), &format!("text {i}"), i as u32))
            .collect();
        rows.reverse();
        let none = BTreeSet::new();
        let out = promotion_candidates(&rows, 4, &none);
        assert_eq!(out.len(), 20, "capped");
        assert_eq!(out[0].corroboration, 29, "most corroborated first");
        assert!(out.iter().all(|r| r.corroboration >= 4));
        assert!(
            promotion_candidates(&rows, 0, &none).is_empty(),
            "threshold 0 disables"
        );
        // 26 rows meet the threshold; ledger ids leave before the cap.
        let ledger: BTreeSet<String> = (8..30).map(|i| format!("fp{i:02}")).collect();
        let out = promotion_candidates(&rows, 4, &ledger);
        assert_eq!(
            out.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["fp07", "fp06", "fp05", "fp04"]
        );
    }

    #[test]
    fn judge_prompt_skips_empty_and_numbers_groups() {
        assert!(judge_prompt(&[], &[], "- Be brief.\n").is_none());
        let g = PlannedGroup {
            winner_id: "fpw".into(),
            winner_text: "alpha beta".into(),
            loser_ids: vec!["fpl".into()],
            loser_texts: vec!["beta alpha".into()],
            skipped_pinned: vec![],
            report_only: true,
        };
        let groups = [&g];
        let learned = "# Learned\nIntro.\n- Be brief.\n- Cite sources.\n";
        let prompt = judge_prompt(&groups, &[], learned).unwrap();
        let v: serde_json::Value = serde_json::from_str(&prompt).unwrap();
        assert_eq!(v["reorder_groups"][0]["index"], 0);
        assert_eq!(v["reorder_groups"][0]["texts"][0], "alpha beta");
        assert_eq!(v["reorder_groups"][0]["texts"][1], "beta alpha");
        assert!(v.get("learned_lines").is_none(), "reorder-only: no rule");
        let candidate = row("fp1", "User wants short replies", 5);
        let prompt = judge_prompt(&groups, &[&candidate], learned).unwrap();
        let v: serde_json::Value = serde_json::from_str(&prompt).unwrap();
        assert_eq!(
            v["learned_lines"],
            serde_json::json!(["Be brief.", "Cite sources."])
        );
        assert_eq!(v["promotion_candidates"][0]["id"], "fp1");
    }

    #[test]
    fn reply_parse_reads_covered_ids() {
        let r = parse_reply("{\"covered\": [\"fp1\"]}");
        assert_eq!(r.covered, serde_json::json!(["fp1"]));
        assert!(r.promotions.is_empty());
        assert!(parse_reply("{}").covered.is_null());
        // A bad `covered` must not throw away the rest of the reply.
        for bad in ["null", "[{\"id\": \"fp1\"}]", "\"fp1\""] {
            let r = parse_reply(&format!(
                "{{\"merge_groups\": [0], \"promotions\": [{{\"id\": \"a\", \"rule\": \"Be brief.\"}}], \"covered\": {bad}}}"
            ));
            assert_eq!(r.merge_groups, [0], "{bad}");
            assert_eq!(r.promotions.len(), 1, "{bad}");
        }
    }

    #[test]
    fn reply_parse_tolerates_fences_and_garbage() {
        let r = parse_reply("Sure!\n```json\n{\"merge_groups\": [0], \"promotions\": []}\n```");
        assert_eq!(r.merge_groups, vec![0]);
        assert!(parse_reply("no json here").merge_groups.is_empty());
        assert!(parse_reply("{}").promotions.is_empty());
        let r = parse_reply("{\"promotions\": [{\"id\": \"fp1\", \"rule\": \"Always X.\"}]}");
        assert_eq!(r.promotions.len(), 1);
        assert_eq!(r.promotions[0].id, "fp1");
    }

    fn promotion(id: &str, rule: &str) -> Promotion {
        Promotion {
            id: id.into(),
            rule: rule.into(),
        }
    }

    #[test]
    fn merge_promotions_dedups_against_existing_and_batch() {
        let (first, added, landed) = merge_promotions(
            "",
            &[
                promotion("fp1", "Never use em-dashes."),
                promotion("fp2", "never USE em-dashes!"),
            ],
        );
        assert_eq!(added, 1);
        assert_eq!(landed, ["fp1", "fp2"], "a batch duplicate is in the rule");
        assert!(first.starts_with("# Learned\n"));
        let (second, added, landed) =
            merge_promotions(&first, &[promotion("fp3", "Never use em-dashes.")]);
        assert_eq!(added, 0);
        assert_eq!(landed, ["fp3"], "already present counts as landed");
        assert_eq!(second.trim_end(), first.trim_end());
        let (_, added, landed) = merge_promotions(&first, &[promotion("fp4", "x")]);
        assert_eq!(added, 0, "too-short lines dropped");
        assert!(landed.is_empty(), "a rejected line is not landed");
    }
}
