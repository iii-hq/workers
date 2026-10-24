//! Recall-quality eval (MOT-5383): a synthetic golden bank and queries scored
//! through the production `Bank::recall`, in `lexical` (no query vector) and
//! `hybrid` (cached embeddings) modes, sliced by category and language.
//!
//! Design follows Hindsight's retrieval eval
//! (`hindsight-system-evals/hindsight_system_evals/retrieval_metrics.py`):
//! trec_eval-style linear-gain nDCG, Recall over all relevant, MRR/Hit at
//! cutoffs, macro averages, a retrieval-vs-ranking blame per query, and a
//! trap count that is reported on its own, never averaged in. Two
//! deliberate differences: vectors are CACHED (Hindsight re-embeds with a
//! local model and its runs drift between two states) so this runs
//! deterministically as a PR gate, and a trap is deterministic — a stale
//! (superseded) memory ranking above the current one — where Hindsight
//! LLM-judges answers.
//!
//! - Report: `cargo test --test recall_quality -- --nocapture`
//! - Re-baseline after an intended scorer change: `UPDATE_GOLDENS=1 cargo
//!   test --test recall_quality`, then review the
//!   `tests/golden/recall_quality.json` diff.
//! - Re-embed after editing `tests/recall_quality/golden.json` (needs a live
//!   engine with an embed provider; `III_URL` picks it): `III_NAMESPACE=<ns>
//!   cargo test --test recall_quality -- --ignored --exact embed_golden_set`,
//!   and against an engine running iii-directory, `-- --ignored --exact
//!   embed_golden_set_minilm`. Only texts without a vector are embedded.
//!
//! The gate runs once per embedder: text-embedding-3-small, and the local
//! MiniLM that serves stacks without an embedding API. Semantic floors are
//! per model, so each one needs its own baseline.
//!
//! `injection_quality` scores what the injection hook sends instead: each
//! query and each negative (a message no memory answers) through
//! `recall_evidence` and `hooks::select` on one bank, without rules, budget
//! or layers, into its own `tests/golden/injection_quality*.json`.

mod support;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use memory::config::WorkerConfig;
use memory::store::{Store, STOPWORDS};
use memory::types::{fingerprint, now_ms, Confidence, Memory};
use serde::{Deserialize, Serialize};

/// Embedding model the cached vectors come from, and the prefix of it kept.
/// text-embedding-3-small is Matryoshka-trained, so a prefix stays a usable
/// embedding at a fraction of the fixture size.
const EMBED_MODEL: &str = "text-embedding-3-small";
const EMBED_DIMS: usize = 256;
/// iii-directory's `provider::minilm::embed`; not Matryoshka, so kept whole.
const MINILM_MODEL: &str = "sentence-transformers/all-MiniLM-L6-v2";
const MINILM_DIMS: usize = 384;
/// Results scored per query (the cutoff of every @10 metric).
const DEPTH: usize = 10;
const DAY_MS: u64 = 86_400_000;

#[derive(Deserialize)]
struct Golden {
    memories: Vec<GoldenMemory>,
    queries: Vec<GoldenQuery>,
    #[serde(default)]
    negatives: Vec<Negative>,
}

/// A message no memory answers: whatever it injects outside `related` is
/// noise. Scored by `injection_quality` only.
#[derive(Deserialize)]
struct Negative {
    id: String,
    text: String,
    lang: String,
    /// `greeting`, `first-person`, `incidental-word` or `near-topic`.
    kind: String,
    /// `dev` (tune on it) or `test` (held out).
    split: String,
    /// Memories a reasonable assistant could use: injecting them is clean.
    #[serde(default)]
    related: Vec<String>,
    /// The words it shares with the bank on purpose, each with the memories
    /// holding it.
    #[serde(default)]
    shares: BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize)]
struct GoldenMemory {
    key: String,
    text: String,
    #[serde(default)]
    entities: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    corroboration: u32,
    age_days: u64,
    /// This memory paraphrases another (a duplicate consolidation should
    /// merge); both in one top-10 wastes a recall slot.
    #[serde(default)]
    dup_of: Option<String>,
}

/// `tests/recall_quality/consolidation.json`: the semantic consolidation
/// tier's decisions on this golden bank (recorded from a live judge run by
/// memory-consolidate's ignored `judge_golden_cases` test), applied in the
/// `hybrid-consolidated` mode so the gate pins what consolidation buys.
#[derive(Deserialize)]
struct Consolidation {
    decisions: Vec<Decision>,
}

#[derive(Deserialize)]
struct Decision {
    /// `merge` (survivor absorbs the retired record) or `supersede`.
    kind: String,
    keep: String,
    retire: String,
}

#[derive(Deserialize)]
struct GoldenQuery {
    id: String,
    text: String,
    lang: String,
    category: String,
    /// Memory keys that answer the query (binary relevance).
    expect: Vec<String>,
    /// Superseded memories that must not outrank the current answer.
    #[serde(default)]
    stale: Vec<String>,
}

/// `tests/recall_quality/vectors.json`: one cached embedding per memory key and query
/// id, stamped with the text it embeds so an edited text fails loudly.
#[derive(Serialize, Deserialize)]
struct Vectors {
    model: String,
    dims: usize,
    items: BTreeMap<String, Embedded>,
}

#[derive(Serialize, Deserialize)]
struct Embedded {
    text: String,
    v: Vec<f32>,
}

fn eval_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/recall_quality")
}

fn load_golden() -> Golden {
    let raw = std::fs::read_to_string(eval_dir().join("golden.json")).expect("golden.json");
    serde_json::from_str(&raw).expect("golden.json parses")
}

