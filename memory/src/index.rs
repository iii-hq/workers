//! In-RAM BM25 index plus the fused recall scorer.
//!
//! The index is deliberately a cache: it is rebuilt from `memories.jsonl` at
//! boot and updated through the store's single commit choke point, so it can
//! never diverge from disk across a restart. At memory scale (10^3–10^5
//! memories) a plain inverted index scores in well under a millisecond — no
//! ANN machinery, no external engine, no query-time LLM.

use std::collections::HashMap;

use crate::types::Memory;

const BM25_K1: f32 = 1.2;
const BM25_B: f32 = 0.75;
/// Relevance multiplier per query-token hit on an entity handle.
const ENTITY_BOOST: f32 = 0.3;
/// Relevance multiplier per unit of `ln(1 + corroboration)`.
const CORROBORATION_BOOST: f32 = 0.3;
/// Relevance multiplier for pinned memories.
const PINNED_BOOST: f32 = 0.25;
/// How much recency can move a score: ±`RECENCY_WEIGHT / 2` around neutral
/// (fresh ×1.2, very old ×0.8). Hindsight uses 0.2; 0.4 measured best on the
/// recall eval (durable-old hit@1 0.5 -> 1.0) without adding traps.
const RECENCY_WEIGHT: f32 = 0.4;

/// Lowercased alphanumeric runs; CJK codepoints additionally emit character
/// bigrams so ideographic text is retrievable without a word segmenter.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut run = String::new();
    let mut prev_cjk: Option<char> = None;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            let lower = ch.to_lowercase().next().unwrap_or(ch);
            if is_cjk(ch) {
                if !run.is_empty() {
                    tokens.push(std::mem::take(&mut run));
                }
                tokens.push(lower.to_string());
                if let Some(prev) = prev_cjk {
                    tokens.push(format!("{prev}{lower}"));
                }
                prev_cjk = Some(lower);
            } else {
                run.push(lower);
                prev_cjk = None;
            }
        } else {
            if !run.is_empty() {
                tokens.push(std::mem::take(&mut run));
            }
            prev_cjk = None;
        }
    }
    if !run.is_empty() {
        tokens.push(run);
    }
    tokens
}

fn is_cjk(ch: char) -> bool {
    matches!(u32::from(ch),
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF)
}

/// Inverted index over memory text + entity handles.
#[derive(Default)]
pub struct Bm25Index {
    postings: HashMap<String, HashMap<String, u32>>,
    doc_tokens: HashMap<String, Vec<String>>,
    doc_len: HashMap<String, usize>,
    total_len: usize,
}

impl Bm25Index {
    pub fn add(&mut self, memory: &Memory) {
        self.remove(&memory.id);
        let mut text = memory.text.clone();
        for e in &memory.entities {
            text.push(' ');
            text.push_str(e);
        }
        let tokens = tokenize(&text);
        for t in &tokens {
            *self
                .postings
                .entry(t.clone())
                .or_default()
                .entry(memory.id.clone())
                .or_insert(0) += 1;
        }
        self.doc_len.insert(memory.id.clone(), tokens.len());
        self.total_len += tokens.len();
        self.doc_tokens.insert(memory.id.clone(), tokens);
    }

