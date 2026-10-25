//! The semantic consolidation tier (MOT-5384): an LLM judge over each new
//! memory and its nearest neighbours, deciding MERGES (the same fact
//! restated) and SUPERSEDES (a state change replacing an outdated fact).
//!
//! Adapted from Hindsight's consolidator
//! (`hindsight_api/engine/consolidation/{consolidator,prompts}.py`): pending
//! records processed oldest-first in bounded batches; per record, candidates
//! from the RAW lexical and semantic arms INTERLEAVED (a fused rank buries
//! the twin); one LLM call per batch whose every action carries a reason;
//! targets only from the record's own candidates; parse failures bisect the
//! batch down to one record, and a record that still fails is skipped so it
//! cannot wedge the tier. Our model has no observation layer — inputs and
//! candidates are the same kind of record — so Hindsight's update-with-
//! evidence becomes a merge (retire one, the survivor absorbs it) and its
//! state-change update / delete+create becomes a supersede (old -> new).
//! No merged text is written: the judge picks the survivor among existing
//! records, so ids, provenance, and fingerprints never drift.
//!
//! Everything that decides is pure and unit tested; the async glue at the
//! bottom only moves data between the bus and those functions.

use std::collections::{BTreeMap, HashSet, VecDeque};

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::consolidate::{call, BankReport, MemoryRow};

/// Static system prompt (identical for every bank and batch, so providers
/// can cache it). Rules adapted from Hindsight's consolidation prompt and
/// its merge-or-keep dedup prompt.
pub const SEMANTIC_SYSTEM: &str = "You consolidate a user's long-term memory bank. Each NEW \
memory comes with CANDIDATE memories already in the bank (its nearest lexical and semantic \
neighbours). For each new memory, decide whether it duplicates or replaces one of ITS OWN \
candidates. Reply ONLY with a JSON object (no prose, no code fences):\n\
{\"actions\": [{\"type\": \"merge\", \"survivor\": \"<alias>\", \"retired\": \"<alias>\", \
\"reason\": \"<one sentence>\"}, {\"type\": \"supersede\", \"old\": \"<alias>\", \"new\": \
\"<alias>\", \"reason\": \"<one sentence>\"}]}\n\
\n\
Actions:\n\
- merge: both state the SAME fact (wording, language, or detail order aside). The retired one \
is removed and the survivor kept. The survivor MUST state every detail of the retired one \
(numbers, names, dates, times, places, conditions): pick the more complete one as survivor; \
when both are equally complete, keep the OLDER one (it carries the original provenance and \
wording); when each has a detail the other lacks, take no action.\n\
- supersede: both are about the SAME facet, and one REPLACES the other — a state change \
(\"moved to\", \"switched to\", \"now\", \"no longer\", a rescheduled date). \"old\" is the \
outdated one, \"new\" the current one; the current one is the one stated LAST (compare \"last \
stated\").\n\
- Anything else: no action. Most new memories need none; an empty list is the normal answer.\n\
\n\
Rules:\n\
1. MATCH BY ENTITY/FACET, NOT TOPIC: memories on the same topic about different entities or \
facets (two projects, two allergies, two people, two preferences) are distinct — no action.\n\
2. If they differ in ANY important detail — a number or quantity, a named entity, a negation, \
a condition — and neither clearly replaces the other, take no action.\n\
3. Never compute or infer: act only on what the texts state.\n\
4. A memory marked (pinned) is never \"retired\" or \"old\".\n\
5. Pair a new memory only with one of ITS OWN listed candidates (another new memory counts \
when it is listed among them).\n\
6. Use the aliases exactly as shown (n1, c3, ...). Every action needs a one-sentence reason.";

/// The judge's reason is carried on memory's events; memory caps it too.
const MAX_REASON_CHARS: usize = 500;
/// Output budget for one batch reply.
const MAX_OUTPUT_TOKENS: u32 = 2_048;

/// Bank progress: the last pending record (by `(updated_at, id)`) whose
/// batch was fully judged and applied. Keyed on `updated_at` so an edited
/// or resurrected record is judged again.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub updated_at: u64,
    pub id: String,
}

impl Cursor {
    fn of(row: &MemoryRow) -> Self {
        Cursor {
            updated_at: row.updated_at,
            id: row.id.clone(),
        }
    }

    fn is_before(&self, row: &MemoryRow) -> bool {
        (self.updated_at, self.id.as_str()) < (row.updated_at, row.id.as_str())
    }
}

/// Records written after the cursor, oldest first, capped.
pub fn pending<'a>(rows: &'a [MemoryRow], cursor: &Cursor, max: usize) -> Vec<&'a MemoryRow> {
    let mut out: Vec<&MemoryRow> = rows.iter().filter(|r| cursor.is_before(r)).collect();
    out.sort_by(|a, b| (a.updated_at, &a.id).cmp(&(b.updated_at, &b.id)));
    out.truncate(max);
    out
}