/// Fixture integrity, checked before any scoring so a broken fixture never
/// reads as a quality change: unique keys, ids and texts, every
/// `expect`/`stale`/`related` key present, every `shares` word in the
/// memories it names, and no `first-person` negative sharing a content word
/// with the bank, so only the pronoun can pull a memory in.
fn validate(golden: &Golden) {
    let mut keys = HashSet::new();
    let mut texts = HashSet::new();
    for m in &golden.memories {
        assert!(
            keys.insert(m.key.as_str()),
            "duplicate memory key {}",
            m.key
        );
        assert!(
            texts.insert(fingerprint(&m.text)),
            "duplicate memory text {}",
            m.key
        );
    }
    for m in &golden.memories {
        if let Some(of) = &m.dup_of {
            assert!(keys.contains(of.as_str()), "{} dup_of unknown {of}", m.key);
        }
    }
    let mut ids = HashSet::new();
    for q in &golden.queries {
        assert!(ids.insert(q.id.as_str()), "duplicate query id {}", q.id);
        assert!(!q.expect.is_empty(), "{} expects nothing", q.id);
        // A repeated key would skew the recall/nDCG denominators and the
        // trap check; a key both current and stale is a contradiction.
        let mut named = HashSet::new();
        for k in q.expect.iter().chain(&q.stale) {
            assert!(keys.contains(k.as_str()), "{} names unknown key {k}", q.id);
            assert!(named.insert(k.as_str()), "{} names {k} twice", q.id);
        }
    }
    // Each memory's tokens as the index holds them: text and entities.
    let tokens: HashMap<&str, HashSet<String>> = golden
        .memories
        .iter()
        .map(|m| {
            let indexed = format!("{} {}", m.text, m.entities.join(" "));
            (
                m.key.as_str(),
                memory::index::tokenize(&indexed).into_iter().collect(),
            )
        })
        .collect();
    for n in &golden.negatives {
        assert!(ids.insert(n.id.as_str()), "duplicate query id {}", n.id);
        for k in &n.related {
            assert!(keys.contains(k.as_str()), "{} names unknown key {k}", n.id);
        }
        for (word, named) in &n.shares {
            for k in named {
                assert!(
                    tokens.get(k.as_str()).is_some_and(|t| t.contains(word)),
                    "{} shares {word:?} with {k}, which does not hold it",
                    n.id
                );
            }
        }
        if n.kind == "first-person" {
            let shared: Vec<String> = memory::index::tokenize(&n.text)
                .into_iter()
                .filter(|t| !STOPWORDS.contains(&t.as_str()))
                .filter(|t| tokens.values().any(|m| m.contains(t)))
                .collect();
            assert!(
                shared.is_empty(),
                "first-person {} shares {shared:?} with the bank",
                n.id
            );
        }
    }
}

// ---- metrics (binary relevance, trec_eval conventions) -------------------

/// Per-query outcome over the top-`DEPTH` ranked keys.
#[derive(Debug, Clone, PartialEq)]
struct Outcome {
    /// 1-based rank of the best-placed relevant key, if any came back.
    first_hit: Option<usize>,
    recall: f64,
    ndcg: f64,
    /// A stale key ranks above the current answer (or came back without it).
    trap: bool,
    /// Both members of a duplicate pair landed in the top `DEPTH`.
    dup_polluted: bool,
}

impl Outcome {
    fn rr(&self) -> f64 {
        self.first_hit.map_or(0.0, |r| 1.0 / r as f64)
    }

    /// Hindsight's blame: `retrieval` when a relevant key never came back,
    /// `ranking` when all did but the best one is not first.
    fn blame(&self) -> &'static str {
        if self.recall < 1.0 {
            "retrieval"
        } else if self.first_hit != Some(1) {
            "ranking"
        } else {
            "ok"
        }
    }
}

fn score(ranked: &[&str], relevant: &[String], stale: &[String]) -> Outcome {
    let ranked = &ranked[..ranked.len().min(DEPTH)];
    let is_rel = |k: &str| relevant.iter().any(|r| r == k);
    let rank_of = |set: &[String]| {
        ranked
            .iter()
            .position(|k| set.iter().any(|s| s == k))
            .map(|i| i + 1)
    };
    let first_hit = rank_of(relevant);
    let found = ranked.iter().filter(|k| is_rel(k)).count();
    // Linear gain, log2(rank + 1) discount — trec_eval's ndcg_cut.
    let dcg: f64 = ranked
        .iter()
        .enumerate()
        .filter(|(_, k)| is_rel(k))
        .map(|(i, _)| 1.0 / ((i + 2) as f64).log2())
        .sum();
    let idcg: f64 = (0..relevant.len().min(DEPTH))
        .map(|i| 1.0 / ((i + 2) as f64).log2())
        .sum();
    // Measured against the CURRENT answer (`expect[0]`), as Hindsight's
    // stale-vs-current check: another relevant memory ranking first must
    // not mask a superseded fact sitting above the current one.
    let trap = match (rank_of(stale), rank_of(&relevant[..1])) {
        (Some(s), Some(c)) => s < c,
        (Some(_), None) => true,
        (None, _) => false,
    };
    Outcome {
        first_hit,
        recall: found as f64 / relevant.len() as f64,
        ndcg: if idcg > 0.0 { dcg / idcg } else { 0.0 },
        trap,
        dup_polluted: false,
    }
}

/// True when both members of any duplicate pair are in the top `DEPTH`.
fn dup_polluted(ranked: &[&str], pairs: &[(&str, &str)]) -> bool {
    let top = &ranked[..ranked.len().min(DEPTH)];
    pairs
        .iter()
        .any(|(a, b)| top.contains(a) && top.contains(b))
}

/// Macro-averaged slice summary; traps are a count, never averaged in.
#[derive(Serialize, Default)]
struct Summary {
    queries: usize,
    hit_at_1: f64,
    hit_at_3: f64,
    mrr_at_10: f64,
    ndcg_at_10: f64,
    recall_at_10: f64,
    trap_queries: usize,
    traps: usize,
    /// Queries whose top 10 holds both members of a duplicate pair.
    dup_polluted: usize,
}

fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

