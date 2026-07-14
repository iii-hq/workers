//! In-RAM BM25 index plus the fused recall scorer.
//!
//! The index is deliberately a cache: it is rebuilt from `facts.jsonl` at
//! boot and updated through the store's single commit choke point, so it can
//! never diverge from disk across a restart. At memory scale (10^3–10^5
//! facts) a plain inverted index scores in well under a millisecond — no
//! ANN machinery, no external engine, no query-time LLM.

use std::collections::HashMap;

use crate::types::Fact;

const BM25_K1: f32 = 1.2;
const BM25_B: f32 = 0.75;
/// Weight of one query-token hit on an entity handle.
const ENTITY_WEIGHT: f32 = 0.6;
/// Weight of `ln(1 + corroboration)`.
const CORROBORATION_WEIGHT: f32 = 0.3;
/// Flat bonus for pinned facts that matched at all.
const PINNED_BONUS: f32 = 0.75;
/// Recency floor so old-but-relevant facts stay reachable.
const RECENCY_FLOOR: f32 = 0.35;

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

/// Inverted index over fact text + entity handles.
#[derive(Default)]
pub struct Bm25Index {
    postings: HashMap<String, HashMap<String, u32>>,
    doc_tokens: HashMap<String, Vec<String>>,
    doc_len: HashMap<String, usize>,
    total_len: usize,
}

impl Bm25Index {
    pub fn add(&mut self, fact: &Fact) {
        self.remove(&fact.id);
        let mut text = fact.text.clone();
        for e in &fact.entities {
            text.push(' ');
            text.push_str(e);
        }
        let tokens = tokenize(&text);
        for t in &tokens {
            *self
                .postings
                .entry(t.clone())
                .or_default()
                .entry(fact.id.clone())
                .or_insert(0) += 1;
        }
        self.doc_len.insert(fact.id.clone(), tokens.len());
        self.total_len += tokens.len();
        self.doc_tokens.insert(fact.id.clone(), tokens);
    }

    /// Removal is part of the same choke point as insertion: a fact that
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

/// Fused recall score for one already-BM25-matched fact: entity overlap,
/// corroboration, and pin bonus added, then a recency multiplier with a
/// floor. Superseded facts are excluded by the caller.
pub fn fused_score(
    bm25: f32,
    fact: &Fact,
    query_tokens: &[String],
    now_ms: u64,
    half_life_days: u64,
) -> f32 {
    let entity_hits = fact
        .entities
        .iter()
        .flat_map(|e| tokenize(e))
        .filter(|t| query_tokens.contains(t))
        .count() as f32;
    let base = bm25
        + ENTITY_WEIGHT * entity_hits
        + CORROBORATION_WEIGHT * (1.0 + fact.corroboration as f32).ln()
        + if fact.pinned { PINNED_BONUS } else { 0.0 };
    let age_days = (now_ms.saturating_sub(fact.updated_at)) as f32 / 86_400_000.0;
    let half_life = half_life_days.max(1) as f32;
    let recency = 0.5_f32.powf(age_days / half_life).max(RECENCY_FLOOR);
    base * recency
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{fingerprint, now_ms, Confidence};

    fn fact(text: &str, entities: &[&str]) -> Fact {
        Fact {
            id: fingerprint(text),
            text: text.into(),
            entities: entities.iter().map(|s| s.to_string()).collect(),
            confidence: Confidence::Extracted,
            corroboration: 0,
            pinned: false,
            source: None,
            created_at: now_ms(),
            updated_at: now_ms(),
            invalid_at: None,
            superseded_by: None,
            revision: 0,
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
        let a = fact("Mike prefers formal writing with short paragraphs", &[]);
        let b = fact("The deploy pipeline uses nine cross-compile targets", &[]);
        idx.add(&a);
        idx.add(&b);
        let scores = idx.score(&tokenize("formal writing style"));
        assert!(scores.get(&a.id).copied().unwrap_or(0.0) > 0.0);
        assert!(!scores.contains_key(&b.id));
    }

    #[test]
    fn remove_unindexes_completely() {
        let mut idx = Bm25Index::default();
        let a = fact("temporary fact about kubernetes", &[]);
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
        let plain = fact("team uses grafana", &[]);
        let mut boosted = fact("observability stack is grafana", &["grafana"]);
        boosted.pinned = true;
        let s_plain = fused_score(1.0, &plain, &q, now, 30);
        let s_boosted = fused_score(1.0, &boosted, &q, now, 30);
        assert!(s_boosted > s_plain);
    }

    #[test]
    fn recency_decays_but_floors() {
        let now = now_ms();
        let recent = fact("recent fact", &[]);
        let mut ancient = fact("ancient fact", &[]);
        ancient.updated_at = now.saturating_sub(400 * 86_400_000);
        let q = tokenize("fact");
        let s_recent = fused_score(1.0, &recent, &q, now, 30);
        let s_ancient = fused_score(1.0, &ancient, &q, now, 30);
        assert!(s_recent > s_ancient);
        assert!(
            s_ancient >= 1.0 * RECENCY_FLOOR * 0.99,
            "floor holds: {s_ancient}"
        );
    }
}
