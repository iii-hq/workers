//! Lexical function search: the BM25 scorer, tokenizer folds, corpus
//! slimming, and fingerprinting behind `directory::search_functions`.
//!
//! Duplicated from the reflex spike (reflex/src/runtime.rs) by decision —
//! reflex is disposable and this worker owns the calibrations from here on.
//! The spike's `select()` (namespace-margin confidence + document-frequency
//! floor) stayed behind: it served the per-message hook, not one-shot search.

use std::collections::HashMap;

use sha2::{Digest, Sha256};

/// Byte budget for a compacted query.
pub(crate) const MAX_INPUT_BYTES: usize = 2_048;
/// Byte budget for a slimmed contract description (first sentence).
const INDEX_DESCRIPTION_BYTES: usize = 160;
/// Function-id prefixes that never participate in search: the engine's own
/// surface, this worker, and the reflex spike (its hook machinery must not
/// be discovered as a capability).
pub(crate) const EXCLUDED_NAMESPACE_PREFIXES: [&str; 1] = ["engine::"];
/// The search's own id: never searchable, never operating evidence.
pub(crate) const SEARCH_FN: &str = "directory::search_functions";
/// Exact function ids hidden from search regardless of their worker: infra
/// primitives an agent must never be handed as a capability (they fail the
/// agent's dispatch policy if returned). `state::claim-namespace` is a
/// worker-lifecycle claim, not a task capability.
pub(crate) const EXCLUDED_FUNCTION_IDS: [&str; 1] = ["state::claim-namespace"];
/// Function-id suffixes that are internal by convention. `<worker>::on-config-change`
/// is the configuration-reload handler every worker registers (see the
/// `iii-config-client` crate). Every in-repo registration now carries the
/// `internal: true` metadata the catalog filter keys on; this rule guards
/// against a future hand-rolled registration that forgets it.
pub(crate) const EXCLUDED_FUNCTION_SUFFIXES: [&str; 1] = ["::on-config-change"];

/// Whether a function id must be kept out of every search lane: the search's
/// own id, an excluded namespace prefix, an explicitly excluded id, or an
/// internal-by-convention suffix. Functions that do carry
/// `metadata.internal: true` are dropped earlier, when the catalog is built.
pub(crate) fn excluded_from_search(id: &str) -> bool {
    id == SEARCH_FN
        || EXCLUDED_NAMESPACE_PREFIXES
            .iter()
            .any(|prefix| id.starts_with(prefix))
        || EXCLUDED_FUNCTION_IDS.contains(&id)
        || EXCLUDED_FUNCTION_SUFFIXES
            .iter()
            .any(|suffix| id.ends_with(suffix))
}

/// One catalog entry: the contract fields search runs on.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

const BM25_STOPWORDS: [&str; 22] = [
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "in", "is", "it", "of", "on",
    "or", "that", "the", "this", "to", "was", "with",
];

pub(crate) fn bm25_terms(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|term| !term.is_empty())
        .flat_map(|term| {
            let mut expanded = camel_parts(term);
            expanded.push(term.to_ascii_lowercase());
            expanded
        })
        .filter(|term| !BM25_STOPWORDS.contains(&term.as_str()))
        .map(singularize)
}

/// Minimal English plural folding so "values" meets "value" and "repos"
/// meets "repo" — the terse real-world contract text ("Set a value in
/// state") never matches a plural query term otherwise. Deliberately
/// conservative: only a trailing `s` on longer words, never `-ss`/`-us`/
/// `-is` (process, status, redis).
fn singularize(term: String) -> String {
    if term.len() > 4 && term.ends_with("ies") {
        let mut term = term;
        term.truncate(term.len() - 3);
        term.push('y');
        return term;
    }
    if term.len() > 3
        && term.ends_with('s')
        && !term.ends_with("ss")
        && !term.ends_with("us")
        && !term.ends_with("is")
    {
        let mut term = term;
        term.pop();
        term
    } else {
        term
    }
}

/// Lowercased camelCase segments of a token: "presignUrl" → ["presign",
/// "url"]. Function names use camelCase for real words ("putObject",
/// "beginTransaction") and natural-language queries never do, so without
/// this split those names can never match their own vocabulary. Tokens
/// without an inner case transition yield nothing extra.
fn camel_parts(term: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut previous_lower = false;
    for character in term.chars() {
        if character.is_ascii_uppercase() && previous_lower {
            parts.push(current.to_ascii_lowercase());
            current = String::new();
        }
        previous_lower = character.is_ascii_lowercase() || character.is_ascii_digit();
        current.push(character);
    }
    if !current.is_empty() {
        parts.push(current.to_ascii_lowercase());
    }
    if parts.len() <= 1 {
        return Vec::new();
    }
    parts
}