fn summarize(outcomes: &[(&GoldenQuery, Outcome)]) -> Summary {
    let n = outcomes.len() as f64;
    let mean =
        |f: &dyn Fn(&Outcome) -> f64| round4(outcomes.iter().map(|(_, o)| f(o)).sum::<f64>() / n);
    Summary {
        queries: outcomes.len(),
        hit_at_1: mean(&|o| f64::from(u8::from(o.first_hit == Some(1)))),
        hit_at_3: mean(&|o| f64::from(u8::from(o.first_hit.is_some_and(|r| r <= 3)))),
        mrr_at_10: mean(&|o| o.rr()),
        ndcg_at_10: mean(&|o| o.ndcg),
        recall_at_10: mean(&|o| o.recall),
        trap_queries: outcomes.iter().filter(|(q, _)| !q.stale.is_empty()).count(),
        traps: outcomes.iter().filter(|(_, o)| o.trap).count(),
        dup_polluted: outcomes.iter().filter(|(_, o)| o.dup_polluted).count(),
    }
}

fn load_consolidation(golden: &Golden) -> Consolidation {
    let raw = std::fs::read_to_string(eval_dir().join("consolidation.json")).expect(
        "tests/recall_quality/consolidation.json missing: record it with memory-consolidate's \
         ignored judge_golden_cases test",
    );
    let c: Consolidation = serde_json::from_str(&raw).expect("consolidation.json parses");
    let keys: HashSet<&str> = golden.memories.iter().map(|m| m.key.as_str()).collect();
    for d in &c.decisions {
        assert!(
            matches!(d.kind.as_str(), "merge" | "supersede"),
            "unknown decision kind {}",
            d.kind
        );
        assert!(
            keys.contains(d.keep.as_str()) && keys.contains(d.retire.as_str()),
            "decision names unknown keys {} / {}",
            d.keep,
            d.retire
        );
    }
    c
}

/// Where each retired key's relevance goes after consolidation: follow the
/// supersede chain to the live survivor. A merged duplicate is no longer a
/// findable answer — its survivor is — and a retired stale record can no
/// longer trap, so the consolidated mode scores the expectations remapped.
fn survivor_of(c: &Consolidation) -> HashMap<&str, &str> {
    let next: HashMap<&str, &str> = c
        .decisions
        .iter()
        .map(|d| (d.retire.as_str(), d.keep.as_str()))
        .collect();
    next.keys()
        .map(|&k| {
            let mut s = k;
            while let Some(&n) = next.get(s) {
                s = n;
            }
            (k, s)
        })
        .collect()
}

/// `expect` with retired keys replaced by their survivors (deduped, order
/// kept) and `stale` without retired keys.
fn remap(q: &GoldenQuery, survivors: &HashMap<&str, &str>) -> (Vec<String>, Vec<String>) {
    let mut expect: Vec<String> = Vec::new();
    for k in &q.expect {
        let s = survivors.get(k.as_str()).copied().unwrap_or(k).to_string();
        if !expect.contains(&s) {
            expect.push(s);
        }
    }
    let stale = q
        .stale
        .iter()
        .filter(|k| !survivors.contains_key(k.as_str()))
        .cloned()
        .collect();
    (expect, stale)
}

/// Apply recorded consolidation decisions with `memory::supersede`'s
/// semantics: retire with a pointer; a merge reinforces a non-pinned
/// survivor (corroboration s + r + 1, latest updated_at).
async fn apply_consolidation(bank: &memory::store::Bank, golden: &Golden, c: &Consolidation) {
    let id_of: HashMap<&str, String> = golden
        .memories
        .iter()
        .map(|m| (m.key.as_str(), fingerprint(&m.text)))
        .collect();
    let now = now_ms();
    for d in &c.decisions {
        let mut retire = bank.get(&id_of[d.retire.as_str()]).await.unwrap();
        let mut keep = bank.get(&id_of[d.keep.as_str()]).await.unwrap();
        let (r_updated, r_corroboration, r_stated) =
            (retire.updated_at, retire.corroboration, retire.stated_at);
        retire.invalid_at = Some(now);
        retire.superseded_by = Some(keep.id.clone());
        retire.updated_at = now;
        retire.revision += 1;
        bank.commit(retire).await.unwrap();
        if d.kind == "merge" && !keep.pinned {
            keep.corroboration += r_corroboration + 1;
            keep.updated_at = keep.updated_at.max(r_updated);
            keep.stated_at = keep.stated_at.max(r_stated);
            keep.revision += 1;
            bank.commit(keep).await.unwrap();
        }
    }
}

// ---- the gate ------------------------------------------------------------

async fn build_bank(
    golden: &Golden,
    vectors: &Vectors,
) -> (
    tempfile::TempDir,
    std::sync::Arc<memory::store::Bank>,
    HashMap<String, String>,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().to_path_buf()).unwrap();
    let (bank, _) = store.ensure_bank("eval", None).await.unwrap();
    let now = now_ms();
    let mut key_of_id = HashMap::new();
    for g in &golden.memories {
        let at = now - g.age_days * DAY_MS;
        let m = Memory {
            id: fingerprint(&g.text),
            text: g.text.clone(),
            entities: g.entities.clone(),
            tags: g.tags.clone(),
            confidence: Confidence::Extracted,
            corroboration: g.corroboration,
            pinned: g.pinned,
            source: None,
            created_at: at,
            updated_at: at,
            invalid_at: None,
            superseded_by: None,
            revision: 0,
            stated_at: Some(at),
        };
        bank.commit(m.clone()).await.unwrap();
        let e = &vectors.items[&g.key];
        bank.set_vector(&m.id, &vectors.model, &m.text, e.v.clone())
            .await
            .unwrap();
        key_of_id.insert(m.id, g.key.clone());
    }
    (dir, bank, key_of_id)
}

