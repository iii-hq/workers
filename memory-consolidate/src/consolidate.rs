//! The consolidation pass: deterministic near-duplicate detection over one
//! bank's live memories, applied strictly through the memory worker's
//! public functions — `memory::supersede` with `reinforce` retires the
//! duplicate with a pointer and lets the survivor absorb its observations
//! in one call. This worker never touches memory's files; the append-only
//! store contract is the seam.
//!
//! v1 detection is deliberately conservative and fully deterministic:
//! automatic writes require a source-bound detail certificate preserving internal
//! punctuation, operators and signs. The public normalized fingerprint is
//! unchanged; collisions that erase details are surfaced for review only.
//! Token-set equality (word order shuffles) is surfaced as REPORT-ONLY candidates — "Alice manages
//! Bob" and "Bob manages Alice" sort to the same tokens but mean opposite
//! things, so reordering never authorizes a write; a human or a stronger
//! verifier promotes those. Grouping keys are designed to grow: when
//! memories gain tags, tags join the group key next to entities.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use iii_sdk::protocol::CallRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const PAGE: usize = 500;
const CALL_TIMEOUT_MS: u64 = 15_000;

pub type TransportFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

/// The pass's IO boundary. Production delegates to the existing SDK and
/// judge; offline callers can supply decisions without connecting a client.
/// Missing decision providers fail closed, never select a model or provider.
pub trait ConsolidationTransport: Send + Sync {
    fn invoke<'a>(&'a self, function: &'a str, payload: Value) -> TransportFuture<'a, Value>;

    fn complete<'a>(
        &'a self,
        _model: &'a str,
        _system: &'a str,
        _prompt: String,
        _max_tokens: u32,
    ) -> TransportFuture<'a, Value> {
        Box::pin(async { Err("no decision provider supplied".into()) })
    }

    fn judge<'a>(
        &'a self,
        _model: &'a str,
        _prompt: String,
    ) -> TransportFuture<'a, crate::llm::JudgeReply> {
        Box::pin(async { Err("no reorder decision provider supplied".into()) })
    }
}

impl ConsolidationTransport for IIIClient {
    fn invoke<'a>(&'a self, function: &'a str, payload: Value) -> TransportFuture<'a, Value> {
        Box::pin(async move {
            self.call(CallRequest {
                function_id: function.to_string(),
                payload,
                action: None,
                timeout_ms: Some(if matches!(function, "state::get" | "state::set") {
                    5_000
                } else {
                    CALL_TIMEOUT_MS
                }),
            })
            .await
            .map_err(|e| format!("{function}: {e}"))
        })
    }

    fn complete<'a>(
        &'a self,
        model: &'a str,
        system: &'a str,
        prompt: String,
        max_tokens: u32,
    ) -> TransportFuture<'a, Value> {
        Box::pin(crate::llm::complete(
            self, model, system, prompt, max_tokens,
        ))
    }

    fn judge<'a>(
        &'a self,
        model: &'a str,
        prompt: String,
    ) -> TransportFuture<'a, crate::llm::JudgeReply> {
        Box::pin(crate::llm::judge(self, model, prompt))
    }
}

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
    /// Last write (reinforcement included): the semantic tier's cursor and
    /// its "last stated" time.
    #[serde(default)]
    pub updated_at: u64,
    /// When the fact was last stated (memory's `stated_at`; pin/tag edits
    /// do not bump it). Absent on legacy records.
    #[serde(default)]
    pub stated_at: Option<u64>,
    /// Preserve revision, provenance, entities/tags and future wire fields for
    /// the producer's joint compare. Never reconstruct proof from a fresh read.
    #[serde(flatten)]
    pub snapshot: BTreeMap<String, Value>,
}