/// Observations quote raw function results, and the JSON keys in those
/// results ("stdout", "exit_code", "duration_ms") are contract vocabulary:
/// measured live, they lexically pull selection toward whichever OTHER
/// worker documents the same response shape (github::exec at 0.66-0.76
/// after plain shell results). The values and the "worker::fn returned:"
/// prefix carry the real signal, so a quoted string directly followed by a
/// colon — a JSON key at any nesting depth — is dropped from the query.
fn without_json_keys(text: &str) -> String {
    let characters: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut position = 0;
    while position < characters.len() {
        if characters[position] != '"' {
            out.push(characters[position]);
            position += 1;
            continue;
        }
        let mut close = position + 1;
        while close < characters.len() && characters[close] != '"' {
            if characters[close] == '\\' {
                close += 1;
            }
            close += 1;
        }
        if close < characters.len() {
            let mut colon = close + 1;
            while colon < characters.len() && characters[colon].is_whitespace() {
                colon += 1;
            }
            if colon < characters.len() && characters[colon] == ':' {
                position = colon + 1;
                continue;
            }
        }
        out.extend(&characters[position..characters.len().min(close + 1)]);
        position = close + 1;
    }
    out
}

fn namespace_of(function_id: &str) -> &str {
    function_id
        .split_once("::")
        .map(|(namespace, _)| namespace)
        .unwrap_or(function_id)
}

struct Bm25Document {
    function_id: String,
    terms: HashMap<String, u32>,
    length: f64,
}

/// Classic BM25 over the slimmed contracts (name, first-sentence description,
/// argument names). Confidence is the margin between the best namespace and
/// the best OTHER namespace, so it composes with `confidence_threshold`
/// exactly like Needle's score: an ambiguous surface yields a low value.
///
/// Two guards, both from measuring live junk picks on the real catalog:
/// an idf floor drops near-stopwords — terms in more than a third of a
/// corpus carry no worker signal, but worker-namespace tokens are exempt
/// because on a narrow surface one worker's functions can legitimately be
/// most of the corpus — and the winning contract must match at least two
/// distinct query terms. Every measured chatter injection ("continue the
/// task", "help me with this") rode a single English function word that is
/// rare in terse contract text, so its idf is high and no frequency floor
/// can catch it; real objectives and observation-driven convergence always
/// matched several terms.
pub(crate) struct Bm25Index {
    documents: Vec<Bm25Document>,
    document_frequency: HashMap<String, u32>,
    namespaces: std::collections::HashSet<String>,
    average_length: f64,
}

impl Bm25Index {
    const K1: f64 = 1.2;
    const B: f64 = 0.75;
    const MINIMUM_MATCHED_TERMS: u32 = 2;

    pub(crate) fn build(tools: &[ToolSchema]) -> Self {
        let mut documents = Vec::with_capacity(tools.len());
        let mut document_frequency: HashMap<String, u32> = HashMap::new();
        for tool in tools {
            // The name appears three times: matching a query term against
            // the function's own name is far stronger relevance evidence
            // than matching shared description vocabulary, and the boosted
            // term frequency (BM25-saturated by K1) is what lets the
            // relative function floor prune same-worker family members
            // that match only the namespace token plus a generic word.
            let mut text = format!("{0} {0} {1}", tool.name, searchable_text(tool));
            if tool.name == "browser::fetch" {
                text.push_str(
                    " default static web page webpage website RSS Atom API content scrape scraping",
                );
            }
            let mut terms: HashMap<String, u32> = HashMap::new();
            for term in bm25_terms(&text) {
                *terms.entry(term).or_default() += 1;
            }
            for term in terms.keys() {
                *document_frequency.entry(term.clone()).or_default() += 1;
            }
            let length = terms.values().map(|count| f64::from(*count)).sum();
            documents.push(Bm25Document {
                function_id: tool.name.clone(),
                terms,
                length,
            });
        }
        let average_length = if documents.is_empty() {
            0.0
        } else {
            documents
                .iter()
                .map(|document| document.length)
                .sum::<f64>()
                / documents.len() as f64
        };
        let namespaces = documents
            .iter()
            .map(|document| namespace_of(&document.function_id).to_string())
            .collect();
        Self {
            documents,
            document_frequency,
            namespaces,
            average_length,
        }
    }

    /// BM25 scores and distinct-matched-term counts for every document.
    /// `df_floor` drops query terms more frequent than a third of the
    /// corpus (namespace tokens exempt): calibrated for the hook's
    /// observation-laden inputs, where common contract vocabulary drags
    /// selection sideways. Discover queries are intentional, so the rank
    /// path keeps every term — its coverage, function, and namespace
    /// pruning layers do the guarding there.
    fn scored(&self, query: &str, df_floor: bool) -> (Vec<f64>, Vec<u32>) {
        let query = without_json_keys(query);
        let total = self.documents.len() as f64;
        let floor = 2.0_f64.max(total / 3.0);
        let mut scores = vec![0.0_f64; self.documents.len()];
        let mut matched = vec![0_u32; self.documents.len()];
        let terms: std::collections::HashSet<String> = bm25_terms(&query).collect();
        for term in terms {
            let Some(&frequency) = self.document_frequency.get(&term) else {
                continue;
            };
            let frequency = f64::from(frequency);
            if df_floor && frequency > floor && !self.namespaces.contains(&term) {
                continue;
            }
            let idf = (((total - frequency + 0.5) / (frequency + 0.5)) + 1.0).ln();
            for (index, document) in self.documents.iter().enumerate() {
                let Some(&term_frequency) = document.terms.get(&term) else {
                    continue;
                };
                let term_frequency = f64::from(term_frequency);
                let norm =
                    Self::K1 * (1.0 - Self::B + Self::B * document.length / self.average_length);
                scores[index] += idf * term_frequency * (Self::K1 + 1.0) / (term_frequency + norm);
                matched[index] += 1;
            }
        }
        (scores, matched)
    }