fn load_vectors(golden: &Golden, file: &str) -> (Vectors, String) {
    // `--exact`: `embed_golden_set` alone would also match the MiniLM one.
    let regen = match file {
        "vectors-minilm.json" => "embed_golden_set_minilm",
        _ => "embed_golden_set",
    };
    let raw = std::fs::read_to_string(eval_dir().join(file)).unwrap_or_else(|_| {
        panic!(
            "tests/recall_quality/{file} missing: run `cargo test --test recall_quality -- \
             --ignored --exact {regen}`"
        )
    });
    let vectors: Vectors = serde_json::from_str(&raw).expect("vectors.json parses");
    // A negative may have no vector: hybrid reports it unmeasured (MiniLM
    // needs an iii-directory built with it to embed one).
    let texts = golden
        .memories
        .iter()
        .map(|m| (&m.key, &m.text, true))
        .chain(golden.queries.iter().map(|q| (&q.id, &q.text, true)))
        .chain(golden.negatives.iter().map(|n| (&n.id, &n.text, false)));
    for (key, text, required) in texts {
        let e = vectors.items.get(key.as_str());
        assert!(
            e.map_or(!required, |e| &e.text == text),
            "cached vector for {key} in {file} is missing or embeds an older text: run \
             `cargo test --test recall_quality -- --ignored --exact {regen}`"
        );
    }
    (vectors, fingerprint(&raw))
}

#[tokio::test]
async fn recall_quality_matches_baseline() {
    gate("vectors.json", "recall_quality.json").await;
}

#[tokio::test]
async fn minilm_recall_quality_matches_baseline() {
    gate("vectors-minilm.json", "recall_quality-minilm.json").await;
}

async fn gate(vectors_file: &str, baseline_file: &str) {
    let golden = load_golden();
    validate(&golden);
    let (vectors, vectors_fp) = load_vectors(&golden, vectors_file);
    let consolidation = load_consolidation(&golden);
    let (_dir, bank, key_of_id) = build_bank(&golden, &vectors).await;
    // A second bank with the recorded consolidation decisions applied.
    let (_dir2, consolidated, _) = build_bank(&golden, &vectors).await;
    apply_consolidation(&consolidated, &golden, &consolidation).await;
    let survivors = survivor_of(&consolidation);
    let half_life = WorkerConfig::default().decay_half_life_days;
    let pairs: Vec<(&str, &str)> = golden
        .memories
        .iter()
        .filter_map(|m| m.dup_of.as_deref().map(|of| (m.key.as_str(), of)))
        .collect();

    let mut report = String::new();
    let mut snapshot = BTreeMap::new();
    for mode in ["lexical", "hybrid", "hybrid-consolidated"] {
        let (bank, hybrid) = match mode {
            "lexical" => (&bank, false),
            "hybrid" => (&bank, true),
            _ => (&consolidated, true),
        };
        let mut outcomes = Vec::new();
        for q in &golden.queries {
            let qv = hybrid.then(|| vectors.items[&q.id].v.as_slice());
            let hits = bank
                .recall(&q.text, qv, &vectors.model, DEPTH, half_life, false)
                .await;
            if hybrid {
                assert!(!hits.is_empty(), "guard: {} recalled nothing", q.id);
            }
            let ranked: Vec<&str> = hits
                .iter()
                .map(|(m, _)| key_of_id[&m.id].as_str())
                .collect();
            let (expect, stale) = if mode == "hybrid-consolidated" {
                remap(q, &survivors)
            } else {
                (q.expect.clone(), q.stale.clone())
            };
            let mut outcome = score(&ranked, &expect, &stale);
            outcome.dup_polluted = dup_polluted(&ranked, &pairs);
            outcomes.push((q, outcome));
        }

        let mut slices: BTreeMap<String, Vec<(&GoldenQuery, Outcome)>> = BTreeMap::new();
        for (q, o) in &outcomes {
            for slice in [
                "all".to_string(),
                format!("category:{}", q.category),
                format!("lang:{}", q.lang),
            ] {
                slices.entry(slice).or_default().push((*q, o.clone()));
            }
        }
        let summaries: BTreeMap<String, Summary> = slices
            .iter()
            .map(|(k, v)| (k.clone(), summarize(v)))
            .collect();

        report.push_str(&format!(
            "\n[{mode}]\n{:<26} {:>4} {:>6} {:>6} {:>7} {:>8} {:>9} {:>6} {:>5}\n",
            "slice", "n", "hit@1", "hit@3", "mrr@10", "ndcg@10", "recall@10", "traps", "dups"
        ));
        for (slice, s) in &summaries {
            report.push_str(&format!(
                "{slice:<26} {:>4} {:>6.3} {:>6.3} {:>7.3} {:>8.3} {:>9.3} {:>3}/{:<2} {:>5}\n",
                s.queries,
                s.hit_at_1,
                s.hit_at_3,
                s.mrr_at_10,
                s.ndcg_at_10,
                s.recall_at_10,
                s.traps,
                s.trap_queries,
                s.dup_polluted
            ));
        }
        let mut worst: Vec<&(&GoldenQuery, Outcome)> = outcomes
            .iter()
            .filter(|(_, o)| o.blame() != "ok" || o.trap)
            .collect();
        worst.sort_by(|a, b| {
            a.1.ndcg
                .partial_cmp(&b.1.ndcg)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.id.cmp(&b.0.id))
        });
        // Traps first, all of them: each is a superseded fact outranking the
        // current one — never hidden behind the worst-5 cut.
        for (q, _) in outcomes.iter().filter(|(_, o)| o.trap) {
            report.push_str(&format!(
                "TRAP {} (stale {:?} above {}) — {}\n",
                q.id, q.stale, q.expect[0], q.text
            ));
        }
        report.push_str("worst:\n");
        for (q, o) in worst.iter().take(5) {
            report.push_str(&format!(
                "  {} [{}{}] first_hit={:?} — {}\n",
                q.id,
                o.blame(),
                if o.trap { ", TRAP" } else { "" },
                o.first_hit,
                q.text
            ));
        }

        // Guard: most queries must reach gold, or the fixture/pipeline broke.
        if hybrid {
            let reached = outcomes
                .iter()
                .filter(|(_, o)| o.first_hit.is_some())
                .count();
            assert!(
                reached * 2 > outcomes.len(),
                "guard: only {reached} queries reached gold\n{report}"
            );
        }
        snapshot.insert(mode.to_string(), serde_json::to_value(&summaries).unwrap());
    }
    println!("{report}");

    let baseline = serde_json::json!({
        "embedding": { "model": vectors.model, "dims": vectors.dims, "vectors_fingerprint": vectors_fp },
        "half_life_days": half_life,
        "modes": snapshot,
    });
    let actual = serde_json::to_string_pretty(&baseline).unwrap() + "\n";
    if let Err(diff) = support::check_golden(baseline_file, &actual) {
        panic!("recall quality changed vs the committed baseline:\n{report}\n{diff}");
    }
}