    /// Removal is part of the same choke point as insertion: a memory that
    /// leaves the map always leaves the index in the same call.
    pub fn remove(&mut self, id: &str) {
        let Some(tokens) = self.doc_tokens.remove(id) else {
            return;
        };
        self.total_len = self.total_len.saturating_sub(tokens.len());
        self.doc_len.remove(id);
        for t in tokens {
            if let Some(docs) = self.postings.get_mut(&t) {
                docs.remove(id);
                if docs.is_empty() {
                    self.postings.remove(&t);
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        self.doc_len.len()
    }

    pub fn is_empty(&self) -> bool {
        self.doc_len.is_empty()
    }

    /// Okapi BM25 over the query tokens.
    pub fn score(&self, query_tokens: &[String]) -> HashMap<String, f32> {
        let n = self.doc_len.len() as f32;
        if n == 0.0 {
            return HashMap::new();
        }
        let avgdl = (self.total_len as f32 / n).max(1.0);
        let mut scores: HashMap<String, f32> = HashMap::new();
        for term in query_tokens {
            let Some(docs) = self.postings.get(term) else {
                continue;
            };
            let df = docs.len() as f32;
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            for (doc, tf) in docs {
                let dl = *self.doc_len.get(doc).unwrap_or(&0) as f32;
                let tf = *tf as f32;
                let s = idf * (tf * (BM25_K1 + 1.0))
                    / (tf + BM25_K1 * (1.0 - BM25_B + BM25_B * dl / avgdl));
                *scores.entry(doc.clone()).or_insert(0.0) += s;
            }
        }
        scores
    }
}

/// Fused recall score for one candidate: its query relevance (BM25 plus the
/// weighted semantic similarity, when any) scaled by entity overlap,
/// corroboration, and pin boosts, then a mild recency multiplier
/// (±`RECENCY_WEIGHT / 2`). Superseded memories are excluded by the caller.
pub fn fused_score(
    relevance: f32,
    memory: &Memory,
    query_tokens: &[String],
    now_ms: u64,
    half_life_days: u64,
) -> f32 {
    let entity_hits = memory
        .entities
        .iter()
        .flat_map(|e| tokenize(e))
        .filter(|t| query_tokens.contains(t))
        .count() as f32;
    // History seeds carry zero relevance; a tiny floor keeps the boosts
    // and recency ordering them.
    let base = relevance.max(1e-6)
        * (1.0 + ENTITY_BOOST * entity_hits)
        * (1.0 + CORROBORATION_BOOST * (1.0 + memory.corroboration as f32).ln())
        * if memory.pinned {
            1.0 + PINNED_BOOST
        } else {
            1.0
        };
    let age_days = (now_ms.saturating_sub(memory.updated_at)) as f32 / 86_400_000.0;
    let half_life = half_life_days.max(1) as f32;
    // Freshness in (0, 1], neutral (0.5) at the half-life, folded in as a
    // MILD multiplier — Hindsight's `1 + alpha * (recency - 0.5)`. A full
    // multiplier faded durable preferences to a third of their weight.
    let recency = 0.5_f32.powf(age_days / half_life);
    base * (1.0 + RECENCY_WEIGHT * (recency - 0.5))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{fingerprint, now_ms, Confidence};

    fn memory(text: &str, entities: &[&str]) -> Memory {
        Memory {
            id: fingerprint(text),
            text: text.into(),
            entities: entities.iter().map(|s| s.to_string()).collect(),
            tags: vec![],
            confidence: Confidence::Extracted,
            corroboration: 0,
            pinned: false,
            source: None,
            created_at: now_ms(),
            updated_at: now_ms(),
            invalid_at: None,
            superseded_by: None,
            revision: 0,
            stated_at: None,
        }
    }

    #[test]
    fn tokenize_handles_unicode_and_cjk() {
        assert_eq!(
            tokenize("Mike's blog-style"),
            vec!["mike", "s", "blog", "style"]
        );
        assert_eq!(tokenize("Café rápido"), vec!["café", "rápido"]);
        let cjk = tokenize("日本語");
        assert!(cjk.contains(&"日本".to_string()), "CJK bigrams: {cjk:?}");
        assert!(cjk.contains(&"本語".to_string()));
    }

    #[test]
    fn bm25_ranks_the_matching_doc_first() {
        let mut idx = Bm25Index::default();
        let a = memory("Mike prefers formal writing with short paragraphs", &[]);
        let b = memory("The deploy pipeline uses nine cross-compile targets", &[]);
        idx.add(&a);
        idx.add(&b);
        let scores = idx.score(&tokenize("formal writing style"));
        assert!(scores.get(&a.id).copied().unwrap_or(0.0) > 0.0);
        assert!(!scores.contains_key(&b.id));
    }

    #[test]
    fn remove_unindexes_completely() {
        let mut idx = Bm25Index::default();
        let a = memory("temporary memory about kubernetes", &[]);
        idx.add(&a);
        assert_eq!(idx.len(), 1);
        idx.remove(&a.id);
        assert!(idx.is_empty());
        assert!(idx.score(&tokenize("kubernetes")).is_empty());
    }

    #[test]
    fn pinned_and_entities_boost_rank() {
        let now = now_ms();
        let q = tokenize("grafana dashboards");
        let plain = memory("team uses grafana", &[]);
        let mut boosted = memory("observability stack is grafana", &["grafana"]);
        boosted.pinned = true;
        let s_plain = fused_score(1.0, &plain, &q, now, 30);
        let s_boosted = fused_score(1.0, &boosted, &q, now, 30);
        assert!(s_boosted > s_plain);
    }

    #[test]
    fn recency_is_a_mild_bounded_multiplier() {
        let now = now_ms();
        let recent = memory("recent memory", &[]);
        let mut ancient = memory("ancient memory", &[]);
        ancient.updated_at = now.saturating_sub(4_000 * 86_400_000);
        let mut at_half_life = memory("half-life memory", &[]);
        at_half_life.updated_at = now.saturating_sub(60 * 86_400_000);
        let q = tokenize("memory");
        let s_recent = fused_score(1.0, &recent, &q, now, 60);
        let s_ancient = fused_score(1.0, &ancient, &q, now, 60);
        let s_half = fused_score(1.0, &at_half_life, &q, now, 60);
        assert!(s_recent > s_half && s_half > s_ancient);
        // Fresh gains at most +20%, very old loses at most -20%, and the
        // half-life is neutral: a durable preference keeps most of its weight.
        assert!((s_recent - 1.2).abs() < 1e-3, "{s_recent}");
        assert!(s_ancient >= 0.8 - 1e-3, "{s_ancient}");
        assert!((s_half - 1.0).abs() < 1e-3, "{s_half}");
    }
}

#[cfg(test)]
mod scoring_tests {
    use super::*;
    use crate::types::{fingerprint, Confidence, Memory};

    fn memory(text: &str, entities: &[&str]) -> Memory {
        Memory {
            id: fingerprint(text),
            text: text.into(),
            entities: entities.iter().map(|e| e.to_string()).collect(),
            tags: vec![],
            confidence: Confidence::Stated,
            corroboration: 0,
            pinned: false,
            source: None,
            created_at: 0,
            updated_at: 0,
            invalid_at: None,
            superseded_by: None,
            revision: 0,
            stated_at: None,
        }
    }

    #[test]
    fn tokenize_folds_case_and_splits_punctuation() {
        assert_eq!(
            tokenize("User PREFERS terse-answers!"),
            vec!["user", "prefers", "terse", "answers"]
        );
    }

    #[test]
    fn tokenize_emits_cjk_bigrams() {
        let tokens = tokenize("周二发布");
        assert!(
            tokens.iter().any(|t| t == "周二"),
            "bigrams expected, got {tokens:?}"
        );
    }

    #[test]
    fn removed_memories_stop_scoring() {
        let mut index = Bm25Index::default();
        let m = memory("the api port is 3000", &[]);
        index.add(&m);
        assert!(!index.score(&tokenize("api port")).is_empty());
        index.remove(&m.id);
        assert!(index.score(&tokenize("api port")).is_empty());
    }

    #[test]
    fn rarer_terms_score_higher() {
        let mut index = Bm25Index::default();
        for i in 0..5 {
            index.add(&memory(&format!("common filler text {i}"), &[]));
        }
        let rare = memory("unique zanzibar reference", &[]);
        index.add(&rare);
        let scores = index.score(&tokenize("zanzibar"));
        assert_eq!(scores.len(), 1);
        let common = index.score(&tokenize("common"));
        assert!(scores[&rare.id] > *common.values().next().unwrap());
    }

    #[test]
    fn pinned_and_corroborated_outrank_plain_at_equal_lexical_score() {
        let now = 1_000_000_000;
        let plain = memory("shared words here", &[]);
        let mut pinned = memory("shared words here!", &[]);
        pinned.pinned = true;
        let mut corroborated = memory("shared words here?", &[]);
        corroborated.corroboration = 4;
        let q = tokenize("shared words");
        let base = fused_score(1.0, &plain, &q, now, 30);
        assert!(fused_score(1.0, &pinned, &q, now, 30) > base);
        assert!(fused_score(1.0, &corroborated, &q, now, 30) > base);
        // History seeds (zero relevance) still order by the boosts.
        assert!(fused_score(0.0, &pinned, &q, now, 30) > fused_score(0.0, &plain, &q, now, 30));
    }

    #[test]
    fn entity_match_boosts() {
        let now = 1_000_000_000;
        let plain = memory("likes coffee", &[]);
        let tagged = memory("likes coffee!", &["mike"]);
        let q = tokenize("what does mike like");
        assert!(fused_score(1.0, &tagged, &q, now, 30) > fused_score(1.0, &plain, &q, now, 30));
    }

    #[test]
    fn recency_decay_is_monotonic_and_spares_pinned() {
        let now: u64 = 90 * 24 * 3_600 * 1_000;
        let q = tokenize("x");
        let mut fresh = memory("x fresh", &[]);
        fresh.updated_at = now;
        let mut stale = memory("x stale", &[]);
        stale.updated_at = 0;
        assert!(fused_score(1.0, &fresh, &q, now, 30) > fused_score(1.0, &stale, &q, now, 30));

        // Pinning lifts a record against peers of the same age; decay
        // still applies to everything (design: pinned protects data and
        // rank WITHIN an age band, it is not an absolute trump).
        let mut stale_pinned = stale.clone();
        stale_pinned.pinned = true;
        assert!(
            fused_score(1.0, &stale_pinned, &q, now, 30) > fused_score(1.0, &stale, &q, now, 30)
        );
    }
}
