//! The alias tier: one thing stored under two entity handles (`build cache`
//! and `build-cache`, `dentista` and `dentist`). The memories carrying the
//! alias are retagged in place through `memory::update`, so everything that
//! reads `entities` (recall, the hook's subjects) sees one handle.
//!
//! A pair of handles is a candidate only when every guard holds:
//! - neither handle is `user`, the speaker's fixed handle;
//! - no live memory carries both: two handles on one memory are two things;
//! - `spelling`: the same characters once whitespace, `-` and `_` are dropped,
//!   case aside (memory's write-time rule: `c++` and `c#` differ); or
//!   `near`: the same word count and digit runs, each word pair equal or at
//!   0.6 trigram similarity or more, and the whole handles at 0.6 or more;
//! - neither handle has a second candidate partner (then it merges with neither).
//!
//! `spelling` merges with no model call. `near` merges only after a judge
//! call per bank confirms it: no similarity floor tells `dentist`/`dentista`
//! from two people (`gabriel`/`gabriela` score the same). A cut or
//! unparseable reply halves its batch, as the semantic tier does. Verdicts
//! and merges live in the state worker (`aliases:{bank}`): a `same` entry
//! retags later stragglers with no call, a `distinct` one is never asked
//! again, and `memory-consolidate::alias-undo` puts a merged handle back.

use std::collections::{BTreeMap, HashSet, VecDeque};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::consolidate::{BankReport, ConsolidationTransport, MemoryRow, call};

/// The speaker's fixed handle: never merged into another, nor another into it.
const USER: &str = "user";
/// Trigram similarity floor of the `near` tier, per word and for the whole handle.
const NEAR: f64 = 0.6;
/// Pairs a pass asks the judge about; the rest wait for the next pass.
const MAX_PAIRS_PER_CALL: usize = 20;
/// Memory texts shown to the judge per handle, each cut to [`MAX_TEXT_CHARS`].
const TEXTS_PER_HANDLE: usize = 2;
const MAX_TEXT_CHARS: usize = 300;
/// The judge's reason is carried on memory's events; memory caps it at 500.
const MAX_REASON_CHARS: usize = 300;
/// Room for every verdict of a full batch: `n`, `same` and a reason of
/// [`MAX_REASON_CHARS`], about 120 tokens each.
const MAX_OUTPUT_TOKENS: u32 = 120 * MAX_PAIRS_PER_CALL as u32;
/// Handles a retag may send: an older memory worker keeps the first eight
/// of an update and drops the rest, `user` included (extraction writes eight
/// handles and then `user`), so a longer list is never sent.
const MAX_HANDLES: usize = 8;

const REASON_SPELLING: &str = "same letters, separators aside";
const REASON_TOGETHER: &str = "tagged together on one memory";
const REASON_UNDONE: &str = "alias merge undone";
const REASON_WAITING: &str = "waits for the judge";
const REASON_AMBIGUOUS: &str = "ambiguous: a handle of this pair has another candidate";
const REASON_UNPARSED: &str = "kept apart: no judge reply about this pair parsed";

/// Static system prompt of the near-spelling judge (cache-friendly).
pub const ALIAS_SYSTEM: &str = "You decide whether two entity handles of a user's memory bank \
name the same thing. Each numbered pair gives handles a and b and up to two memories tagged \
with each. Reply ONLY with a JSON object (no prose, no code fences):\n\
{\"pairs\": [{\"n\": <pair number>, \"same\": <true or false>, \"reason\": \"<one sentence>\"}]}\n\
One item per pair; every field is required.\n\
Rules:\n\
- same is true only when both handles name the very same person, project, place or tool: \
another spelling of it, its singular or plural, or another language's word for it.\n\
- Two people, two services, two versions, two places, or a product and one of its parts are \
different: same is false. Names a letter or two apart are often two people.\n\
- Similar spelling is not evidence: decide from what the memories say about each handle.\n\
- When unsure, same is false.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// The same characters, case, whitespace, `-` and `_` aside: merged with
    /// no model call.
    Spelling,
    /// A near spelling: merged only after the judge confirms it.
    Near,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Same,
    Distinct,
}

/// One ledger entry: a pair decided by the rule or the judge, and the
/// memories a `same` verdict retagged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub from: String,
    pub to: String,
    pub tier: Tier,
    pub verdict: Verdict,
    #[serde(default)]
    pub reason: String,
    /// When the entry was last decided (ms).
    pub ms: u64,
    /// Memories a merge retagged from `from` to `to`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<String>,
}