// ---- injection_quality: what the hook sends --------------------------------

/// A message the injection eval sends: a golden query, or a negative.
struct Message<'a> {
    id: &'a str,
    text: &'a str,
    negative: bool,
    /// Needed keys (none for a negative), superseded keys, usable keys.
    expect: &'a [String],
    stale: &'a [String],
    related: &'a [String],
    /// `all`, `lang:*`, `category:*` or `kind:*`, and `split:*`.
    slices: [String; 4],
}

/// Held-out positives: about a third, by id, so no data edit moves one.
fn split_of(id: &str) -> &'static str {
    match fingerprint(id).as_bytes()[2] {
        b'0'..=b'4' => "test",
        _ => "dev",
    }
}

fn messages(golden: &Golden) -> Vec<Message<'_>> {
    let queries = golden.queries.iter().map(|q| Message {
        id: &q.id,
        text: &q.text,
        negative: false,
        expect: &q.expect,
        stale: &q.stale,
        related: &[],
        slices: [
            "all".into(),
            format!("lang:{}", q.lang),
            format!("category:{}", q.category),
            format!("split:{}", split_of(&q.id)),
        ],
    });
    let negatives = golden.negatives.iter().map(|n| Message {
        id: &n.id,
        text: &n.text,
        negative: true,
        expect: &[],
        stale: &[],
        related: &n.related,
        slices: [
            "all".into(),
            format!("lang:{}", n.lang),
            format!("kind:{}", n.kind),
            format!("split:{}", n.split),
        ],
    });
    queries.chain(negatives).collect()
}

/// Each memory's paraphrase cluster: the key its `dup_of` chain ends at.
fn clusters(golden: &Golden) -> HashMap<&str, &str> {
    let of: HashMap<&str, &str> = golden
        .memories
        .iter()
        .filter_map(|m| m.dup_of.as_deref().map(|d| (m.key.as_str(), d)))
        .collect();
    golden
        .memories
        .iter()
        .map(|m| {
            let mut k = m.key.as_str();
            while let Some(&n) = of.get(k) {
                k = n;
            }
            (m.key.as_str(), k)
        })
        .collect()
}

/// What one message's injection held, in golden keys and fact clusters.
struct Shown<'a> {
    injected: Vec<&'a str>,
    /// Needed clusters, and how many of them came through.
    needed: usize,
    delivered: usize,
    /// Needed clusters recall ranks within `recall_limit` that `select` dropped.
    lost: Vec<&'a str>,
    /// Lines neither needed, related nor stale.
    irrelevant: usize,
    /// Two lines of one cluster.
    duplicate: bool,
    /// A superseded line, and whether the current fact (`expect[0]`) came too.
    stale: bool,
    current: bool,
}

fn clusters_of<'a>(
    keys: &[impl AsRef<str>],
    cluster: &HashMap<&str, &'a str>,
) -> BTreeSet<&'a str> {
    keys.iter().map(|k| cluster[k.as_ref()]).collect()
}

fn judge<'a>(
    m: &Message,
    injected: Vec<&'a str>,
    ranked: &[&str],
    cluster: &HashMap<&str, &'a str>,
) -> Shown<'a> {
    let need = clusters_of(m.expect, cluster);
    let related = clusters_of(m.related, cluster);
    let ranked = clusters_of(ranked, cluster);
    let got = clusters_of(&injected, cluster);
    let is_stale = |k: &str| m.stale.iter().any(|s| s == k);
    Shown {
        needed: need.len(),
        delivered: need.intersection(&got).count(),
        lost: need
            .iter()
            .filter(|c| ranked.contains(*c) && !got.contains(*c))
            .copied()
            .collect(),
        irrelevant: injected
            .iter()
            .filter(|k| !need.contains(cluster[*k]) && !related.contains(cluster[*k]))
            .filter(|k| !is_stale(k))
            .count(),
        duplicate: got.len() < injected.len(),
        stale: injected.iter().any(|k| is_stale(k)),
        current: m
            .expect
            .first()
            .is_some_and(|k| got.contains(cluster[k.as_str()])),
        injected,
    }
}

/// A slice's positives: what the needed facts cost and what came with them.
#[derive(Serialize)]
struct Positives {
    queries: usize,
    /// Needed fact clusters (`dup_of` closure), and the share injected.
    needed: usize,
    expected_injected: f64,
    /// `query: cluster` for each needed cluster `Bank::recall` ranks within
    /// `recall_limit` that `select` dropped.
    injection_loss: Vec<String>,
    /// Lines neither needed, related nor stale.
    irrelevant_injected: usize,
    /// Queries with two lines of one cluster.
    duplicate_queries: usize,
    /// Queries with a superseded line beside the current one, or alone.
    stale_with_current: usize,
    stale_alone: usize,
}

/// A slice's negatives.
#[derive(Serialize)]
struct Negatives {
    negatives: usize,
    /// Without a vector in this file: not scored in hybrid mode.
    unmeasured: usize,
    /// Share of the scored ones that injected nothing outside `related`.
    negative_clean_rate: Option<f64>,
    /// Lines they injected outside `related`.
    negative_injected: usize,
}