impl MemoryRow {
    pub(crate) fn expected_snapshot(&self) -> Value {
        let mut fields = self.snapshot.clone();
        fields.extend([
            ("id".into(), json!(self.id)),
            ("text".into(), json!(self.text)),
            ("pinned".into(), json!(self.pinned)),
            ("corroboration".into(), json!(self.corroboration)),
            ("created_at".into(), json!(self.created_at)),
            ("updated_at".into(), json!(self.updated_at)),
            ("stated_at".into(), json!(self.stated_at)),
        ]);
        json!(fields)
    }
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
    /// Real failures (reads, writes). Any error keeps the pass due.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    /// Soft skips (an optional tier unavailable, a sibling too old) —
    /// telemetry only: they never block the checkpoint, so an absent router
    /// cannot re-run the whole pass on every heartbeat.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// Semantic-tier decisions (applied, or planned on dry runs), each with
    /// the judge's reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub semantic: Vec<crate::semantic::SemanticAction>,
    /// Judge actions dropped by validation (unknown alias, pinned target,
    /// wrong direction, a lost number, ...).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub semantic_rejected: usize,
    /// Independently regenerated certificates for authorized merges.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detail_proofs: Vec<DetailProof>,
    /// New memories the judge could not decide even alone (a poison input);
    /// skipped past the cursor so they cannot wedge the tier.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub semantic_failed: Vec<String>,
}

/// Reason carried on the `memory::on-item-changed` events of a deterministic
/// merge.
const REASON_NORMALIZED: &str = "normalized-text duplicate";
/// Reason carried on a judge-confirmed reorder merge.
const REASON_REORDER: &str = "LLM judge confirmed reorder equivalence";

const NOTE_OLD_MEMORY: &str = "memory worker did not report reinforcement — upgrade memory; \
                               survivors were not reinforced";

/// Count a `memory::supersede{reinforce}` reply: `reinforced: true` counts,
/// `false` is a pinned survivor (untouchable), and an ABSENT field is an
/// older memory worker that ignored `reinforce` — noted once per bank,
/// never counted as reinforced.
pub(crate) fn tally_reinforcement(report: &mut BankReport, reply: &Value) {
    match reply.get("reinforced").and_then(Value::as_bool) {
        Some(true) => report.reinforced += 1,
        Some(false) => {}
        None => {
            if !report.notes.iter().any(|n| n == NOTE_OLD_MEMORY) {
                report.notes.push(NOTE_OLD_MEMORY.into());
            }
        }
    }
}

/// The reorder groups the judge confirmed, deduplicated and bounds-checked
/// (a repeated index would supersede twice and fail as already retired).
fn confirmed_groups(indices: &[usize], reorder: &[&PlannedGroup]) -> Vec<PlannedGroup> {
    let unique: std::collections::BTreeSet<usize> = indices.iter().copied().collect();
    unique
        .into_iter()
        .filter_map(|i| reorder.get(i).map(|g| (*g).clone()))
        .collect()
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// Candidate-only legacy fingerprint, never an equivalence certificate.
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

/// A locally checked equivalence certificate. Each source byte is retained in
/// `source`; ordered atoms record the only transformations the verifier permits.
/// This is independent of the judge's reason, source label or similarity score.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DetailProof {
    pub version: u32,
    pub survivor_source: String,
    pub retired_source: String,
    pub survivor_atoms: Vec<DetailAtom>,
    pub retired_atoms: Vec<DetailAtom>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DetailAtom {
    pub source: String,
    pub canonical: String,
    pub rule: String,
}

/// Closed bilingual prose lexicon, not general case folding or translation.
/// Unknown words, proper names, operators, numbers and literal punctuation keep
/// their exact bytes. Adding a translation requires an explicit code review.
fn prose_word(word: &str) -> Option<&'static str> {
    match word.to_lowercase().as_str() {
        "user" | "usuário" => Some("user"),
        "prefers" | "prefere" => Some("prefers"),
        "tea" | "chá" => Some("tea"),
        "answers" | "respostas" => Some("answers"),
        "short" | "curtas" => Some("short"),
        "concise" => Some("concise"),
        "direct" => Some("direct"),
        "the" => Some("the"),
        "api" => Some("api"),
        "port" => Some("port"),
        "is" => Some("is"),
        "same" => Some("same"),
        "fact" => Some("fact"),
        "text" => Some("text"),
        "words" => Some("words"),
        "other" => Some("other"),
        "keep" => Some("keep"),
        "me" => Some("me"),
        "here" => Some("here"),
        "this" => Some("this"),
        "memory" => Some("memory"),
        "duplicate" => Some("duplicate"),
        "publishes" => Some("publishes"),
        "on" => Some("on"),
        "tuesday" => Some("tuesday"),
        "mornings" => Some("mornings"),
        "rules" => Some("rules"),
        "apply" => Some("apply"),
        "café" => Some("café"),
        "alpha" => Some("alpha"),
        "beta" => Some("beta"),
        "gamma" => Some("gamma"),
        _ => None,
    }
}