impl Entry {
    fn names(&self, a: &str, b: &str) -> bool {
        (self.from == a && self.to == b) || (self.from == b && self.to == a)
    }
}

/// One alias pair as reported.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AliasAction {
    /// The handle retagged away.
    pub from: String,
    /// The canonical handle: on more live memories, then the older, then the
    /// smaller string.
    pub to: String,
    pub tier: Tier,
    /// pg_trgm similarity of the two handles.
    pub similarity: f64,
    /// Memories were retagged this pass (never on a dry run).
    pub applied: bool,
    /// Memories retagged; on a dry run, the ones that would be.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<String>,
    /// Memories left alone because they are pinned.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped_pinned: Vec<String>,
    /// The rule, the judge's reason, or what the pair waits for.
    pub reason: String,
}

/// A candidate pair, canonical handle as `to`.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub from: String,
    pub to: String,
    pub tier: Tier,
    pub similarity: f64,
}

#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Candidates to merge (after the judge, for `near`).
    pub pairs: Vec<Candidate>,
    /// Similar handles one live memory carries together: two things.
    pub together: Vec<Candidate>,
    /// Candidates a handle of which has another candidate partner.
    pub ambiguous: Vec<Candidate>,
}

/// A memory row's entity handles.
pub fn entities(row: &MemoryRow) -> Vec<String> {
    row.snapshot
        .get("entities")
        .and_then(Value::as_array)
        .map(|handles| {
            handles
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// pg_trgm's words: lowercase runs of letters and digits.
fn words(handle: &str) -> Vec<String> {
    handle
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// pg_trgm's trigrams: each word padded with two blanks before and one after.
fn trigrams(words: &[String]) -> HashSet<String> {
    let mut out = HashSet::new();
    for word in words {
        let padded: Vec<char> = "  ".chars().chain(word.chars()).chain([' ']).collect();
        out.extend(padded.windows(3).map(|t| t.iter().collect::<String>()));
    }
    out
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    a.intersection(b).count() as f64 / union as f64
}

/// pg_trgm's `similarity` of two handles.
pub fn similarity(a: &str, b: &str) -> f64 {
    jaccard(&trigrams(&words(a)), &trigrams(&words(b)))
}

/// A handle without whitespace, `-` and `_`, lowercased: memory's write-time
/// snap uses the same rule, so `c++` and `c#` stay two spellings.
fn spelling(handle: &str) -> String {
    handle
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

/// What the rule reads of one handle, computed once per pass.
struct Shape {
    handle: String,
    words: Vec<String>,
    joined: String,
    digits: Vec<String>,
    grams: HashSet<String>,
}

impl Shape {
    fn of(handle: &str) -> Self {
        let words = words(handle);
        Shape {
            handle: handle.to_string(),
            joined: spelling(handle),
            digits: handle
                .split(|c: char| !c.is_numeric())
                .filter(|run| !run.is_empty())
                .map(str::to_string)
                .collect(),
            grams: trigrams(&words),
            words,
        }
    }
}

fn tier_of(a: &Shape, b: &Shape) -> Option<Tier> {
    if a.handle == USER || b.handle == USER || a.joined.is_empty() || b.joined.is_empty() {
        return None;
    }
    if a.joined == b.joined {
        return Some(Tier::Spelling);
    }
    let near = a.words.len() == b.words.len()
        && a.digits == b.digits
        && jaccard(&a.grams, &b.grams) >= NEAR
        && a.words.iter().zip(&b.words).all(|(x, y)| {
            x == y
                || jaccard(
                    &trigrams(std::slice::from_ref(x)),
                    &trigrams(std::slice::from_ref(y)),
                ) >= NEAR
        });
    near.then_some(Tier::Near)
}

/// The tier two handles may merge under; `None` when they never may.
pub fn tier(a: &str, b: &str) -> Option<Tier> {
    tier_of(&Shape::of(a), &Shape::of(b))
}

/// The alias plan of one bank's live memories. Pure and order-free.
pub fn plan(rows: &[MemoryRow]) -> Plan {
    // handle -> (live memories carrying it, oldest created_at among them)
    let mut handles: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    let tagged: Vec<Vec<String>> = rows.iter().map(entities).collect();
    for (row, tags) in rows.iter().zip(&tagged) {
        let unique: HashSet<&String> = tags.iter().collect();
        for handle in unique {
            let stat = handles.entry(handle.clone()).or_insert((0, u64::MAX));
            stat.0 += 1;
            stat.1 = stat.1.min(row.created_at);
        }
    }
    let shapes: Vec<Shape> = handles.keys().map(|h| Shape::of(h)).collect();
    // shortcut: every pair of handles is compared, fine at thousands of handles; bucket by word count and digits if a bank outgrows that.
    let mut candidates = Vec::new();
    let mut partners: BTreeMap<&str, usize> = BTreeMap::new();
    let mut out = Plan::default();
    for (i, a) in shapes.iter().enumerate() {
        for b in &shapes[i + 1..] {
            let Some(tier) = tier_of(a, b) else { continue };
            let key = |h: &str| (std::cmp::Reverse(handles[h].0), handles[h].1, h.to_string());
            let (to, from) = if key(&a.handle) <= key(&b.handle) {
                (&a.handle, &b.handle)
            } else {
                (&b.handle, &a.handle)
            };
            let pair = Candidate {
                from: from.clone(),
                to: to.clone(),
                tier,
                similarity: (jaccard(&a.grams, &b.grams) * 100.0).round() / 100.0,
            };
            if tagged.iter().any(|t| t.contains(from) && t.contains(to)) {
                out.together.push(pair);
            } else {
                *partners.entry(&a.handle).or_default() += 1;
                *partners.entry(&b.handle).or_default() += 1;
                candidates.push(pair);
            }
        }
    }
    for pair in candidates {
        if partners[pair.from.as_str()] > 1 || partners[pair.to.as_str()] > 1 {
            out.ambiguous.push(pair);
        } else {
            out.pairs.push(pair);
        }
    }
    out
}

/// `entities` with `from` replaced by `to`, order kept, repeats dropped.
pub fn retagged(entities: &[String], from: &str, to: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for handle in entities {
        let handle = if handle == from { to } else { handle };
        if !out.iter().any(|h| h == handle) {
            out.push(handle.to_string());
        }
    }
    out
}

/// The judge's verdicts by pair number. `None` when the reply is not the
/// JSON shape or answers a pair twice: it records nothing, and its pairs are
/// asked again.
pub fn parse_verdicts(text: &str) -> Option<BTreeMap<usize, (bool, String)>> {
    #[derive(Deserialize)]
    struct Item {
        n: usize,
        same: bool,
        reason: String,
    }
    #[derive(Deserialize)]
    struct Reply {
        pairs: Vec<Item>,
    }
    let reply: Reply = serde_json::from_str(crate::semantic::json_object(text).ok()?).ok()?;
    let mut out = BTreeMap::new();
    for item in reply.pairs {
        let reason: String = item.reason.trim().chars().take(MAX_REASON_CHARS).collect();
        if out.insert(item.n, (item.same, reason)).is_some() {
            return None; // one item per pair: a repeat is a verdict taken back
        }
    }
    Some(out)
}

/// The judge's user message: each pair with up to two memory texts per handle.
fn judge_prompt(pairs: &[Candidate], rows: &[MemoryRow]) -> String {
    let texts = |handle: &str| {
        let mut tagged: Vec<&MemoryRow> = rows
            .iter()
            .filter(|r| entities(r).iter().any(|h| h == handle))
            .collect();
        tagged.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        tagged
            .iter()
            .take(TEXTS_PER_HANDLE)
            .map(|r| r.text.chars().take(MAX_TEXT_CHARS).collect::<String>())
            .collect::<Vec<_>>()
    };
    let pairs: Vec<Value> = pairs
        .iter()
        .enumerate()
        .map(|(i, p)| {
            json!({
                "n": i + 1,
                "a": p.from,
                "a_memories": texts(&p.from),
                "b": p.to,
                "b_memories": texts(&p.to),
            })
        })
        .collect();
    json!({ "pairs": pairs }).to_string()
}

pub fn ledger_key(bank: &str) -> String {
    format!("aliases:{bank}")
}

/// The bank's ledger; `Err` when it cannot be read or decoded (never "empty").
async fn read_ledger<T: ConsolidationTransport + ?Sized>(
    iii: &T,
    bank: &str,
) -> Result<Vec<Entry>, String> {
    match crate::functions::state_try_get(iii, &ledger_key(bank)).await? {
        None => Ok(Vec::new()),
        Some(v) => serde_json::from_value(v).map_err(|e| format!("alias ledger undecodable: {e}")),
    }
}

pub struct AliasOptions {
    pub model: String,
    /// Model calls this bank may still make: the run's cap less what
    /// earlier banks used.
    pub max_llm_calls: usize,
    pub dry_run: bool,
}

fn action(pair: &Candidate, reason: &str) -> AliasAction {
    AliasAction {
        from: pair.from.clone(),
        to: pair.to.clone(),
        tier: pair.tier,
        similarity: pair.similarity,
        applied: false,
        ids: Vec::new(),
        skipped_pinned: Vec::new(),
        reason: reason.to_string(),
    }
}

/// The tier for one bank, on the deterministic pass's scan less the records
/// it retires. Each retag replaces its row in `rows` with the record memory
/// returned, so the tiers after it guard their writes with the new revision.
pub async fn run_alias_tier<T: ConsolidationTransport + ?Sized>(
    iii: &T,
    bank: &str,
    rows: &mut [MemoryRow],
    report: &mut BankReport,
    budget: &mut usize,
    opts: &AliasOptions,
) {
    let writable = report.groups.iter().filter(|g| !g.report_only);
    let retired: HashSet<&str> = writable
        .clone()
        .flat_map(|g| g.loser_ids.iter().map(String::as_str))
        .collect();
    // A merge winner was reinforced after the scan, so its snapshot is stale:
    // the next pass retags it.
    let reinforced: HashSet<String> = writable
        .filter(|_| !opts.dry_run)
        .map(|g| g.winner_id.clone())
        .collect();
    let live: Vec<MemoryRow> = rows
        .iter()
        .filter(|r| !retired.contains(r.id.as_str()))
        .cloned()
        .collect();
    let plan = plan(&live);
    if plan == Plan::default() {
        return;
    }
    if !report.errors.is_empty() {
        // A partial scan can miss the memory that tags both handles.
        report
            .notes
            .push("alias tier skipped: the bank had errors earlier in the pass".into());
        return;
    }
    let mut ledger = match read_ledger(iii, bank).await {
        Ok(ledger) => ledger,
        Err(e) => {
            report.notes.push(format!("alias tier skipped: {e}"));
            return;
        }
    };
    let read = ledger.clone();
    let now = crate::functions::now_ms();
    let entry = |pair: &Candidate, verdict, reason: &str| Entry {
        from: pair.from.clone(),
        to: pair.to.clone(),
        tier: pair.tier,
        verdict,
        reason: reason.to_string(),
        ms: now,
        ids: Vec::new(),
    };

    let mut merges: Vec<AliasAction> = Vec::new();
    let mut asks: Vec<Candidate> = Vec::new();
    for pair in &plan.pairs {
        match ledger.iter().find(|e| e.names(&pair.from, &pair.to)) {
            Some(e) if e.verdict == Verdict::Distinct => {}
            // A later straggler: the recorded direction, no call.
            Some(e) => merges.push(AliasAction {
                from: e.from.clone(),
                to: e.to.clone(),
                ..action(pair, &e.reason)
            }),
            None if pair.tier == Tier::Spelling => {
                ledger.push(entry(pair, Verdict::Same, REASON_SPELLING));
                merges.push(action(pair, REASON_SPELLING));
            }
            None => asks.push(pair.clone()),
        }
    }
    for pair in &plan.together {
        if !ledger.iter().any(|e| e.names(&pair.from, &pair.to)) {
            ledger.push(entry(pair, Verdict::Distinct, REASON_TOGETHER));
        }
    }

    // Verdicts by index into `asks`. As in the semantic tier, a cut or
    // unparseable reply halves its batch; a pair that still fails alone is
    // kept apart once another reply parsed (the judge works, this pair is
    // what it cannot answer), else asked again on the next pass.
    let asked = asks.len().min(MAX_PAIRS_PER_CALL);
    let calls = report.llm_calls;
    let mut verdicts: BTreeMap<usize, (bool, String)> = BTreeMap::new();
    let (mut unparsed, mut parsed) = (Vec::new(), false);
    let mut queue = VecDeque::new();
    if asked > 0 && !opts.dry_run {
        queue.push_back(0..asked);
    }
    while let Some(group) = queue.pop_front() {
        if report.llm_calls >= opts.max_llm_calls {
            report
                .notes
                .push("alias judge deferred: per-run LLM call cap reached".into());
            break;
        }
        report.llm_calls += 1;
        let result = iii
            .complete(
                &opts.model,
                ALIAS_SYSTEM,
                judge_prompt(&asks[group.clone()], &live),
                MAX_OUTPUT_TOKENS,
            )
            .await;
        if let Ok(reply) = &result {
            crate::consolidate::tally_usage(report, &reply["usage"]);
        }
        let reply = match crate::semantic::classify(&result) {
            crate::semantic::Outcome::Reply(text) => parse_verdicts(&text),
            crate::semantic::Outcome::InputShaped(_) => None,
            crate::semantic::Outcome::Transport(e) => {
                report.notes.push(format!("alias judge skipped: {e}"));
                break;
            }
        };
        match reply {
            Some(found) => {
                parsed = true;
                for (n, verdict) in found
                    .into_iter()
                    .filter(|(n, _)| (1..=group.len()).contains(n))
                {
                    verdicts.insert(group.start + n - 1, verdict);
                }
            }
            None if group.len() > 1 => {
                let mid = group.start + group.len() / 2;
                queue.push_front(mid..group.end);
                queue.push_front(group.start..mid);
            }
            None => unparsed.push(group.start),
        }
    }
    if !parsed && !unparsed.is_empty() {
        report
            .notes
            .push("alias judge reply did not parse; its pairs are asked again".into());
    }
    if report.llm_calls > calls && asks.len() > asked {
        report.notes.push(format!(
            "alias judge: {} pairs wait for the next pass",
            asks.len() - asked
        ));
    }
    let mut waiting = Vec::new();
    for (i, pair) in asks.iter().enumerate() {
        let kept_apart = (parsed && unparsed.contains(&i)).then(|| (false, REASON_UNPARSED.into()));
        match verdicts.remove(&i).or(kept_apart) {
            Some((true, why)) => {
                ledger.push(entry(pair, Verdict::Same, &why));
                merges.push(action(pair, &why));
            }
            Some((false, why)) => {
                ledger.push(entry(pair, Verdict::Distinct, &why));
                waiting.push(action(pair, &why));
            }
            None => waiting.push(action(pair, REASON_WAITING)),
        }
    }
    waiting.extend(plan.ambiguous.iter().map(|p| action(p, REASON_AMBIGUOUS)));

    // The memories each merge retags: live, unpinned, within the cap.
    let mut left = if opts.dry_run { usize::MAX } else { *budget };
    for merge in &mut merges {
        let mut too_many = Vec::new();
        for row in live.iter().filter(|r| entities(r).contains(&merge.from)) {
            if row.pinned {
                merge.skipped_pinned.push(row.id.clone());
            } else if reinforced.contains(&row.id) {
                continue;
            } else if retagged(&entities(row), &merge.from, &merge.to).len() > MAX_HANDLES {
                too_many.push(row.id.clone());
            } else if left == 0 {
                report.capped += 1;
            } else {
                left -= 1;
                merge.ids.push(row.id.clone());
            }
        }
        if !too_many.is_empty() {
            report.notes.push(format!(
                "alias {} -> {}: {} left alone: more than {MAX_HANDLES} handles",
                merge.from,
                merge.to,
                too_many.join(", ")
            ));
        }
        if let Some(e) = ledger.iter_mut().find(|e| e.names(&merge.from, &merge.to)) {
            for id in &merge.ids {
                if !e.ids.contains(id) {
                    e.ids.push(id.clone());
                }
            }
        }
    }
    if opts.dry_run {
        report.aliases.extend(merges.into_iter().chain(waiting));
        return;
    }
    // Written before the retags, so an undo always finds what they touch; a
    // recorded memory a retag then missed still carries `from` and is skipped.
    if ledger != read
        && let Err(e) = crate::functions::state_set(iii, &ledger_key(bank), json!(ledger)).await
    {
        report
            .notes
            .push(format!("alias tier stopped: ledger not saved: {e}"));
        for merge in &mut merges {
            merge.ids.clear();
        }
        report.aliases.extend(merges.into_iter().chain(waiting));
        return;
    }
    let mut failed = false;
    for merge in &mut merges {
        let planned = std::mem::take(&mut merge.ids);
        for id in planned {
            if failed {
                break;
            }
            let Some(pos) = rows.iter().position(|r| r.id == id) else {
                continue;
            };
            let payload = json!({
                "bank": bank,
                "id": id,
                "entities": retagged(&entities(&rows[pos]), &merge.from, &merge.to),
                "expected_memory": rows[pos].expected_snapshot(),
                "reason": format!("alias: {} -> {} ({})", merge.from, merge.to, merge.reason),
            });
            match call(iii, "memory::update", payload).await {
                Ok(reply) => {
                    *budget = budget.saturating_sub(1);
                    if let Some(row) = reply
                        .get("memory")
                        .and_then(|m| serde_json::from_value::<MemoryRow>(m.clone()).ok())
                    {
                        rows[pos] = row;
                    }
                    merge.ids.push(id);
                }
                Err(e) => {
                    report.errors.push(e);
                    failed = true;
                }
            }
        }
        merge.applied = !merge.ids.is_empty();
    }
    report.aliases.extend(merges.into_iter().chain(waiting));
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UndoRequest {
    /// Bank of the merge.
    pub bank: String,
    /// The handle the merge retagged away.
    pub from: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UndoResponse {
    pub bank: String,
    pub from: String,
    /// The handle the merge had retagged to.
    pub to: String,
    /// Memories given `from` back.
    pub restored: Vec<String>,
    /// Recorded memories left alone: gone, retired, pinned, no longer
    /// carrying `to`, already carrying `from`, or carrying more than eight
    /// handles.
    pub skipped: Vec<String>,
}

/// Undo the bank's merge of `from`: the ledger entry turns `distinct` first,
/// so no pass merges the pair again, then each recorded memory that is live,
/// unpinned, still carries `to` and does not already carry `from` gets
/// `from` back. A failed write leaves the rest for a second call.
pub async fn undo<T: ConsolidationTransport + ?Sized>(
    iii: &T,
    req: UndoRequest,
) -> Result<UndoResponse, String> {
    let bank = req.bank;
    let mut ledger = read_ledger(iii, &bank).await?;
    // No pass merges `user`, so an entry naming it was written by hand.
    let at = ledger
        .iter()
        .rposition(|e| {
            e.from == req.from
                && e.from != USER
                && e.to != USER
                && (e.verdict == Verdict::Same || !e.ids.is_empty())
        })
        .ok_or_else(|| format!("no alias merge from `{}` in bank `{bank}`", req.from))?;
    let entry = &mut ledger[at];
    entry.verdict = Verdict::Distinct;
    entry.reason = REASON_UNDONE.into();
    entry.ms = crate::functions::now_ms();
    let entry = entry.clone();
    crate::functions::state_set(iii, &ledger_key(&bank), json!(ledger)).await?;
    let (mut restored, mut skipped) = (Vec::new(), Vec::new());
    for id in &entry.ids {
        let memory = match call(iii, "memory::get", json!({ "bank": bank, "id": id })).await {
            Ok(reply) => reply.get("memory").cloned().unwrap_or(Value::Null),
            Err(e) if e.contains("memory_not_found") => Value::Null,
            Err(e) => return Err(e),
        };
        let row = serde_json::from_value::<MemoryRow>(memory.clone()).ok();
        let Some(row) = row.filter(|r| {
            memory.get("invalid_at").is_none_or(Value::is_null)
                && !r.pinned
                && entities(r).contains(&entry.to)
                // With both handles, the merge or a person may have added `to`: drop neither.
                && !entities(r).contains(&entry.from)
                && retagged(&entities(r), &entry.to, &entry.from).len() <= MAX_HANDLES
        }) else {
            skipped.push(id.clone());
            continue;
        };
        let payload = json!({
            "bank": bank,
            "id": id,
            "entities": retagged(&entities(&row), &entry.to, &entry.from),
            "expected_memory": memory,
            "reason": format!("{REASON_UNDONE}: {} -> {}", entry.to, entry.from),
        });
        call(iii, "memory::update", payload).await?;
        restored.push(id.clone());
    }
    Ok(UndoResponse {
        bank,
        from: entry.from,
        to: entry.to,
        restored,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, handles: &[&str], created_at: u64) -> MemoryRow {
        MemoryRow {
            id: id.into(),
            text: format!("memory {id}"),
            pinned: false,
            corroboration: 0,
            created_at,
            updated_at: created_at,
            stated_at: None,
            snapshot: [("entities".to_string(), json!(handles))].into(),
        }
    }

    fn pairs(plan: &Plan) -> Vec<(&str, &str, Tier)> {
        plan.pairs
            .iter()
            .map(|p| (p.from.as_str(), p.to.as_str(), p.tier))
            .collect()
    }

    #[test]
    fn similarity_matches_pg_trgm() {
        let two = |a, b| (similarity(a, b) * 100.0).round() / 100.0;
        assert_eq!(two("dentist", "dentista"), 0.70);
        assert_eq!(two("gabriel", "gabriela"), 0.70);
        assert_eq!(two("payments-api", "ledger-api"), 0.20);
        assert_eq!(two("postgres", "postgresql"), 0.67);
        assert_eq!(two("payments-api", "payments api"), 1.00);
        assert_eq!(two("payments-api", "payment-api"), 0.79);
        assert_eq!(similarity("ledger", "ledgerly"), 0.6, "on the floor");
    }

    #[test]
    fn sibling_handles_are_never_candidates() {
        for (a, b) in [
            ("payments-api", "ledger-api"),
            ("payments-api", "payments-worker"),
            ("billing-db", "ledger-db"),
            (
                "analytics-export-original-plan",
                "analytics-export-corrected-plan",
            ),
            ("us-east", "us-west"),
            ("us-east-1", "us-east-2"),
            ("pg15", "pg16"),
            ("python", "python3"),
            ("john smith", "jane smith"),
            ("ledgerly", "ledgerly api"),
            ("github", "github actions"),
            ("user", "usuário"),
            ("user", "users"),
            ("user", "us-er"),
            ("júlia", "julia"),
            ("dana", "kai"),
        ] {
            assert_eq!(tier(a, b), None, "{a} / {b}");
            assert_eq!(tier(b, a), None, "{b} / {a}");
        }
    }

    #[test]
    fn separator_variants_are_spelling() {
        for (a, b) in [
            ("build cache", "build-cache"),
            ("payments-api", "payments api"),
            ("vs code", "vscode"),
            ("billing_webhook", "billing webhook"),
        ] {
            assert_eq!(tier(a, b), Some(Tier::Spelling), "{a} / {b}");
        }
    }

    #[test]
    fn symbols_and_dots_are_never_separators() {
        // Only whitespace, `-` and `_` are, as in memory's write-time snap.
        for (a, b) in [
            ("c++", "c#"),
            ("c", "c++"),
            ("java 1.8", "java 18"),
            ("angular 1.5", "angular 15"),
        ] {
            assert_ne!(tier(a, b), Some(Tier::Spelling), "{a} / {b}");
        }
        // Their words match, so only the judge may merge them.
        assert_eq!(tier("c++", "c#"), Some(Tier::Near));
        assert_eq!(tier("c", "c++"), Some(Tier::Near));
        // Other word counts and digit runs: never candidates.
        assert_eq!(tier("java 1.8", "java 18"), None);
        assert_eq!(tier("angular 1.5", "angular 15"), None);
    }

    #[test]
    fn the_judge_batch_fits_its_output_budget() {
        // Every verdict with a reason at its cap, at a conservative three
        // characters per token.
        let item = json!({"n": MAX_PAIRS_PER_CALL, "same": false,
            "reason": "x".repeat(MAX_REASON_CHARS)});
        let reply = json!({ "pairs": vec![item; MAX_PAIRS_PER_CALL] }).to_string();
        assert!(
            reply.len() / 3 <= MAX_OUTPUT_TOKENS as usize,
            "{}",
            reply.len()
        );
    }

    #[test]
    fn near_spellings_are_candidates_for_the_judge() {
        // gabriel/gabriela, daniel/daniela and ledger/ledgerly are two things
        // that score like a true alias: why a near pair waits for the judge.
        for (a, b) in [
            ("dentist", "dentista"),
            ("postgres", "postgresql"),
            ("payments-api", "payment-api"),
            ("gabriel", "gabriela"),
            ("daniel", "daniela"),
            ("adrian", "adriana"),
            ("ledger", "ledgerly"),
        ] {
            assert_eq!(tier(a, b), Some(Tier::Near), "{a} / {b}");
        }
    }

    #[test]
    fn handles_tagged_together_are_distinct() {
        let rows = vec![
            row("a", &["build cache", "build-cache"], 1),
            row("b", &["build-cache"], 2),
        ];
        let plan = plan(&rows);
        assert!(plan.pairs.is_empty());
        assert_eq!(plan.together.len(), 1);
        assert_eq!(plan.together[0].from, "build cache");
    }

    #[test]
    fn a_handle_with_two_partners_merges_with_neither() {
        let rows = vec![
            row("a", &["gabriel"], 1),
            row("b", &["gabriela"], 2),
            row("c", &["gabriell"], 3),
            row("d", &["build cache"], 4),
            row("e", &["build-cache"], 5),
        ];
        let plan = plan(&rows);
        assert_eq!(
            pairs(&plan),
            [("build-cache", "build cache", Tier::Spelling)]
        );
        let ambiguous: Vec<(&str, &str)> = plan
            .ambiguous
            .iter()
            .map(|p| (p.from.as_str(), p.to.as_str()))
            .collect();
        assert_eq!(
            ambiguous,
            [
                ("gabriela", "gabriel"),
                ("gabriell", "gabriel"),
                ("gabriell", "gabriela")
            ]
        );
    }

    #[test]
    fn the_more_used_handle_is_canonical() {
        let rows = vec![
            row("a", &["build cache"], 1),
            row("b", &["build-cache"], 2),
            row("c", &["build-cache"], 3),
        ];
        assert_eq!(
            pairs(&plan(&rows)),
            [("build cache", "build-cache", Tier::Spelling)]
        );
        // A tie: the handle on the older memory.
        let rows = vec![row("a", &["build-cache"], 2), row("b", &["build cache"], 1)];
        assert_eq!(
            pairs(&plan(&rows)),
            [("build-cache", "build cache", Tier::Spelling)]
        );
        // Same count, same age: the smaller string.
        let rows = vec![row("a", &["build-cache"], 1), row("b", &["build cache"], 1)];
        assert_eq!(
            pairs(&plan(&rows)),
            [("build-cache", "build cache", Tier::Spelling)]
        );
    }

    #[test]
    fn the_plan_ignores_row_order() {
        let mut rows = vec![
            row("a", &["user", "dentista"], 5),
            row("b", &["dentist"], 1),
            row("c", &["dentist", "user"], 3),
            row("d", &["build cache"], 4),
            row("e", &["build-cache", "payments-api"], 2),
            row("f", &["ledger-api"], 6),
        ];
        let forward = plan(&rows);
        rows.reverse();
        assert_eq!(plan(&rows), forward);
        assert_eq!(
            pairs(&forward),
            [
                ("build cache", "build-cache", Tier::Spelling),
                ("dentista", "dentist", Tier::Near),
            ]
        );
    }

    #[test]
    fn a_retag_keeps_order_and_drops_repeats() {
        let tags = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            retagged(
                &tags(&["user", "dentista", "clinic"]),
                "dentista",
                "dentist"
            ),
            tags(&["user", "dentist", "clinic"])
        );
        assert_eq!(
            retagged(&tags(&["dentist", "dentista"]), "dentista", "dentist"),
            tags(&["dentist"])
        );
    }

    #[test]
    fn a_reply_that_is_not_the_json_shape_gives_no_verdict() {
        let ok = parse_verdicts(
            "```json\n{\"pairs\":[{\"n\":1,\"same\":true,\"reason\":\"one dentist\"}]}\n```",
        )
        .unwrap();
        assert_eq!(ok[&1], (true, "one dentist".to_string()));
        for bad in [
            "no json here",
            "{}",
            "{\"pairs\":{}}",
            // A pair answered twice: the judge took one verdict back.
            "{\"pairs\":[{\"n\":1,\"same\":true,\"reason\":\"first guess\"},\
             {\"n\":1,\"same\":false,\"reason\":\"two people\"}]}",
            "{\"pairs\":[{\"n\":1,\"same\":\"yes\",\"reason\":\"r\"}]}",
            "{\"pairs\":[{\"n\":1,\"same\":true}]}",
            "{\"pairs\":[{\"same\":true,\"reason\":\"r\"}]}",
        ] {
            assert!(parse_verdicts(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_judge_sees_both_handles_and_two_texts_each() {
        let rows = vec![
            row("a", &["dentista"], 3),
            row("b", &["dentista"], 1),
            row("c", &["dentista"], 2),
            row("d", &["dentist"], 4),
        ];
        let pair = Candidate {
            from: "dentista".into(),
            to: "dentist".into(),
            tier: Tier::Near,
            similarity: 0.7,
        };
        let prompt: Value = serde_json::from_str(&judge_prompt(&[pair], &rows)).unwrap();
        assert_eq!(
            prompt,
            json!({"pairs":[{"n":1,"a":"dentista","a_memories":["memory b","memory c"],
                "b":"dentist","b_memories":["memory d"]}]})
        );
    }
}
