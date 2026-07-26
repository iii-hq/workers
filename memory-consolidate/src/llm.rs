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
//!   fingerprint-deduped, never touching hand-authored rules.
//!
//! Fail-soft: no router, a malformed reply, or `llm_assist_enabled: false`
//! means this tier simply does not run — the deterministic pass already
//! completed. The tier never deletes: merges supersede (pointer kept) and
//! promotions only append.

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::consolidate::{MemoryRow, PlannedGroup};

const JUDGE_SYSTEM: &str = "You judge memory-bank hygiene candidates. Reply ONLY with a JSON \
object (no prose, no code fences):\n\
{\"merge_groups\": [int], \"promotions\": [{\"id\": string, \"rule\": string}]}\n\
Rules:\n\
- merge_groups: indices of REORDER groups whose texts state the SAME fact with the same \
meaning. Word-order changes that swap roles (\"Alice manages Bob\" vs \"Bob manages Alice\") \
are NOT the same — when unsure, leave the group out.\n\
- promotions: candidates that are standing instructions about how an assistant should behave \
(style, format, workflow). Phrase `rule` as ONE imperative line under 200 characters. Facts \
about people or the world are never promoted — omit them.\n\
- Empty lists are the normal answer.";

const LLM_TIMEOUT_MS: u64 = 60_000;
/// Name of the auto-managed rule promotions append to (the same file the
/// memory worker's extraction writes).
pub(crate) const LEARNED_RULE: &str = "learned";
const LEARNED_HEADER: &str = "# Learned\nStanding instructions captured automatically from \
conversations. Edit or delete lines freely — extraction only ever appends new ones.\n";

#[derive(Debug, Default, Deserialize)]
pub struct JudgeReply {
    #[serde(default)]
    pub merge_groups: Vec<usize>,
    #[serde(default)]
    pub promotions: Vec<Promotion>,
}

#[derive(Debug, Deserialize)]
pub struct Promotion {
    pub id: String,
    pub rule: String,
}

/// Live, unpinned-or-pinned memories reinforced often enough to be worth a
/// promotion judgment. Pinned records are eligible: promotion copies a
/// line into the learned rule and never modifies the memory itself.
pub fn promotion_candidates(rows: &[MemoryRow], threshold: u32) -> Vec<&MemoryRow> {
    if threshold == 0 {
        return Vec::new();
    }
    let mut out: Vec<&MemoryRow> = rows
        .iter()
        .filter(|r| r.corroboration >= threshold)
        .collect();
    out.sort_by(|a, b| b.corroboration.cmp(&a.corroboration).then(a.id.cmp(&b.id)));
    out.truncate(20);
    out
}

/// Render the judge prompt for one bank's candidates. `None` when there is
/// nothing to judge (skip the call entirely).
pub fn judge_prompt(reorder_groups: &[&PlannedGroup], promotions: &[&MemoryRow]) -> Option<String> {
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
    Some(json!({ "reorder_groups": groups, "promotion_candidates": candidates }).to_string())
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
/// content and how many lines were added; `None` content = nothing new.
pub fn merge_promotions(existing: &str, rules: &[String]) -> (String, usize) {
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
    for rule in rules {
        let line = rule.trim();
        if line.len() < 3 || line.len() > 300 {
            continue;
        }
        if !seen.insert(crate::consolidate::normalize(line)) {
            continue;
        }
        out.push_str("- ");
        out.push_str(line);
        out.push('\n');
        added += 1;
    }
    (out, added)
}

/// The extraction model: the configured pin, else the first catalog entry.
async fn resolve_model(iii: &IIIClient, configured: &str) -> Result<String, String> {
    if !configured.is_empty() {
        return Ok(configured.to_string());
    }
    let request = TriggerRequest {
        function_id: "router::models::list".into(),
        payload: json!({}),
        action: None,
        timeout_ms: Some(5_000),
    };
    let reply = match iii.namespace() {
        Some(ns) => iii.trigger(request.namespace(ns)).await,
        None => iii.trigger(request).await,
    }
    .map_err(|e| format!("router::models::list: {e}"))?;
    reply
        .get("models")
        .and_then(Value::as_array)
        .and_then(|m| m.first())
        .and_then(|m| m.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .ok_or_else(|| "no models in the router catalog".to_string())
}

/// One judge call for one bank. Fail-soft: any error returns the reason
/// for the report; the deterministic pass already ran.
pub async fn judge(
    iii: &IIIClient,
    configured_model: &str,
    prompt: String,
) -> Result<JudgeReply, String> {
    let model = resolve_model(iii, configured_model).await?;
    let request = TriggerRequest {
        function_id: "router::complete".into(),
        payload: json!({
            "model": model,
            "system_prompt": JUDGE_SYSTEM,
            "messages": [{
                "role": "user",
                "content": [{ "type": "text", "text": prompt }],
                "timestamp": now_ms() as i64,
            }],
            "max_output_tokens": 1_024,
        }),
        action: None,
        timeout_ms: Some(LLM_TIMEOUT_MS),
    };
    let reply = match iii.namespace() {
        Some(ns) => iii.trigger(request.namespace(ns)).await,
        None => iii.trigger(request).await,
    }
    .map_err(|e| format!("router::complete: {e}"))?;
    let text = reply
        .pointer("/message/content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    Ok(parse_reply(&text))
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
        }
    }

    #[test]
    fn promotion_candidates_filter_sort_and_cap() {
        let mut rows: Vec<MemoryRow> = (0..30)
            .map(|i| row(&format!("fp{i:02}"), &format!("text {i}"), i as u32))
            .collect();
        rows.reverse();
        let out = promotion_candidates(&rows, 4);
        assert_eq!(out.len(), 20, "capped");
        assert_eq!(out[0].corroboration, 29, "most corroborated first");
        assert!(out.iter().all(|r| r.corroboration >= 4));
        assert!(
            promotion_candidates(&rows, 0).is_empty(),
            "threshold 0 disables"
        );
    }

    #[test]
    fn judge_prompt_skips_empty_and_numbers_groups() {
        assert!(judge_prompt(&[], &[]).is_none());
        let g = PlannedGroup {
            winner_id: "fpw".into(),
            winner_text: "alpha beta".into(),
            loser_ids: vec!["fpl".into()],
            loser_texts: vec!["beta alpha".into()],
            skipped_pinned: vec![],
            report_only: true,
        };
        let groups = [&g];
        let prompt = judge_prompt(&groups, &[]).unwrap();
        let v: serde_json::Value = serde_json::from_str(&prompt).unwrap();
        assert_eq!(v["reorder_groups"][0]["index"], 0);
        assert_eq!(v["reorder_groups"][0]["texts"][0], "alpha beta");
        assert_eq!(v["reorder_groups"][0]["texts"][1], "beta alpha");
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

    #[test]
    fn merge_promotions_dedups_against_existing_and_batch() {
        let (first, added) = merge_promotions(
            "",
            &["Never use em-dashes.".into(), "never USE em-dashes!".into()],
        );
        assert_eq!(added, 1);
        assert!(first.starts_with("# Learned\n"));
        let (second, added) = merge_promotions(&first, &["Never use em-dashes.".into()]);
        assert_eq!(added, 0);
        assert_eq!(second.trim_end(), first.trim_end());
        let (_, added) = merge_promotions(&first, &["x".into()]);
        assert_eq!(added, 0, "too-short lines dropped");
    }
}