#[derive(Serialize)]
struct InjectionSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    positives: Option<Positives>,
    #[serde(skip_serializing_if = "Option::is_none")]
    negatives: Option<Negatives>,
}

/// A row is a message and what it injected (`None`: not scored in this mode).
type Row<'a> = (&'a Message<'a>, Option<Shown<'a>>);

fn summarize_injection<'a>(rows: &[&'a Row<'a>]) -> InjectionSummary {
    let scored = |negative: bool| -> Vec<(&'a str, &'a Shown<'a>)> {
        rows.iter()
            .copied()
            .filter(|row| row.0.negative == negative)
            .filter_map(|row: &'a Row<'a>| row.1.as_ref().map(|s| (row.0.id, s)))
            .collect()
    };
    let positives = scored(false);
    let scored_negatives = scored(true);
    let negatives = rows.iter().filter(|row| row.0.negative).count();
    let count =
        |of: &[(&str, &Shown)], f: &dyn Fn(&Shown) -> bool| of.iter().filter(|(_, s)| f(s)).count();
    let needed: usize = positives.iter().map(|(_, s)| s.needed).sum();
    let delivered: usize = positives.iter().map(|(_, s)| s.delivered).sum();
    InjectionSummary {
        positives: (!positives.is_empty()).then(|| Positives {
            queries: positives.len(),
            needed,
            expected_injected: round4(delivered as f64 / needed as f64),
            injection_loss: positives
                .iter()
                .flat_map(|(id, s)| s.lost.iter().map(move |c| format!("{id}: {c}")))
                .collect(),
            irrelevant_injected: positives.iter().map(|(_, s)| s.irrelevant).sum(),
            duplicate_queries: count(&positives, &|s| s.duplicate),
            stale_with_current: count(&positives, &|s| s.stale && s.current),
            stale_alone: count(&positives, &|s| s.stale && !s.current),
        }),
        negatives: (negatives > 0).then(|| Negatives {
            negatives,
            unmeasured: negatives - scored_negatives.len(),
            negative_clean_rate: (!scored_negatives.is_empty()).then(|| {
                let clean = count(&scored_negatives, &|s| s.irrelevant == 0);
                round4(clean as f64 / scored_negatives.len() as f64)
            }),
            negative_injected: scored_negatives.iter().map(|(_, s)| s.irrelevant).sum(),
        }),
    }
}

#[tokio::test]
async fn injection_quality_matches_baseline() {
    injection_gate("vectors.json", "injection_quality.json").await;
}

#[tokio::test]
async fn minilm_injection_quality_matches_baseline() {
    injection_gate("vectors-minilm.json", "injection_quality-minilm.json").await;
}