/// Round-robin across the two arms (Hindsight's `interleave`): each arm's
/// top hit is guaranteed a slot, the record itself and repeats skipped.
pub fn interleave<'a>(
    lexical: &'a [MemoryRow],
    semantic: &'a [MemoryRow],
    self_id: &str,
    cap: usize,
) -> Vec<&'a MemoryRow> {
    let mut out: Vec<&MemoryRow> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::from([self_id]);
    for i in 0..lexical.len().max(semantic.len()) {
        for arm in [lexical, semantic] {
            if let Some(r) = arm.get(i) {
                if out.len() < cap && seen.insert(r.id.as_str()) {
                    out.push(r);
                }
            }
        }
    }
    out
}

/// One recall arm's reply. Only the exact label of the requested mode
/// counts: `semantic-unavailable` — or a hybrid label from an older memory
/// worker that ignores `mode` — must never pass for "no neighbours".
pub fn read_arm(reply: &Value, expected: &str) -> Result<Vec<MemoryRow>, String> {
    let label = reply.get("retrieval").and_then(Value::as_str).unwrap_or("");
    if label != expected {
        return Err(format!(
            "{expected} recall unavailable (retrieval: {label:?})"
        ));
    }
    Ok(reply
        .get("memories")
        .and_then(Value::as_array)
        .map(|hits| {
            hits.iter()
                .filter_map(|h| h.get("memory"))
                .filter_map(|m| serde_json::from_value::<MemoryRow>(m.clone()).ok())
                .collect()
        })
        .unwrap_or_default())
}

/// `YYYY-MM-DD HH:MM` (UTC) — the judge only needs ordering and the day.
pub fn stamp(ms: u64) -> String {
    let secs = (ms / 1_000) as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Howard Hinnant's civil-from-days (as in judge-openai).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        rem / 3_600,
        (rem % 3_600) / 60
    )
}

/// When the record's fact was last asserted: a re-observation (reinforce)
/// counts, so a fact the user restated after its "replacement" stays
/// current. Memory's own clock (`Memory::last_stated`): keep both alike.
pub fn last_stated(row: &MemoryRow) -> u64 {
    // `stated_at` ignores pin/tag edits and a late write (an extraction
    // pass run after newer turns); legacy records without it fall back to
    // `updated_at`.
    row.stated_at.unwrap_or(row.updated_at)
}

/// One judge batch: aliased records, ownership, and the prompt.
#[derive(Debug, Clone)]
pub struct Batch {
    /// alias -> record (n* = new memories, c* = candidates).
    pub records: BTreeMap<String, MemoryRow>,
    /// new-memory alias -> the aliases of ITS candidates.
    pub owns: BTreeMap<String, Vec<String>>,
    pub prompt: String,
}

/// Alias the batch (a candidate that is itself new keeps its `n` alias) and
/// render the user message.
pub fn build_batch(items: &[(MemoryRow, Vec<MemoryRow>)]) -> Batch {
    let mut alias_of: BTreeMap<String, String> = BTreeMap::new();
    let mut records: BTreeMap<String, MemoryRow> = BTreeMap::new();
    for (i, (row, _)) in items.iter().enumerate() {
        let alias = format!("n{}", i + 1);
        alias_of.insert(row.id.clone(), alias.clone());
        records.insert(alias, row.clone());
    }
    let mut next_c = 1;
    let mut owns: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (row, candidates) in items {
        let mut own = Vec::new();
        for c in candidates {
            let alias = alias_of
                .entry(c.id.clone())
                .or_insert_with(|| {
                    let a = format!("c{next_c}");
                    next_c += 1;
                    records.insert(a.clone(), c.clone());
                    a
                })
                .clone();
            if !own.contains(&alias) {
                own.push(alias);
            }
        }
        owns.insert(alias_of[&row.id].clone(), own);
    }

    let line = |alias: &str, r: &MemoryRow| {
        format!(
            "{alias} [created {} · last stated {}]{} {}",
            stamp(r.created_at),
            stamp(last_stated(r)),
            if r.pinned { " (pinned)" } else { "" },
            r.text
        )
    };
    let mut prompt = String::from("NEW memories (judge each against ITS OWN candidates):\n");
    for i in 1..=items.len() {
        let alias = format!("n{i}");
        prompt.push_str(&line(&alias, &records[&alias]));
        let own = owns.get(&alias).cloned().unwrap_or_default();
        prompt.push_str(&format!(
            "\n   candidates: {}\n",
            if own.is_empty() {
                "(none)".to_string()
            } else {
                own.join(", ")
            }
        ));
    }
    prompt.push_str("\nCANDIDATES:\n");
    for (alias, r) in records.iter().filter(|(a, _)| a.starts_with('c')) {
        prompt.push_str(&line(alias, r));
        prompt.push('\n');
    }
    Batch {
        records,
        owns,
        prompt,
    }
}

/// One action as the judge wrote it (validated before anything applies).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawAction {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub survivor: Option<String>,
    #[serde(default)]
    pub retired: Option<String>,
    #[serde(default)]
    pub old: Option<String>,
    #[serde(default)]
    pub new: Option<String>,
    #[serde(default)]
    pub reason: String,
}

/// Parse the judge's reply. Unlike the reorder judge, garbage is an ERROR
/// (it bisects the batch) — never an empty "no actions".
pub fn parse_actions(text: &str) -> Result<Vec<RawAction>, String> {
    #[derive(Deserialize)]
    struct Reply {
        actions: Vec<RawAction>,
    }
    let trimmed = text.trim();
    let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) else {
        return Err("judge reply has no JSON object".into());
    };
    if end < start {
        return Err("judge reply has no JSON object".into());
    }
    serde_json::from_str::<Reply>(&trimmed[start..=end])
        .map(|r| r.actions)
        .map_err(|e| format!("unparseable judge reply: {e}"))
}

