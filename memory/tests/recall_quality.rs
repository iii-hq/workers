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
//!   engine with an embed provider): `III_NAMESPACE=<ns> cargo test --test
//!   recall_quality -- --ignored embed_golden_set`, and against an engine
//!   running iii-directory, `-- --ignored embed_golden_set_minilm`.
//!
//! The gate runs once per embedder: text-embedding-3-small, and the local
//! MiniLM that serves stacks without an embedding API. Semantic floors are
//! per model, so each one needs its own baseline.

mod support;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use memory::config::WorkerConfig;
use memory::store::Store;
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
/// reads as a quality change: unique keys, ids and texts, and every
/// `expect`/`stale` key present.
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
    let raw = std::fs::read_to_string(eval_dir().join(file)).unwrap_or_else(|_| {
        panic!("tests/recall_quality/{file} missing: run `cargo test --test recall_quality -- --ignored embed_golden_set`")
    });
    let vectors: Vectors = serde_json::from_str(&raw).expect("vectors.json parses");
    let texts = golden
        .memories
        .iter()
        .map(|m| (&m.key, &m.text))
        .chain(golden.queries.iter().map(|q| (&q.id, &q.text)));
    for (key, text) in texts {
        let e = vectors.items.get(key.as_str());
        assert!(
            e.is_some_and(|e| &e.text == text),
            "cached vector for {key} is missing or embeds an older text: run \
             `cargo test --test recall_quality -- --ignored embed_golden_set`"
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

/// Regenerates `tests/recall_quality/vectors.json` through a live engine's
/// `router::embed`. Pins the model so reruns are comparable.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a live engine with an embed provider; rewrites tests/recall_quality/vectors.json"]
async fn embed_golden_set() {
    embed_golden(EMBED_MODEL, EMBED_DIMS, "vectors.json").await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a live engine running iii-directory; rewrites tests/recall_quality/vectors-minilm.json"]
async fn embed_golden_set_minilm() {
    embed_golden(MINILM_MODEL, MINILM_DIMS, "vectors-minilm.json").await;
}

async fn embed_golden(model: &str, dims: usize, file: &str) {
    let golden = load_golden();
    validate(&golden);
    let url = std::env::var("III_URL").unwrap_or_else(|_| "ws://127.0.0.1:49134".into());
    let iii = iii_sdk::register_worker(&url, iii_sdk::InitOptions::default());
    let items: Vec<(String, String)> = golden
        .memories
        .iter()
        .map(|m| (m.key.clone(), m.text.clone()))
        .chain(
            golden
                .queries
                .iter()
                .map(|q| (q.id.clone(), q.text.clone())),
        )
        .collect();
    let texts: Vec<String> = items.iter().map(|(_, t)| t.clone()).collect();
    let (reported, embedded) = memory::embed_client::embed_texts(&iii, model, &texts, 120_000)
        .await
        .expect("router::embed failed (engine up? embed provider configured? III_NAMESPACE set?)");
    let vectors = Vectors {
        // The model the provider reported: what recall keys vectors by.
        model: reported,
        dims,
        items: items
            .into_iter()
            .zip(embedded)
            .map(|((key, text), v)| {
                let v = v
                    .into_iter()
                    .take(dims)
                    .map(|x| (x * 1e4).round() / 1e4)
                    .collect();
                (key, Embedded { text, v })
            })
            .collect(),
    };
    let out = serde_json::to_string(&vectors).unwrap() + "\n";
    std::fs::write(eval_dir().join(file), out).unwrap();
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