/// Each message through the hook's path on one bank: every candidate with
/// its evidence, then `hooks::select` with nothing visible. One bank's
/// shared BM25 max is its own best, as the hook's is with no layers.
async fn injection_gate(vectors_file: &str, baseline_file: &str) {
    let golden = load_golden();
    validate(&golden);
    let (vectors, vectors_fp) = load_vectors(&golden, vectors_file);
    let (_dir, bank, key_of_id) = build_bank(&golden, &vectors).await;
    let cfg = WorkerConfig::default();
    let (half_life, limit) = (cfg.decay_half_life_days, cfg.recall_limit);
    let cluster = clusters(&golden);
    let messages = messages(&golden);

    let mut report = String::new();
    let mut snapshot = BTreeMap::new();
    for mode in ["lexical", "hybrid"] {
        let mut rows: Vec<Row> = Vec::new();
        for m in &messages {
            let qv = match (mode, vectors.items.get(m.id)) {
                ("lexical", _) => None,
                (_, Some(e)) => Some(e.v.as_slice()),
                (_, None) => {
                    rows.push((m, None));
                    continue;
                }
            };
            let (hits, evidence) = bank
                .recall_evidence(
                    m.text,
                    qv,
                    &vectors.model,
                    usize::MAX,
                    half_life,
                    false,
                    0.0,
                )
                .await;
            let key = |mem: &Memory| key_of_id[&mem.id].as_str();
            // `Bank::recall` at `recall_limit` is this ranking, cut.
            let ranked: Vec<&str> = hits.iter().take(limit).map(|(mem, _)| key(mem)).collect();
            let injected = memory::hooks::select(hits, &evidence, &HashSet::new(), limit)
                .iter()
                .map(|(mem, _)| key(mem))
                .collect();
            rows.push((m, Some(judge(m, injected, &ranked, &cluster))));
        }

        let mut slices: BTreeMap<&str, Vec<&Row>> = BTreeMap::new();
        for row in &rows {
            for slice in &row.0.slices {
                slices.entry(slice.as_str()).or_default().push(row);
            }
        }
        let summaries: BTreeMap<&str, InjectionSummary> = slices
            .iter()
            .map(|(k, v)| (*k, summarize_injection(v)))
            .collect();

        report.push_str(&format!(
            "\n[{mode}]\n{:<26} {:>4} {:>6} {:>8} {:>4} {:>10} {:>4} {:>9} {:>11} | {:>4} {:>5} {:>6} {:>8}\n",
            "slice", "n", "needed", "expected", "loss", "irrelevant", "dups", "stale+cur",
            "stale-alone", "neg", "unmsr", "clean", "injected"
        ));
        for (slice, s) in &summaries {
            let mut line = format!("{slice:<26}");
            match &s.positives {
                Some(p) => line.push_str(&format!(
                    " {:>4} {:>6} {:>8.3} {:>4} {:>10} {:>4} {:>9} {:>11}",
                    p.queries,
                    p.needed,
                    p.expected_injected,
                    p.injection_loss.len(),
                    p.irrelevant_injected,
                    p.duplicate_queries,
                    p.stale_with_current,
                    p.stale_alone
                )),
                None => line.push_str(&format!("{:64}", "")),
            }
            if let Some(n) = &s.negatives {
                line.push_str(&format!(
                    " | {:>4} {:>5} {:>6} {:>8}",
                    n.negatives,
                    n.unmeasured,
                    n.negative_clean_rate
                        .map_or("-".into(), |r| format!("{r:.3}")),
                    n.negative_injected
                ));
            }
            report.push_str(line.trim_end());
            report.push('\n');
        }
        // Every offending message, never a worst-N cut.
        for (m, s) in &rows {
            let Some(s) = s else { continue };
            let mut why = Vec::new();
            if m.negative && s.irrelevant > 0 {
                why.push(format!("NEGATIVE {}", m.slices[2]));
            }
            if !s.lost.is_empty() {
                why.push(format!("LOSS {:?}", s.lost));
            }
            if s.duplicate {
                why.push("DUPLICATE".into());
            }
            if s.stale {
                why.push(
                    if s.current {
                        "STALE+CURRENT"
                    } else {
                        "STALE ALONE"
                    }
                    .into(),
                );
            }
            if !why.is_empty() {
                report.push_str(&format!(
                    "{} [{}] injected {:?} — {}\n",
                    m.id,
                    why.join(", "),
                    s.injected,
                    m.text
                ));
            }
        }

        // A superseded fact told without its replacement misleads the turn.
        let stale_alone = summaries["all"]
            .positives
            .as_ref()
            .map_or(0, |p| p.stale_alone);
        assert_eq!(
            stale_alone, 0,
            "{mode}: a stale line injected without the current one\n{report}"
        );
        // A needed line recall ranks but the hook drops: only the three
        // reached through a bare `user` alone (no stemming: "release" is not
        // "releases"), in lexical mode. A new loss fails here, not only in
        // the snapshot.
        let lost = summaries["all"]
            .positives
            .as_ref()
            .map(|p| p.injection_loss.clone())
            .unwrap_or_default();
        let known: &[&str] = match mode {
            "lexical" => &[
                "q-en-work-release-days: en-work-deploy-window",
                "q-en-work-late-week-rollout: en-work-deploy-window",
                "q-id-job-title: id-role",
            ],
            _ => &[],
        };
        assert_eq!(
            lost, known,
            "{mode}: injection dropped a needed line recall ranks\n{report}"
        );
        // Nothing relevant, nothing injected: a greeting, or a task in the
        // first person the bank shares no word with.
        if mode == "lexical" {
            for kind in ["kind:greeting", "kind:first-person"] {
                let clean = summaries[kind]
                    .negatives
                    .as_ref()
                    .and_then(|n| n.negative_clean_rate);
                assert_eq!(
                    clean,
                    Some(1.0),
                    "{mode}: a {kind} negative injected\n{report}"
                );
            }
        }
        snapshot.insert(mode, serde_json::to_value(&summaries).unwrap());
    }
    println!("{report}");

    let baseline = serde_json::json!({
        "embedding": { "model": vectors.model, "dims": vectors.dims, "vectors_fingerprint": vectors_fp },
        "half_life_days": half_life,
        "recall_limit": limit,
        "modes": snapshot,
    });
    let actual = serde_json::to_string_pretty(&baseline).unwrap() + "\n";
    if let Err(diff) = support::check_golden(baseline_file, &actual) {
        panic!("injection quality changed vs the committed baseline:\n{report}\n{diff}");
    }
}