    /// `rank` plus each document's distinct-matched-term count, for
    /// coverage-aware pruning downstream.
    pub(crate) fn rank_with_matches(&self, query: &str) -> Vec<(String, f64, u32)> {
        self.ranked_with_minimum(query, Self::MINIMUM_MATCHED_TERMS)
    }

    fn ranked_with_minimum(&self, query: &str, minimum: u32) -> Vec<(String, f64, u32)> {
        let (scores, matched) = self.scored(query, false);
        let mut ranked: Vec<(String, f64, u32)> = self
            .documents
            .iter()
            .enumerate()
            .filter(|(index, _)| scores[*index] > 0.0 && matched[*index] >= minimum)
            .map(|(index, document)| (document.function_id.clone(), scores[index], matched[index]))
            .collect();
        ranked.sort_by(|left, right| right.1.total_cmp(&left.1));
        ranked
    }
}

/// Compact a raw search query to the byte budget on a char boundary.
pub(crate) fn compact_query(query: &str) -> String {
    truncate(query.to_string(), MAX_INPUT_BYTES)
}

fn truncate(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value
}

fn canonical_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let object = keys
                .into_iter()
                .map(|key| (key.clone(), canonical_value(&object[key])))
                .collect();
            serde_json::Value::Object(object)
        }
        serde_json::Value::Array(array) => {
            serde_json::Value::Array(array.iter().map(canonical_value).collect())
        }
        _ => value.clone(),
    }
}

// Needle completion latency scales with the size of the indexed tool corpus
// (measured: ~0.9 s at 40 full contracts, past the 4.5 s hook budget at ~120).
// Search needs only the matching signal: name, first description sentence,
// and argument names. Full request contracts are fetched later through
// `engine::functions::info` for the selected candidates.
pub(crate) fn slim_description(description: &str) -> String {
    let line = description.lines().next().unwrap_or_default();
    let sentence = line
        .char_indices()
        .find_map(|(position, punctuation)| {
            let end = position + punctuation.len_utf8();
            (matches!(punctuation, '.' | '?' | '!')
                && line[end..].chars().next().is_none_or(char::is_whitespace))
            .then_some(&line[..end])
        })
        .unwrap_or(line);
    truncate(sentence.trim_end().to_string(), INDEX_DESCRIPTION_BYTES)
}

/// Canonical text shared by lexical and semantic search: id, first
/// description sentence, then request-property names in stable order.
pub(crate) fn searchable_text(tool: &ToolSchema) -> String {
    let mut text = format!("{} {}", tool.name, slim_description(&tool.description));
    if let Some(properties) = tool
        .parameters
        .get("properties")
        .and_then(serde_json::Value::as_object)
    {
        let mut keys: Vec<&String> = properties.keys().collect();
        keys.sort_unstable();
        for key in keys {
            text.push(' ');
            text.push_str(key);
        }
    }
    text
}

fn slim_parameters(parameters: &serde_json::Value) -> serde_json::Value {
    let mut slim = serde_json::Map::new();
    slim.insert("type".into(), serde_json::Value::String("object".into()));
    if let Some(properties) = parameters
        .get("properties")
        .and_then(serde_json::Value::as_object)
    {
        let names = properties
            .keys()
            .map(|key| {
                (
                    key.clone(),
                    serde_json::Value::Object(serde_json::Map::new()),
                )
            })
            .collect();
        slim.insert("properties".into(), serde_json::Value::Object(names));
    }
    if let Some(required) = parameters.get("required").filter(|value| value.is_array()) {
        slim.insert("required".into(), required.clone());
    }
    serde_json::Value::Object(slim)
}

pub(crate) fn canonical_tools(tools: &[ToolSchema]) -> Vec<ToolSchema> {
    let mut tools = tools
        .iter()
        // The engine control plane is not a selectable "worker", and the
        // search must never rank itself: excluding both keeps results to
        // capabilities a task can actually be built from.
        .filter(|tool| !excluded_from_search(&tool.name))
        .map(|tool| ToolSchema {
            name: tool.name.clone(),
            description: slim_description(&tool.description),
            parameters: canonical_value(&slim_parameters(&tool.parameters)),
        })
        .collect::<Vec<_>>();
    tools.sort_by(|left, right| {
        left.name.cmp(&right.name).then_with(|| {
            serde_json::to_string(left)
                .expect("tool schema serializes")
                .cmp(&serde_json::to_string(right).expect("tool schema serializes"))
        })
    });
    tools
}

pub fn tool_fingerprint(tools: &[ToolSchema]) -> String {
    let serialized = serde_json::to_vec(&canonical_tools(tools)).expect("tool schemas serialize");
    Sha256::digest(serialized)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