/// How one `router::complete` attempt ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Reply(String),
    /// Provider/transport trouble (router absent, auth, quota, transient):
    /// stop the tier for this bank, keep the cursor, retry next pass.
    Transport(String),
    /// Something about THIS input (unparseable reply, truncation, context
    /// overflow): bisect the batch.
    InputShaped(String),
}

pub fn classify(result: &Result<Value, String>) -> Outcome {
    let reply = match result {
        Ok(v) => v,
        Err(e) if e.contains("context_overflow") => return Outcome::InputShaped(e.clone()),
        Err(e) => return Outcome::Transport(e.clone()),
    };
    let message = reply.get("message").cloned().unwrap_or(Value::Null);
    match message.get("stop_reason").and_then(Value::as_str) {
        Some("error") => {
            let kind = message
                .get("error_kind")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let detail = message
                .get("error_message")
                .and_then(Value::as_str)
                .unwrap_or("");
            let why = format!("provider error {kind}: {detail}");
            match kind {
                "context_overflow" => Outcome::InputShaped(why),
                // `permanent` is a billing wall, a missing credential, or a
                // retired model: bisecting would skip every pending memory
                // past the cursor. Stop and hold instead.
                _ => Outcome::Transport(why),
            }
        }
        Some("length") => Outcome::InputShaped("judge reply truncated".into()),
        _ => Outcome::Reply(crate::llm::reply_text(reply)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ActionKind {
    Merge,
    Supersede,
}

/// A validated decision: `retire` leaves in favor of `keep` (a merge
/// survivor, or a supersede's new record).
#[derive(Debug, Clone)]
pub struct Action {
    pub kind: ActionKind,
    pub keep: MemoryRow,
    pub retire: MemoryRow,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct Verdict {
    pub accepted: Vec<Action>,
    pub rejected: Vec<(String, &'static str)>,
    /// Valid actions beyond the run's supersede budget.
    pub capped: usize,
}

/// Digit-bearing tokens ("2026-10-15" -> {2026, 10, 15}), so a merge that
/// would retire a number the survivor lacks is caught across formats.
fn number_tokens(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().any(|c| c.is_ascii_digit()))
        .map(|t| {
            // "09:30" and "9:30" are the same number; a decimal's fraction
            // ("0.05" vs "0.5") is not, so it keeps its zeros.
            let start = t.as_ptr() as usize - text.as_ptr() as usize;
            let fraction = text[..start].ends_with(['.', ',']);
            if !fraction && t.bytes().all(|b| b.is_ascii_digit()) {
                let trimmed = t.trim_start_matches('0');
                if trimmed.is_empty() {
                    "0".to_string()
                } else {
                    trimmed.to_string()
                }
            } else {
                t.to_lowercase()
            }
        })
        .collect()
}

/// Every check a judge action must pass before anything is written.
/// `retired` = ids retired earlier this run; `budget` = supersedes left.
pub fn validate(
    raw: &[RawAction],
    batch: &Batch,
    retired: &HashSet<String>,
    budget: usize,
) -> Verdict {
    let mut verdict = Verdict::default();
    let mut gone: HashSet<String> = retired.clone();
    // A merge accepted earlier in this reply makes the survivor carry the
    // later statement of the two (as `memory::supersede` does), so a
    // following supersede compares against that, not the stale snapshot.
    let mut stated: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for a in raw {
        let label = format!("{} {:?}", a.kind, a);
        let (kind, keep_alias, retire_alias) = match a.kind.as_str() {
            "merge" => (ActionKind::Merge, &a.survivor, &a.retired),
            "supersede" => (ActionKind::Supersede, &a.new, &a.old),
            _ => {
                verdict.rejected.push((label, "unknown action type"));
                continue;
            }
        };
        let (Some(keep_alias), Some(retire_alias)) = (keep_alias, retire_alias) else {
            verdict.rejected.push((label, "missing alias"));
            continue;
        };
        let (Some(keep), Some(retire)) = (
            batch.records.get(keep_alias),
            batch.records.get(retire_alias),
        ) else {
            verdict.rejected.push((label, "unknown alias"));
            continue;
        };
        if keep.id == retire.id {
            verdict
                .rejected
                .push((label, "a memory cannot replace itself"));
            continue;
        }
        // Hindsight: an update target must come from THAT fact's own
        // recall. One side must be a new memory listing the other.
        let owned = |owner: &str, other: &str| {
            batch
                .owns
                .get(owner)
                .is_some_and(|own| own.iter().any(|o| o == other))
        };
        if !owned(keep_alias, retire_alias) && !owned(retire_alias, keep_alias) {
            verdict
                .rejected
                .push((label, "not one of that new memory's candidates"));
            continue;
        }
        if retire.pinned {
            verdict
                .rejected
                .push((label, "pinned memories are untouchable"));
            continue;
        }
        if gone.contains(&retire.id) {
            verdict.rejected.push((label, "already retired this run"));
            continue;
        }
        if gone.contains(&keep.id) {
            verdict
                .rejected
                .push((label, "survivor was retired this run"));
            continue;
        }
        let seen = |r: &MemoryRow| stated.get(&r.id).copied().unwrap_or_else(|| last_stated(r));
        if kind == ActionKind::Supersede && seen(retire) > seen(keep) {
            verdict
                .rejected
                .push((label, "the outdated side was stated after its replacement"));
            continue;
        }
        // ponytail: digits-only detail check; a names-only superset
        // ("Dr. Silva") still relies on the prompt's survivor rule.
        if kind == ActionKind::Merge
            && !number_tokens(&retire.text).is_subset(&number_tokens(&keep.text))
        {
            verdict
                .rejected
                .push((label, "the survivor would lose a number"));
            continue;
        }
        let reason: String = a.reason.trim().chars().take(MAX_REASON_CHARS).collect();
        if reason.is_empty() {
            verdict.rejected.push((label, "no reason"));
            continue;
        }
        if verdict.accepted.len() >= budget {
            verdict.capped += 1;
            continue;
        }
        gone.insert(retire.id.clone());
        if kind == ActionKind::Merge && !keep.pinned {
            let latest = seen(keep).max(seen(retire));
            stated.insert(keep.id.clone(), latest);
        }
        verdict.accepted.push(Action {
            kind,
            keep: keep.clone(),
            retire: retire.clone(),
            reason,
        });
    }
    verdict
}

/// One semantic-tier decision as reported (and kept in `last_report`).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SemanticAction {
    pub kind: ActionKind,
    /// Merge survivor / supersede replacement.
    pub keep_id: String,
    pub keep_text: String,
    /// Merge duplicate / supersede outdated record.
    pub retire_id: String,
    pub retire_text: String,
    pub reason: String,
    /// False on dry runs. Actions dropped by the pre-write freshness check
    /// are not listed here; they count in `semantic_rejected` with a note.
    pub applied: bool,
}

// ---- async glue ----------------------------------------------------------

pub struct SemanticOptions {
    pub model: String,
    pub batch: usize,
    pub candidates: usize,
    pub max_per_run: usize,
}

fn cursor_key(bank: &str) -> String {
    format!("semantic_cursor:{bank}")
}

async fn recall_arm(
    iii: &IIIClient,
    bank: &str,
    text: &str,
    mode: &str,
    k: usize,
) -> Result<Vec<MemoryRow>, String> {
    let reply = call(
        iii,
        "memory::recall",
        json!({ "bank": bank, "query": text, "limit": k + 1, "mode": mode }),
    )
    .await?;
    read_arm(&reply, mode)
}

/// Re-read a record right before writing: the judge decided on the text it
/// saw, so a record edited, retired, or gone meanwhile drops the action
/// (`Ok(false)`). Any other failure is transport (`Err`), which stops the
/// tier with the cursor held, never a silent drop.
async fn still_as_seen(iii: &IIIClient, bank: &str, seen: &MemoryRow) -> Result<bool, String> {
    match call(iii, "memory::get", json!({ "bank": bank, "id": seen.id })).await {
        Ok(v) => {
            let m = v.get("memory").cloned().unwrap_or(Value::Null);
            Ok(m.get("invalid_at").is_none_or(Value::is_null)
                && m.get("text").and_then(Value::as_str) == Some(seen.text.as_str()))
        }
        Err(e) if e.contains("memory_not_found") => Ok(false),
        Err(e) => Err(e),
    }
}

enum Step {
    /// Batch fully judged and applied (or a lone poison record skipped).
    Done,
    /// Stop the tier for this bank; the cursor stays before this batch.
    Stop,
    /// Input-shaped failure on a multi-record batch: judge halves instead.
    Split,
}

/// The tier for one bank. Runs LAST (after the deterministic and reorder
/// tiers) on a fresh fetch, so it never sees records those tiers retired.
pub async fn run_semantic_tier(
    iii: &IIIClient,
    bank: &str,
    opts: &SemanticOptions,
    dry_run: bool,
    budget: &mut usize,
    report: &mut BankReport,
) {
    let rows = match crate::consolidate::fetch_live(iii, bank, &mut report.errors).await {
        Ok(rows) => rows,
        Err(e) => {
            report.errors.push(e);
            return;
        }
    };
    // A failed read must never pass for "no cursor": that would rescan the
    // bank from its oldest record and overwrite real progress.
    let mut cursor: Cursor = match crate::functions::state_try_get(iii, &cursor_key(bank)).await {
        Ok(None) => Cursor::default(),
        Ok(Some(v)) => match serde_json::from_value(v) {
            Ok(c) => c,
            Err(e) => {
                report
                    .notes
                    .push(format!("semantic tier skipped: cursor undecodable: {e}"));
                return;
            }
        },
        Err(e) => {
            report
                .notes
                .push(format!("semantic tier skipped: cursor unreadable: {e}"));
            return;
        }
    };
    let todo: Vec<MemoryRow> = pending(&rows, &cursor, opts.max_per_run)
        .into_iter()
        .cloned()
        .collect();
    let mut queue: VecDeque<Vec<MemoryRow>> = todo
        .chunks(opts.batch.max(1))
        .map(<[MemoryRow]>::to_vec)
        .collect();
    let mut retired: HashSet<String> = HashSet::new();

    while let Some(group) = queue.pop_front() {
        if *budget == 0 {
            report
                .notes
                .push("semantic tier deferred: per-run supersede cap reached".into());
            break;
        }
        match judge_group(
            iii,
            bank,
            opts,
            dry_run,
            budget,
            report,
            &group,
            &mut retired,
        )
        .await
        {
            Step::Done => {
                if !dry_run {
                    if let Some(last) = group.last() {
                        cursor = Cursor::of(last);
                        if let Err(e) = crate::functions::state_set(
                            iii,
                            &cursor_key(bank),
                            serde_json::to_value(&cursor).unwrap_or(Value::Null),
                        )
                        .await
                        {
                            report.notes.push(format!("semantic cursor not saved: {e}"));
                            break;
                        }
                    }
                }
            }
            Step::Stop => break,
            Step::Split => {
                let (a, b) = group.split_at(group.len() / 2);
                queue.push_front(b.to_vec());
                queue.push_front(a.to_vec());
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn judge_group(
    iii: &IIIClient,
    bank: &str,
    opts: &SemanticOptions,
    dry_run: bool,
    budget: &mut usize,
    report: &mut BankReport,
    group: &[MemoryRow],
    retired: &mut HashSet<String>,
) -> Step {
    // Candidates are recalled per batch, after earlier batches wrote: a
    // record retired since then is no longer live and drops out.
    let mut items: Vec<(MemoryRow, Vec<MemoryRow>)> = Vec::new();
    for row in group.iter().filter(|r| !retired.contains(&r.id)) {
        let lexical = recall_arm(iii, bank, &row.text, "lexical", opts.candidates).await;
        let semantic = recall_arm(iii, bank, &row.text, "semantic", opts.candidates).await;
        let (lexical, semantic) = match (lexical, semantic) {
            (Ok(l), Ok(s)) => (l, s),
            (Err(e), _) | (_, Err(e)) => {
                report.notes.push(format!("semantic tier stopped: {e}"));
                return Step::Stop;
            }
        };
        let candidates: Vec<MemoryRow> =
            interleave(&lexical, &semantic, &row.id, 2 * opts.candidates)
                .into_iter()
                .filter(|c| !retired.contains(&c.id))
                .cloned()
                .collect();
        items.push((row.clone(), candidates));
    }
    if items.iter().all(|(_, c)| c.is_empty()) {
        return Step::Done;
    }

    let batch = build_batch(&items);
    let result = crate::llm::complete(
        iii,
        &opts.model,
        SEMANTIC_SYSTEM,
        batch.prompt.clone(),
        MAX_OUTPUT_TOKENS,
    )
    .await;
    let parsed = match classify(&result) {
        Outcome::Reply(text) => parse_actions(&text).map_err(Outcome::InputShaped),
        other => Err(other),
    };
    let actions = match parsed {
        Ok(actions) => actions,
        Err(Outcome::Transport(e)) => {
            report.notes.push(format!("semantic tier stopped: {e}"));
            return Step::Stop;
        }
        Err(_) if group.len() > 1 => return Step::Split,
        Err(_) => {
            report.semantic_failed.push(group[0].id.clone());
            return Step::Done;
        }
    };

    let verdict = validate(&actions, &batch, retired, *budget);
    report.semantic_rejected += verdict.rejected.len();
    for (what, why) in &verdict.rejected {
        tracing::debug!(bank, why, action = %what, "semantic action rejected");
    }
    for action in verdict.accepted {
        let mut applied = false;
        if !dry_run {
            let fresh = match (
                still_as_seen(iii, bank, &action.keep).await,
                still_as_seen(iii, bank, &action.retire).await,
            ) {
                (Ok(a), Ok(b)) => a && b,
                (Err(e), _) | (_, Err(e)) => {
                    report.notes.push(format!("semantic tier stopped: {e}"));
                    return Step::Stop;
                }
            };
            if !fresh {
                report.semantic_rejected += 1;
                report.notes.push(format!(
                    "semantic action dropped: `{}` or `{}` changed since the judge read it",
                    action.keep.id, action.retire.id
                ));
                continue;
            }
            let reply = call(
                iii,
                "memory::supersede",
                json!({
                    "bank": bank,
                    "id": action.retire.id,
                    "superseded_by": action.keep.id,
                    "reinforce": action.kind == ActionKind::Merge,
                    "reason": action.reason,
                }),
            )
            .await;
            match reply {
                Ok(reply) => {
                    applied = true;
                    *budget = budget.saturating_sub(1);
                    report.superseded += 1;
                    if action.kind == ActionKind::Merge {
                        crate::consolidate::tally_reinforcement(report, &reply);
                    }
                }
                Err(e) => {
                    report.errors.push(e);
                    return Step::Stop;
                }
            }
        }
        retired.insert(action.retire.id.clone());
        report.semantic.push(SemanticAction {
            kind: action.kind,
            keep_id: action.keep.id,
            keep_text: action.keep.text,
            retire_id: action.retire.id,
            retire_text: action.retire.text,
            reason: action.reason,
            applied,
        });
    }
    if verdict.capped > 0 {
        report.capped += verdict.capped;
        report
            .notes
            .push("semantic tier deferred: per-run supersede cap reached".into());
        return Step::Stop;
    }
    Step::Done
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, text: &str, created_at: u64, updated_at: u64) -> MemoryRow {
        MemoryRow {
            id: id.into(),
            text: text.into(),
            pinned: false,
            corroboration: 0,
            created_at,
            updated_at,
            stated_at: None,
        }
    }

    fn action(kind: &str, keep: &str, retire: &str, reason: &str) -> RawAction {
        let (a, b) = (Some(keep.to_string()), Some(retire.to_string()));
        match kind {
            "merge" => RawAction {
                kind: kind.into(),
                survivor: a,
                retired: b,
                reason: reason.into(),
                ..Default::default()
            },
            _ => RawAction {
                kind: kind.into(),
                new: a,
                old: b,
                reason: reason.into(),
                ..Default::default()
            },
        }
    }

    #[test]
    fn pending_is_after_the_cursor_oldest_first_and_capped() {
        let rows = vec![
            row("b", "x", 0, 20),
            row("a", "x", 0, 20),
            row("c", "x", 0, 10),
            row("d", "x", 0, 30),
        ];
        let ids = |p: Vec<&MemoryRow>| p.iter().map(|r| r.id.clone()).collect::<Vec<_>>();
        assert_eq!(
            ids(pending(&rows, &Cursor::default(), 10)),
            vec!["c", "a", "b", "d"]
        );
        // (updated_at, id) tie-break: the cursor at (20, "a") still sees b.
        let cursor = Cursor {
            updated_at: 20,
            id: "a".into(),
        };
        assert_eq!(ids(pending(&rows, &cursor, 10)), vec!["b", "d"]);
        assert_eq!(ids(pending(&rows, &Cursor::default(), 2)), vec!["c", "a"]);
        // An edited/resurrected record (old created_at, fresh updated_at)
        // comes back.
        let edited = vec![row("old", "x", 1, 99)];
        let cursor = Cursor {
            updated_at: 50,
            id: "z".into(),
        };
        assert_eq!(ids(pending(&edited, &cursor, 10)), vec!["old"]);
    }

    #[test]
    fn interleave_round_robins_dedups_skips_self_and_caps() {
        let lex = vec![
            row("me", "", 0, 0),
            row("l1", "", 0, 0),
            row("both", "", 0, 0),
        ];
        let sem = vec![
            row("s1", "", 0, 0),
            row("both", "", 0, 0),
            row("s2", "", 0, 0),
        ];
        let ids: Vec<&str> = interleave(&lex, &sem, "me", 10)
            .iter()
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(ids, vec!["s1", "l1", "both", "s2"]);
        let capped: Vec<&str> = interleave(&lex, &sem, "me", 2)
            .iter()
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(capped, vec!["s1", "l1"]);
    }

    #[test]
    fn an_arm_counts_only_with_its_exact_label() {
        let hit = json!({"memory": {"id": "x", "text": "t", "created_at": 1, "updated_at": 2}, "score": 0.9});
        let ok = read_arm(
            &json!({"retrieval": "semantic", "memories": [hit]}),
            "semantic",
        )
        .unwrap();
        assert_eq!(ok[0].id, "x");
        assert!(read_arm(
            &json!({"retrieval": "semantic-unavailable", "memories": []}),
            "semantic"
        )
        .is_err());
        // An older memory worker ignores `mode` and answers hybrid.
        assert!(read_arm(
            &json!({"retrieval": "bm25-entity", "memories": []}),
            "lexical"
        )
        .is_err());
    }

    #[test]
    fn stamp_formats_utc_minutes() {
        assert_eq!(stamp(0), "1970-01-01 00:00");
        // 2025-10-14T12:34:00Z
        assert_eq!(stamp(1_760_445_240_000), "2025-10-14 12:34");
    }

    #[test]
    fn batch_aliases_new_memories_first_and_reuses_n_for_new_candidates() {
        let n1 = row("id1", "dentist on 2026-10-15", 1, 1);
        let n2 = row("id2", "dentist moved to 2026-10-20", 2, 2);
        let c = row("idc", "user lives in lisbon", 0, 0);
        let batch = build_batch(&[
            (n1.clone(), vec![n2.clone(), c.clone()]),
            (n2.clone(), vec![n1.clone()]),
        ]);
        assert_eq!(batch.records["n1"].id, "id1");
        assert_eq!(batch.records["n2"].id, "id2");
        assert_eq!(batch.records["c1"].id, "idc");
        assert_eq!(batch.owns["n1"], vec!["n2", "c1"]);
        assert_eq!(batch.owns["n2"], vec!["n1"]);
        assert!(batch.prompt.contains("n1 [created 1970-01-01 00:00"));
        assert!(batch.prompt.contains("candidates: n2, c1"));
        assert!(
            batch.prompt.contains("c1 ["),
            "candidates listed once with their text"
        );
    }

    #[test]
    fn parse_requires_a_json_actions_object() {
        let ok = parse_actions("```json\n{\"actions\": [{\"type\": \"merge\", \"survivor\": \"c1\", \"retired\": \"n1\", \"reason\": \"same\"}]}\n```").unwrap();
        assert_eq!(ok.len(), 1);
        assert_eq!(parse_actions("{\"actions\": []}").unwrap().len(), 0);
        assert!(
            parse_actions("no idea").is_err(),
            "garbage must bisect, not read as no-op"
        );
        assert!(parse_actions("{\"merge_groups\": []}").is_err());
        assert!(parse_actions("} {").is_err());
    }

    #[test]
    fn classify_separates_transport_from_input_shaped() {
        let reply = |stop: &str, kind: &str| json!({"message": {"stop_reason": stop, "error_kind": kind, "content": [{"type": "text", "text": "{\"actions\": []}"}]}});
        assert!(matches!(
            classify(&Err("router::complete: function_not_found".into())),
            Outcome::Transport(_)
        ));
        assert!(matches!(
            classify(&Err("router/context_overflow: too long".into())),
            Outcome::InputShaped(_)
        ));
        assert!(matches!(
            classify(&Ok(reply("error", "rate_limited"))),
            Outcome::Transport(_)
        ));
        assert!(matches!(
            classify(&Ok(reply("error", "auth_expired"))),
            Outcome::Transport(_)
        ));
        assert!(matches!(
            classify(&Ok(reply("error", "context_overflow"))),
            Outcome::InputShaped(_)
        ));
        assert!(matches!(
            classify(&Ok(reply("error", "permanent"))),
            Outcome::Transport(_)
        ));
        assert!(matches!(
            classify(&Ok(reply("length", ""))),
            Outcome::InputShaped(_)
        ));
        assert_eq!(
            classify(&Ok(reply("stop", ""))),
            Outcome::Reply("{\"actions\": []}".into())
        );
    }

    fn dentist_batch() -> Batch {
        let old = row(
            "old",
            "User has a dentist appointment on 2026-10-15.",
            100,
            100,
        );
        let new = row(
            "new",
            "User moved the dentist appointment to 2026-10-20.",
            200,
            200,
        );
        let para = row(
            "para",
            "User's dentist appointment is on 2026-10-15 at 9:30.",
            300,
            300,
        );
        let other = row("other", "User is allergic to peanuts.", 50, 50);
        build_batch(&[
            (new.clone(), vec![old.clone(), other.clone()]),
            (para, vec![old]),
        ])
    }

    #[test]
    fn validate_accepts_owned_pairs_and_counts_budget_as_capped() {
        let batch = dentist_batch();
        // n1=new, n2=para, c1=old, c2=other
        let raw = vec![
            action("supersede", "n1", "c1", "the appointment was rescheduled"),
            action("merge", "n2", "c1", "same appointment restated"),
        ];
        let v = validate(&raw, &batch, &HashSet::new(), 10);
        assert_eq!(v.accepted.len(), 1, "{:?}", v.rejected);
        assert_eq!(v.accepted[0].retire.id, "old");
        assert_eq!(
            v.rejected.len(),
            1,
            "c1 is already retired by the first action"
        );
        let capped = validate(&raw[..1], &batch, &HashSet::new(), 0);
        assert!(capped.accepted.is_empty());
        assert_eq!(capped.capped, 1);
    }

    #[test]
    fn validate_rejects_every_unsafe_shape() {
        let batch = dentist_batch();
        let none = HashSet::new();
        let reject = |a: RawAction| {
            let v = validate(&[a], &batch, &none, 10);
            assert!(v.accepted.is_empty(), "{v:?}");
            v.rejected[0].1
        };
        assert_eq!(
            reject(action("rename", "n1", "c1", "r")),
            "unknown action type"
        );
        assert_eq!(reject(action("merge", "n1", "c9", "r")), "unknown alias");
        assert_eq!(
            reject(action("merge", "n1", "n1", "r")),
            "a memory cannot replace itself"
        );
        // n2 (para) does not list c2 (other) as a candidate.
        assert_eq!(
            reject(action("merge", "n2", "c2", "r")),
            "not one of that new memory's candidates"
        );
        // Direction: the outdated side cannot be the one stated last.
        assert_eq!(
            reject(action("supersede", "c1", "n1", "r")),
            "the outdated side was stated after its replacement"
        );
        // Merging the 9:30 detail away loses a number.
        assert_eq!(
            reject(action("merge", "c1", "n2", "same")),
            "the survivor would lose a number"
        );
        assert_eq!(reject(action("supersede", "n1", "c1", "   ")), "no reason");
        let mut gone = HashSet::new();
        gone.insert("old".to_string());
        let v = validate(&[action("supersede", "n1", "c1", "r")], &batch, &gone, 10);
        assert_eq!(v.rejected[0].1, "already retired this run");
    }

    #[test]
    fn validate_never_retires_pinned_and_allows_new_new_pairs() {
        let mut pinned_old = row("old", "User lives in São Paulo.", 1, 1);
        pinned_old.pinned = true;
        let new = row("new", "User moved to Lisbon.", 2, 2);
        let batch = build_batch(&[(new.clone(), vec![pinned_old])]);
        let v = validate(
            &[action("supersede", "n1", "c1", "moved")],
            &batch,
            &HashSet::new(),
            10,
        );
        assert_eq!(v.rejected[0].1, "pinned memories are untouchable");

        // Same-session twins: two NEW memories, each listing the other.
        let a = row("a", "Dentist on 2026-10-15.", 10, 10);
        let b = row("b", "Dentist moved to 2026-10-20.", 20, 20);
        let batch = build_batch(&[(a.clone(), vec![b.clone()]), (b, vec![a])]);
        let raw = vec![
            action("supersede", "n2", "n1", "rescheduled"),
            action("supersede", "n1", "n2", "reverse"),
        ];
        let v = validate(&raw, &batch, &HashSet::new(), 10);
        assert_eq!(v.accepted.len(), 1);
        assert_eq!(v.accepted[0].retire.id, "a");
        assert_eq!(v.rejected.len(), 1, "the reverse names a retired record");
    }

    #[test]
    fn a_merge_earlier_in_the_reply_carries_the_later_statement() {
        // n1 = the move (stated 200), n2 = Tuesdays restated (300),
        // c1 = Tuesdays, old (100). Merging n2 into c1 makes c1 "stated at
        // 300", so superseding c1 by the move must now be rejected.
        let n1 = row("n1", "Dentist moved to Thursdays.", 200, 200);
        let n2 = row("n2", "Dentist is on Tuesdays.", 300, 300);
        let c1 = row("c1", "User's dentist is on Tuesdays.", 100, 100);
        let batch = build_batch(&[(n1, vec![c1.clone()]), (n2, vec![c1])]);
        let raw = vec![
            action("merge", "c1", "n2", "same weekday restated"),
            action("supersede", "n1", "c1", "moved"),
        ];
        let v = validate(&raw, &batch, &HashSet::new(), 10);
        assert_eq!(v.accepted.len(), 1);
        assert_eq!(
            v.rejected[0].1,
            "the outdated side was stated after its replacement"
        );
    }

    #[test]
    fn pin_and_tag_edits_do_not_count_as_a_restatement() {
        // A was stated first and only TAG-edited later (updated_at 300,
        // stated_at 10): the move B (stated 200) still supersedes it.
        let mut a = row("a", "User lives in São Paulo.", 10, 300);
        a.stated_at = Some(10);
        let mut b = row("b", "User moved to Lisbon.", 200, 200);
        b.stated_at = Some(200);
        let batch = build_batch(&[(b, vec![a])]);
        let v = validate(
            &[action("supersede", "n1", "c1", "moved")],
            &batch,
            &HashSet::new(),
            10,
        );
        assert_eq!(v.accepted.len(), 1, "{:?}", v.rejected);
    }

    #[test]
    fn the_newer_statement_wins_by_when_it_was_said_not_written() {
        // A was said first (10) but written last (300: a late extraction
        // pass); the move B, said at 200, supersedes it.
        let mut a = row("a", "User lives in São Paulo.", 300, 300);
        a.stated_at = Some(10);
        let mut b = row("b", "User moved to Lisbon.", 200, 200);
        b.stated_at = Some(200);
        let batch = build_batch(&[(b, vec![a])]);
        let v = validate(
            &[action("supersede", "n1", "c1", "moved")],
            &batch,
            &HashSet::new(),
            10,
        );
        assert_eq!(v.accepted.len(), 1, "{:?}", v.rejected);
    }

    #[test]
    fn numbers_match_across_padding_but_not_across_decimals() {
        assert_eq!(
            number_tokens("at 09:30 on 05/10"),
            number_tokens("at 9:30 on 5/10")
        );
        assert_ne!(number_tokens("dose 0.05 mg"), number_tokens("dose 0.5 mg"));
        let survivor = row("s", "Appointment on 2026-10-05 at 9:30.", 1, 1);
        let dup = row("d", "Consulta em 5/10/2026 às 09:30.", 2, 2);
        let batch = build_batch(&[(dup, vec![survivor])]);
        let v = validate(
            &[action("merge", "c1", "n1", "same appointment")],
            &batch,
            &HashSet::new(),
            10,
        );
        assert_eq!(v.accepted.len(), 1, "{:?}", v.rejected);
    }

    #[test]
    fn a_repeated_pending_row_never_panics_the_batch() {
        let a = row("a", "x", 1, 1);
        let batch = build_batch(&[(a.clone(), vec![]), (a, vec![])]);
        assert!(batch.prompt.contains("n1 ["));
    }

    #[test]
    fn a_restated_fact_stays_current_and_long_reasons_are_clamped() {
        // A ("Tuesdays", created first) was re-observed AFTER B
        // ("moved to Thursdays") was written: A is the current statement.
        let a = row("a", "Dentist is on Tuesdays.", 10, 300);
        let b = row("b", "Dentist moved to Thursdays.", 200, 200);
        let batch = build_batch(&[(b, vec![a])]);
        let v = validate(
            &[action("supersede", "n1", "c1", "moved")],
            &batch,
            &HashSet::new(),
            10,
        );
        assert_eq!(
            v.rejected[0].1,
            "the outdated side was stated after its replacement"
        );

        let old = row("o", "User uses Jenkins.", 1, 1);
        let new = row("n", "User migrated to GitHub Actions.", 2, 2);
        let batch = build_batch(&[(new, vec![old])]);
        let long = "ção".repeat(300);
        let v = validate(
            &[action("supersede", "n1", "c1", &long)],
            &batch,
            &HashSet::new(),
            10,
        );
        assert_eq!(v.accepted[0].reason.chars().count(), MAX_REASON_CHARS);
    }
}
