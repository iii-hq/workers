//! The requested answer slot of a recall query: "in which city" in "In which
//! city do I live?". A deliberately partial projection, not a parser: what
//! it cannot read is `None`, and recall keeps its plain word evidence.
//! Ranges are half-open indices into the unchanged `index::tokenize` output.
//! English only, by decision.
use std::ops::Range;

use crate::store::STOPWORDS;

/// "in which", "at what", "during which": a fronted prepositional WH phrase.
fn prefix(words: &[String]) -> bool {
    matches!(words, [prep, wh, ..]
        if matches!(prep.as_str(), "in" | "at" | "during") && matches!(wh.as_str(), "what" | "which"))
}

fn subject(word: &str) -> bool {
    matches!(word, "i" | "we" | "you" | "he" | "she" | "they" | "it")
}

fn auxiliary(word: &str) -> bool {
    matches!(
        word,
        "do" | "does"
            | "did"
            | "am"
            | "are"
            | "is"
            | "can"
            | "could"
            | "will"
            | "would"
            | "should"
            | "have"
            | "has"
    )
}

/// Only open-class runs and nominal linkers are admitted inside the slot.
/// No noun, topic, synonym, entity or candidate memory is consulted.
fn nominal(words: &[String]) -> bool {
    let content = |w: &str| !STOPWORDS.contains(&w);
    !words.is_empty()
        && content(&words[0])
        && content(words.last().unwrap())
        && words
            .iter()
            .all(|w| content(w) || matches!(w.as_str(), "of" | "the" | "a" | "an"))
}

/// An inverted auxiliary and its subject before a content word: "did I move".
fn has_clause(words: &[String]) -> bool {
    words
        .windows(3)
        .any(|w| auxiliary(&w[0]) && subject(&w[1]) && !STOPWORDS.contains(&w[2].as_str()))
}

/// The token span of the slot `query` asks to fill, or `None` (unknown):
/// a consumer then keeps its previous evidence and never infers a relation
/// from a failure to parse. Two shapes are read: a fronted prepositional WH
/// phrase before an inverted auxiliary and its subject ("In which city do I
/// live?"), or one coordinated after an inverted clause ("Did I move last
/// year, and in which city?"). `words` is `index::tokenize(query)`.
pub(crate) fn requested_slot(query: &str, words: &[String]) -> Option<Range<usize>> {
    // Quoting, multiple sentences and embedded clauses are outside it.
    let body = query.trim().strip_suffix('?').unwrap_or(query.trim());
    if body.chars().any(|c| {
        matches!(
            c,
            '?' | '.' | '!' | ';' | ':' | '\n' | '\r' | '"' | '\'' | '“' | '”' | '(' | ')'
        )
    }) {
        return None;
    }
    if let Some((left, right)) = body.rsplit_once(',') {
        if left.contains(',') {
            return None;
        }
        let lhs = crate::index::tokenize(left);
        let rhs = crate::index::tokenize(right);
        if !has_clause(&lhs)
            || rhs.first()? != "and"
            || !prefix(&rhs[1..])
            || !nominal(rhs.get(3..)?)
            || lhs.len() + rhs.len() != words.len()
        {
            return None;
        }
        return Some(lhs.len() + 1..words.len());
    }
    if !prefix(words) {
        return None;
    }
    let end = (2..words.len().saturating_sub(1))
        .find(|&i| auxiliary(&words[i]) && subject(&words[i + 1]))?;
    if !nominal(&words[2..end])
        || !words[end + 2..]
            .iter()
            .any(|w| !STOPWORDS.contains(&w.as_str()))
    {
        return None;
    }
    Some(0..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(query: &str) -> Option<Range<usize>> {
        requested_slot(query, &crate::index::tokenize(query))
    }

    #[test]
    fn a_fronted_wh_phrase_is_the_slot() {
        // in which mountain range / did / i / hike / last / summer
        assert_eq!(
            slot("In which mountain range did I hike last summer?"),
            Some(0..4)
        );
        assert_eq!(
            slot("Did I adopt a cat in spring, and in which shelter?"),
            Some(8..11)
        );
    }

    #[test]
    fn what_it_cannot_read_is_unknown() {
        for query in [
            "Which mountain range did I hike?",
            "In which range?",
            "In which mountain range did I?",
            "In which range did I hike? And when?",
            "In which \"range\" did I hike?",
            "I hiked, I swam, and in which lake?",
            "",
        ] {
            assert_eq!(slot(query), None, "{query}");
        }
    }
}