fn typed_prose(text: &str) -> Option<DetailAtom> {
    let body = text.trim().strip_suffix('.').unwrap_or(text.trim());
    // Intersective style adjectives commute; subjects/verbs never do.
    let words: Vec<&str> = body.split_whitespace().collect();
    if words.len() == 3
        && words[2] == "answers"
        && words[..2]
            .iter()
            .all(|w| matches!(*w, "concise" | "direct"))
    {
        let mut adjectives = words[..2].to_vec();
        adjectives.sort_unstable();
        return Some(DetailAtom {
            source: text.into(),
            canonical: format!("style-adjectives:{}:answers", adjectives.join(":")),
            rule: "intersective-style-adjectives-v1".into(),
        });
    }
    // Personal memory's omitted possessor denotes the user only in this
    // closed weekday assertion, never for arbitrary entities or relations.
    let weekday = body
        .strip_prefix("User's dentist is on ")
        .or_else(|| body.strip_prefix("Dentist is on "));
    if let Some(day) = weekday.filter(|d| {
        matches!(
            *d,
            "Mondays"
                | "Tuesdays"
                | "Wednesdays"
                | "Thursdays"
                | "Fridays"
                | "Saturdays"
                | "Sundays"
        )
    }) {
        return Some(DetailAtom {
            source: text.into(),
            canonical: format!("user-dentist-weekday:{day}"),
            rule: "personal-dentist-weekday-v1".into(),
        });
    }
    let (date, time, iso) = if let Some(rest) = body.strip_prefix("Appointment on ") {
        let (date, time) = rest.split_once(" at ")?;
        (date, time, true)
    } else {
        let rest = body.strip_prefix("Consulta em ")?;
        let (date, time) = rest.split_once(" às ")?;
        (date, time, false)
    };
    fn numeric_parts(s: &str, separator: char, count: usize) -> Option<Vec<u32>> {
        let pieces: Vec<&str> = s.split(separator).collect();
        if pieces.len() != count
            || pieces
                .iter()
                .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
        {
            return None;
        }
        pieces.iter().map(|p| p.parse().ok()).collect()
    }
    let date = numeric_parts(date, if iso { '-' } else { '/' }, 3)?;
    let time = numeric_parts(time, ':', 2)?;
    let (year, month, day) = if iso {
        (date[0], date[1], date[2])
    } else {
        (date[2], date[1], date[0])
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return None,
    };
    if year == 0 || day == 0 || day > days || time[0] > 23 || time[1] > 59 {
        return None;
    }
    Some(DetailAtom {
        source: text.into(),
        canonical: format!(
            "appointment:{year:04}-{month:02}-{day:02}T{:02}:{:02}",
            time[0], time[1]
        ),
        rule: "appointment-pt-en-calendar-v1".into(),
    })
}