/// Regenerates `tests/recall_quality/vectors.json` through a live engine's
/// `router::embed`. Pins the model so reruns are comparable, and the
/// provider: only provider-openai serves it, and an auto-pick could land on
/// one that echoes the requested model (llama-server) with its own vectors.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a live engine with provider-openai; rewrites tests/recall_quality/vectors.json"]
async fn embed_golden_set() {
    embed_golden(EMBED_MODEL, EMBED_DIMS, "openai", "vectors.json").await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a live engine running iii-directory; rewrites tests/recall_quality/vectors-minilm.json"]
async fn embed_golden_set_minilm() {
    // Named: an auto-pick tries every registered embedder first, and one
    // that echoes the requested model (llama-server) would answer for it.
    embed_golden(MINILM_MODEL, MINILM_DIMS, "minilm", "vectors-minilm.json").await;
}

/// The reply must be from `model` (bare or `model@<revision>`) and `provider`.
/// Only Matryoshka text-embedding-3-small is cut to its first `dims`; any
/// other model's vectors must be exactly `dims` long, and none may be shorter.
/// Incremental: only a key without a vector, or whose text changed, is
/// embedded; every other entry stays byte-identical (a re-embed drifts about
/// 1e-4 and would move every baseline), and keys the golden set dropped go.
async fn embed_golden(model: &str, dims: usize, provider: &str, file: &str) {
    let golden = load_golden();
    validate(&golden);
    let path = eval_dir().join(file);
    let cached: Option<Vectors> = std::fs::read_to_string(&path)
        .ok()
        .map(|raw| serde_json::from_str(&raw).expect("vectors file parses"));
    let all: Vec<(String, String)> = golden
        .memories
        .iter()
        .map(|m| (m.key.clone(), m.text.clone()))
        .chain(
            golden
                .queries
                .iter()
                .map(|q| (q.id.clone(), q.text.clone())),
        )
        .chain(
            golden
                .negatives
                .iter()
                .map(|n| (n.id.clone(), n.text.clone())),
        )
        .collect();
    let wanted: HashSet<String> = all.iter().map(|(k, _)| k.clone()).collect();
    let items: Vec<(String, String)> = all
        .into_iter()
        .filter(|(key, text)| {
            !cached
                .as_ref()
                .and_then(|c| c.items.get(key))
                .is_some_and(|e| &e.text == text)
        })
        .collect();
    let mut vectors = cached.unwrap_or_else(|| Vectors {
        model: String::new(),
        dims,
        items: BTreeMap::new(),
    });
    vectors.items.retain(|key, _| wanted.contains(key));
    if items.is_empty() {
        println!("{file}: every text already has a vector");
        std::fs::write(&path, serde_json::to_string(&vectors).unwrap() + "\n").unwrap();
        return;
    }
    let url = std::env::var("III_URL").unwrap_or_else(|_| "ws://127.0.0.1:49134".into());
    let iii = iii_sdk::register_worker(&url, iii_sdk::InitOptions::default());
    let texts: Vec<String> = items.iter().map(|(_, t)| t.clone()).collect();
    let reply = iii
        .trigger(iii_sdk::protocol::TriggerRequest {
            function_id: "router::embed".into(),
            payload: serde_json::json!({ "input": texts, "model": model, "provider": provider }),
            action: None,
            timeout_ms: Some(120_000),
        })
        .await
        .expect("router::embed failed (engine up? embed provider configured? III_NAMESPACE set?)");
    let reported = reply["model"].as_str().unwrap_or_default().to_string();
    assert!(
        reported == model || reported.starts_with(&format!("{model}@")),
        "router::embed answered with {reported:?}, not {model}"
    );
    assert_eq!(reply["provider"], provider, "answered by another provider");
    // The model the provider reported: what recall keys vectors by. Vectors
    // of two revisions live in different spaces.
    assert!(
        vectors.items.is_empty() || vectors.model == reported,
        "{file} holds {} vectors, router::embed answered with {reported}: delete the file \
         to re-embed every text",
        vectors.model
    );
    vectors.model = reported;
    let embedded: Vec<Vec<f32>> =
        serde_json::from_value(reply["embeddings"].clone()).expect("embeddings array");
    assert_eq!(embedded.len(), texts.len(), "one vector per input");
    let matryoshka = model == EMBED_MODEL;
    for ((key, text), v) in items.into_iter().zip(embedded) {
        assert!(
            v.len() == dims || (matryoshka && v.len() > dims),
            "{key}: {} dims, want {dims}",
            v.len()
        );
        let v = v
            .into_iter()
            .take(dims)
            .map(|x| (x * 1e4).round() / 1e4)
            .collect();
        vectors.items.insert(key, Embedded { text, v });
    }
    println!("{file}: embedded {} texts", texts.len());
    std::fs::write(&path, serde_json::to_string(&vectors).unwrap() + "\n").unwrap();
}

#[cfg(test)]
mod metric_tests {
    use super::*;

    fn keys(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn single_relevant_at_rank_two() {
        let o = score(&["a", "b", "c"], &keys(&["b"]), &[]);
        assert_eq!(o.first_hit, Some(2));
        assert!(close(o.rr(), 0.5));
        assert!(close(o.recall, 1.0));
        // (1/log2(3)) / (1/log2(2))
        assert!(close(o.ndcg, 0.6309), "{}", o.ndcg);
        assert_eq!(o.blame(), "ranking");
    }

    #[test]
    fn two_relevant_with_a_gap() {
        // DCG = 1 + 1/log2(4) = 1.5; IDCG = 1 + 1/log2(3) = 1.6309
        let o = score(&["a", "x", "b"], &keys(&["a", "b"]), &[]);
        assert!(close(o.ndcg, 0.9197), "{}", o.ndcg);
        assert_eq!(o.first_hit, Some(1));
        assert_eq!(o.blame(), "ok");
    }

    #[test]
    fn nothing_relevant_came_back() {
        let o = score(&["x", "y"], &keys(&["a", "b"]), &[]);
        assert_eq!((o.first_hit, o.recall, o.ndcg), (None, 0.0, 0.0));
        assert_eq!(o.blame(), "retrieval");
    }

    #[test]
    fn traps_need_a_stale_key_above_the_current_answer() {
        let rel = keys(&["a"]);
        let stale = keys(&["s"]);
        assert!(score(&["s", "a"], &rel, &stale).trap);
        assert!(!score(&["a", "s"], &rel, &stale).trap);
        assert!(score(&["s"], &rel, &stale).trap, "stale alone is a trap");
        assert!(!score(&["x"], &rel, &stale).trap, "neither came back");
        // A second relevant memory ranked first must not mask a stale one
        // sitting above the CURRENT answer (expect[0]).
        let rel2 = keys(&["current", "also"]);
        assert!(score(&["also", "s", "current"], &rel2, &stale).trap);
        assert!(!score(&["also", "current", "s"], &rel2, &stale).trap);
    }

    #[test]
    fn consolidated_expectations_follow_the_supersede_chain() {
        let c = Consolidation {
            decisions: vec![
                Decision {
                    kind: "supersede".into(),
                    keep: "moved".into(),
                    retire: "oct15".into(),
                },
                Decision {
                    kind: "merge".into(),
                    keep: "dup".into(),
                    retire: "moved".into(),
                },
            ],
        };
        let survivors = survivor_of(&c);
        assert_eq!(
            survivors["oct15"], "dup",
            "chains resolve to the live survivor"
        );
        let q = GoldenQuery {
            id: "q".into(),
            text: "t".into(),
            lang: "en".into(),
            category: "dated-event".into(),
            expect: keys(&["moved", "dup", "other"]),
            stale: keys(&["oct15"]),
        };
        let (expect, stale) = remap(&q, &survivors);
        assert_eq!(expect, keys(&["dup", "other"]), "deduped, order kept");
        assert!(
            stale.is_empty(),
            "a retired stale record can no longer trap"
        );
    }

    #[test]
    fn only_the_top_depth_counts() {
        let mut ranked: Vec<String> = (0..DEPTH).map(|i| format!("x{i}")).collect();
        ranked.push("a".into());
        let ranked: Vec<&str> = ranked.iter().map(String::as_str).collect();
        assert_eq!(score(&ranked, &keys(&["a"]), &[]).first_hit, None);
    }
}