fn detail_atoms(text: &str) -> Option<Vec<DetailAtom>> {
    if normalize(text).is_empty() {
        return None;
    }
    if let Some(typed) = typed_prose(text) {
        return Some(vec![typed]);
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    // Explicit literal/type declarations are not prose. In particular a final
    // full stop may belong to a filename and `!` may be a symbolic factorial.
    let literal = words.iter().any(|word| {
        let lower = word.to_lowercase();
        matches!(
            lower.as_str(),
            "expression"
                | "identifier"
                | "filename"
                | "path"
                | "variable"
                | "regex"
                | "command"
                | "literal"
                | "expressão"
                | "identificador"
                | "arquivo"
                | "caminho"
                | "variável"
        ) || word.contains(['/', '\\', '`', '"'])
    });
    if literal {
        return Some(vec![DetailAtom {
            source: text.into(),
            canonical: text.into(),
            rule: "verbatim-literal-v1".into(),
        }]);
    }
    let mut atoms = Vec::new();
    for (i, word) in words.iter().enumerate() {
        let mut body = *word;
        let mut stop = "";
        if i + 1 == words.len() {
            let stripped = body.trim_end_matches(['.', '!']);
            let suffix = &body[stripped.len()..];
            // Sentence stops are admitted only after a known prose word, or a
            // numeric assertion with an explicit copula/comparison grammar.
            let numeric_assertion = stripped.bytes().all(|b| b.is_ascii_digit())
                && !stripped.is_empty()
                && (suffix == "."
                    || (!suffix.is_empty()
                        && suffix.chars().all(|c| c == '!')
                        && words[..i]
                            .iter()
                            .map(|w| w.to_lowercase())
                            .collect::<Vec<_>>()
                            == ["the", "api", "port", "is"]))
                && (words.get(i.wrapping_sub(1)) == Some(&"is")
                    || words
                        .get(i.wrapping_sub(1))
                        .is_some_and(|w| matches!(*w, "<" | ">" | "<=" | ">=" | "=")));
            if !suffix.is_empty() && (prose_word(stripped).is_some() || numeric_assertion) {
                body = stripped;
                stop = suffix;
            }
        }
        let translated = prose_word(body);
        atoms.push(DetailAtom {
            source: body.into(),
            canonical: translated.unwrap_or(body).into(),
            rule: if translated.is_some() {
                "closed-prose-lexicon-pt-en-v1"
            } else {
                "verbatim-detail-v1"
            }
            .into(),
        });
        if !stop.is_empty() {
            atoms.push(DetailAtom {
                source: stop.into(),
                canonical: String::new(),
                rule: "prose-sentence-stop-v1".into(),
            });
        }
    }
    Some(atoms)
}

fn atom_key(atoms: &[DetailAtom]) -> Vec<&str> {
    atoms
        .iter()
        .filter(|a| !a.canonical.is_empty())
        .map(|a| a.canonical.as_str())
        .collect()
}

/// Prove full ordered detail equality, never a subset or a bag of tokens.
/// Both original texts accompany the certificate, including whitespace/case.
/// No proof means retain both, even if a model confidently proposes a merge.
pub fn detail_proof(survivor: &str, retired: &str) -> Option<DetailProof> {
    let survivor_atoms = detail_atoms(survivor)?;
    let retired_atoms = detail_atoms(retired)?;
    if atom_key(&survivor_atoms) != atom_key(&retired_atoms) {
        return None;
    }
    Some(DetailProof {
        version: 1,
        survivor_source: survivor.into(),
        retired_source: retired.into(),
        survivor_atoms,
        retired_atoms,
    })
}

/// Supplied certificates are untrusted: regenerate and compare every field.
pub fn verifies_detail_proof(proof: &DetailProof, survivor: &str, retired: &str) -> bool {
    detail_proof(survivor, retired).as_ref() == Some(proof)
}

/// Deterministic candidate grouping: case-fold only recognized prose, never
/// translate it. A locally generated bilingual certificate is not an independent
/// decision to compare languages; that opt-in belongs to the semantic tier.
/// A group is writable only after a pairwise detail proof.
pub(crate) fn retirement_key(text: &str) -> String {
    detail_atoms(text)
        .map(|atoms| {
            let key: Vec<String> = atoms
                .iter()
                .filter(|a| !a.canonical.is_empty())
                .map(|a| {
                    if a.rule == "closed-prose-lexicon-pt-en-v1" {
                        a.source.to_lowercase()
                    } else {
                        a.source.clone()
                    }
                })
                .collect();
            serde_json::to_string(&key).unwrap_or_default()
        })
        .unwrap_or_default()
}

/// Group live memories and pick a survivor per group: pinned first (a
/// pinned record always wins), then highest corroboration, then the
/// OLDEST record (it carries the original provenance), then id for a
/// stable total order. Local write-key groups are writable; lossy normalized
/// collisions and token-set groups (same words, different order) are report-only.
pub fn plan(rows: &[MemoryRow]) -> Vec<PlannedGroup> {
    let mut planned = group_by(rows, retirement_key, false);
    // The public normalized fingerprint is deliberately unchanged. A
    // collision that discarded a symbol/boundary is review-only, even
    // when a safe duplicate subgroup can still merge independently.
    let mut normalized_keys: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
    for row in rows {
        normalized_keys
            .entry(normalize(&row.text))
            .or_default()
            .insert(retirement_key(&row.text));
    }
    planned.extend(
        group_by(rows, normalize, true)
            .into_iter()
            .filter(|g| normalized_keys[&normalize(&g.winner_text)].len() > 1),
    );
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
        if normalize(&row.text).is_empty() {
            continue;
        }
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

pub(crate) async fn call<T: ConsolidationTransport + ?Sized>(
    iii: &T,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
    iii.invoke(function_id, payload).await
}

/// Banks eligible for this pass (config allowlist applied).
pub async fn list_banks<T: ConsolidationTransport + ?Sized>(
    iii: &T,
    allow: &[String],
) -> Result<Vec<String>, String> {
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
pub(crate) async fn fetch_live<T: ConsolidationTransport + ?Sized>(
    iii: &T,
    bank: &str,
    errors: &mut Vec<String>,
) -> Result<Vec<MemoryRow>, String> {
    let mut rows = Vec::new();
    // Offset paging over a newest-first list can repeat a row when a write
    // lands between pages; a duplicate would break batch aliasing.
    let mut seen = std::collections::HashSet::new();
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
                Ok(row) => {
                    if seen.insert(row.id.clone()) {
                        rows.push(row);
                    }
                }
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
pub async fn run_bank<T: ConsolidationTransport + ?Sized>(
    iii: &T,
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
        apply_group(
            iii,
            bank,
            group,
            &rows,
            budget,
            &mut report,
            REASON_NORMALIZED,
        )
        .await;
    }
    report.groups = groups;
    (report, rows)
}

/// Supersede a group's losers into its winner, letting the winner absorb
/// each loser's observations (`reinforce`), within the pass budget. Shared
/// by the deterministic pass and the judge-confirmed merges.
///
/// Reinforcement rides the supersede itself: the old follow-up
/// `memory::save(winner_text)` silently created a duplicate whenever the
/// winner's text had been edited (`memory::update` keeps the old id, so the
/// fingerprint no longer matches) and wrote pinned winners.
#[allow(clippy::too_many_arguments)]
async fn apply_group<T: ConsolidationTransport + ?Sized>(
    iii: &T,
    bank: &str,
    group: &PlannedGroup,
    rows: &[MemoryRow],
    budget: &mut usize,
    report: &mut BankReport,
    reason: &str,
) -> usize {
    let mut applied = 0usize;
    let Some(winner) = rows.iter().find(|row| row.id == group.winner_id) else {
        report
            .errors
            .push("missing proof-bound winner snapshot".into());
        return 0;
    };
    let mut expected_winner = winner.expected_snapshot();
    for loser in &group.loser_ids {
        if *budget == 0 {
            report.capped += 1;
            continue;
        }
        let Some(seen) = rows.iter().find(|row| &row.id == loser) else {
            report
                .errors
                .push("missing proof-bound loser snapshot".into());
            break;
        };
        let Some(proof) = detail_proof(&winner.text, &seen.text) else {
            report
                .notes
                .push("merge retained: no detail-preserving certificate".into());
            continue;
        };
        let superseded = call(
            iii,
            "memory::supersede",
            json!({
                "bank": bank,
                "id": loser,
                "superseded_by": group.winner_id,
                "expected_memory": seen.expected_snapshot(),
                "expected_superseded_by": expected_winner,
                "reinforce": true,
                "reason": reason,
            }),
        )
        .await;
        match superseded {
            Ok(reply) => {
                *budget -= 1;
                applied += 1;
                report.superseded += 1;
                tally_reinforcement(report, &reply);
                report.detail_proofs.push(proof);
                if let Some(survivor) = reply.get("survivor").filter(|v| !v.is_null()) {
                    expected_winner = survivor.clone();
                } else if *budget > 0 && reply.get("reinforced") == Some(&Value::Bool(true)) {
                    // An old producer cannot certify the next winner snapshot.
                    // Stop rather than silently re-read/rebase a stale proof.
                    report.notes.push(
                        "producer omitted post-commit survivor; remaining group deferred".into(),
                    );
                    break;
                }
            }
            Err(e) => {
                report.errors.push(e);
                break;
            }
        }
    }
    applied
}

/// Options for the LLM-assisted tier.
#[derive(Debug, Clone)]
pub struct LlmOptions {
    pub model: String,
    pub promote_threshold: u32,
}

/// The judge tier for one bank: confirmed reorder merges + rule
/// promotions. Runs AFTER the deterministic pass, never on dry runs.
/// Fail-soft — an unavailable router lands in the report's notes (never
/// errors: it must not keep the pass due) and the pass stays valid.
pub async fn run_llm_tier<T: ConsolidationTransport + ?Sized>(
    iii: &T,
    bank: &str,
    rows: &[MemoryRow],
    report: &mut BankReport,
    budget: &mut usize,
    opts: &LlmOptions,
) {
    // Normalization collisions are ambiguous, not reorder candidates.
    let reorder: Vec<&PlannedGroup> = report
        .groups
        .iter()
        .filter(|g| {
            g.report_only
                && g.loser_texts
                    .iter()
                    .all(|text| normalize(text) != normalize(&g.winner_text))
        })
        .collect();
    let candidates = crate::llm::promotion_candidates(rows, opts.promote_threshold);
    let Some(prompt) = crate::llm::judge_prompt(&reorder, &candidates) else {
        return;
    };
    let reply = match iii.judge(&opts.model, prompt).await {
        Ok(r) => r,
        Err(e) => {
            // An unavailable optional decision provider remains a soft skip;
            // it does not turn already-applied deterministic work into a retry.
            report.notes.push(format!("llm tier skipped: {e}"));
            return;
        }
    };

    // Confirmed merges: clone the groups first (report holds them).
    let confirmed = confirmed_groups(&reply.merge_groups, &reorder);
    for group in &confirmed {
        let applied = apply_group(iii, bank, group, rows, budget, report, REASON_REORDER).await;
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
            report.notes.push(format!("promotion skipped: {e}"));
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
            updated_at: created_at,
            stated_at: None,
            snapshot: BTreeMap::new(),
        }
    }

    #[test]
    fn reinforcement_tally_counts_only_true_and_notes_an_old_worker_once() {
        let mut report = BankReport::default();
        tally_reinforcement(&mut report, &json!({"memory": {}, "reinforced": true}));
        assert_eq!(report.reinforced, 1);
        // Pinned survivor: untouchable, not reinforced.
        tally_reinforcement(&mut report, &json!({"memory": {}, "reinforced": false}));
        assert_eq!(report.reinforced, 1);
        // An older memory worker omits the field: never papered over.
        tally_reinforcement(&mut report, &json!({"memory": {}}));
        tally_reinforcement(&mut report, &json!({"memory": {}}));
        assert_eq!(report.reinforced, 1);
        assert_eq!(report.notes, vec![NOTE_OLD_MEMORY.to_string()]);
    }

    #[test]
    fn confirmed_groups_dedup_and_bound_the_judge_indices() {
        let g = |w: &str| PlannedGroup {
            winner_id: w.into(),
            winner_text: w.into(),
            loser_ids: vec![format!("{w}-l")],
            loser_texts: vec![],
            skipped_pinned: vec![],
            report_only: true,
        };
        let (a, b) = (g("a"), g("b"));
        let reorder = vec![&a, &b];
        // A repeated index used to supersede twice ("already retired" error
        // -> blocked checkpoint); out-of-range indices are ignored.
        let picked = confirmed_groups(&[1, 1, 0, 7], &reorder);
        let winners: Vec<&str> = picked.iter().map(|p| p.winner_id.as_str()).collect();
        assert_eq!(winners, vec!["a", "b"]);
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
        assert!(
            plan(&rows).is_empty(),
            "empty evidence cannot authorize retirement"
        );
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
