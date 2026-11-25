//! Fuzzy full-text search over session titles and transcripts — the engine
//! behind `session::search`.
//!
//! Each session becomes a [`SearchDoc`]: its title plus the plain text of
//! every user and assistant message (function calls, their results and
//! thinking are left out — they are mostly file dumps and would drown the
//! words people remember). A query is split into words, and every word must
//! land somewhere in the session, by one of three strengths:
//!
//! - **exact**: the whole word (`login` finds `login`);
//! - **prefix**: the start of a word (`stream` finds `streamlines`);
//! - **fuzzy**: a typo away (`stremline` finds `streamline`): one edit for
//!   words of four to seven characters, two from eight, the first letter
//!   always right. Shorter words never match fuzzily — `cat` would find
//!   half the dictionary.
//!
//! Results rank in tiers (see [`Tier`]): title hits first, then a message
//! holding the query as a phrase, then a message holding every word, then
//! anything looser; newest first inside a tier. Content hits carry a
//! snippet of the best message with the matched words marked, offsets in
//! UTF-16 code units so a browser can slice the string directly.

use std::collections::{HashMap, HashSet};

use schemars::JsonSchema;
use serde::Serialize;

use crate::types::{AgentMessage, ContentBlock, SessionEntry};

/// How strongly one query word matched one transcript word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Strength {
    Fuzzy = 1,
    Prefix = 2,
    Exact = 3,
}

/// Which message role a snippet came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SnippetRole {
    User,
    Assistant,
}

/// One message's searchable text.
#[derive(Debug, Clone)]
struct DocMessage {
    entry_id: String,
    role: SnippetRole,
    text: String,
}

/// A session's searchable text, built once per revision and reused until
/// the session changes.
#[derive(Debug, Clone, Default)]
pub struct SearchDoc {
    messages: Vec<DocMessage>,
    /// Every distinct lowercase word across `messages`.
    vocabulary: HashSet<String>,
}

impl SearchDoc {
    /// Build from a session's entries. Every branch is indexed, not only
    /// the active path: a branch switch does not move `updated_at`, and a
    /// word in an abandoned branch is still something the chat said.
    pub fn from_entries(entries: &[SessionEntry]) -> Self {
        let mut sorted: Vec<&SessionEntry> = entries.iter().collect();
        sorted.sort_by(|a, b| {
            a.timestamp()
                .cmp(&b.timestamp())
                .then_with(|| a.id().cmp(b.id()))
        });
        let mut doc = SearchDoc::default();
        for entry in sorted {
            let SessionEntry::Message { id, message, .. } = entry else {
                continue;
            };
            let (role, content) = match message.as_ref() {
                AgentMessage::User { content, .. } => (SnippetRole::User, content),
                AgentMessage::Assistant { content, .. } => (SnippetRole::Assistant, content),
                _ => continue,
            };
            let text = content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .filter(|text| !text.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if text.trim().is_empty() {
                continue;
            }
            for (start, end) in words(&text) {
                doc.vocabulary.insert(text[start..end].to_lowercase());
            }
            doc.messages.push(DocMessage {
                entry_id: id.clone(),
                role,
                text,
            });
        }
        doc
    }
}

/// Ranking tiers, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// The title holds the query as typed (case-insensitive).
    TitlePhrase = 0,
    /// Every word starts a title word.
    TitleWords = 1,
    /// Every word is in the title, some only with a typo.
    TitleFuzzy = 2,
    /// One message holds the query's words in order.
    ContentPhrase = 3,
    /// One message holds every word (exact or prefix).
    ContentWords = 4,
    /// Every word is somewhere — spread over messages or the title, or
    /// with a typo.
    ContentLoose = 5,
}

/// A half-open `[start, end)` range in UTF-16 code units.
pub type Range16 = [usize; 2];

/// A slice of the best-matching message.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Snippet {
    /// The message's entry id.
    pub entry_id: String,
    pub role: SnippetRole,
    /// About 160 characters around the first match, whitespace collapsed,
    /// with `…` where it was cut.
    pub text: String,
    /// The matched words in `text`, UTF-16 code units, sorted.
    pub ranges: Vec<Range16>,
}

/// How one session matched a query.
#[derive(Debug, Clone)]
pub struct DocMatch {
    pub tier: Tier,
    /// The matched words in the title, UTF-16 code units, sorted.
    pub title_ranges: Vec<Range16>,
    /// Absent when the title alone answered the query.
    pub snippet: Option<Snippet>,
}

/// A parsed query: its lowercase words and the phrase they make.
#[derive(Debug, Clone)]
pub struct Query {
    terms: Vec<String>,
    phrase: String,
}

/// The longest query accepted, in characters. A search box query is a few
/// words; anything near this is a paste, not a search.
pub const MAX_QUERY_CHARS: usize = 256;
/// The most words a query may hold. Matching builds a words-by-terms table
/// per message, so the term count is what bounds a search's memory.
pub const MAX_QUERY_TERMS: usize = 16;

impl Query {
    /// `Ok(None)` when the text holds no word characters; `Err` (the
    /// message for `session/invalid_request`) when it exceeds
    /// [`MAX_QUERY_CHARS`] or [`MAX_QUERY_TERMS`].
    pub fn parse(text: &str) -> Result<Option<Self>, String> {
        if text.chars().count() > MAX_QUERY_CHARS {
            return Err(format!("query is longer than {MAX_QUERY_CHARS} characters"));
        }
        let terms: Vec<String> = words(text)
            .into_iter()
            .map(|(start, end)| text[start..end].to_lowercase())
            .collect();
        if terms.len() > MAX_QUERY_TERMS {
            return Err(format!("query has more than {MAX_QUERY_TERMS} words"));
        }
        if terms.is_empty() {
            return Ok(None);
        }
        Ok(Some(Self {
            phrase: text.trim().to_lowercase(),
            terms,
        }))
    }
}

/// Match one session. `None` when some query word lands nowhere.
pub fn match_doc(query: &Query, title: &str, doc: &SearchDoc) -> Option<DocMatch> {
    // Each query word's matches among the title words and the transcript
    // vocabulary: word -> strength, so scanning a message is a lookup per
    // word rather than an edit distance per word.
    let title_words: Vec<String> = words(title)
        .into_iter()
        .map(|(s, e)| title[s..e].to_lowercase())
        .collect();
    let mut title_best = Vec::with_capacity(query.terms.len());
    let mut content_hits: Vec<HashMap<&str, Strength>> = Vec::with_capacity(query.terms.len());
    for term in &query.terms {
        title_best.push(
            title_words
                .iter()
                .filter_map(|word| strength(term, word))
                .max(),
        );
        let mut hits = HashMap::new();
        for word in &doc.vocabulary {
            if let Some(s) = strength(term, word) {
                hits.insert(word.as_str(), s);
            }
        }
        content_hits.push(hits);
    }

    let all_in_title = title_best.iter().all(Option::is_some);
    let everywhere = title_best
        .iter()
        .zip(&content_hits)
        .all(|(title, content)| title.is_some() || !content.is_empty());
    if !everywhere {
        return None;
    }

    let title_ranges = mark(title, |word| {
        query
            .terms
            .iter()
            .any(|term| strength(term, &word.to_lowercase()).is_some())
    });

    if all_in_title {
        let tier = if title.to_lowercase().contains(&query.phrase) {
            Tier::TitlePhrase
        } else if title_best.iter().all(|s| *s >= Some(Strength::Prefix)) {
            Tier::TitleWords
        } else {
            Tier::TitleFuzzy
        };
        return Some(DocMatch {
            tier,
            title_ranges,
            snippet: None,
        });
    }

    // The best message: a phrase beats every word beats most words; the
    // earliest such message wins a tie (it is usually the ask that names
    // the topic).
    let lookup = |word: &str| -> Vec<Option<Strength>> {
        let lower = word.to_lowercase();
        content_hits
            .iter()
            .map(|hits| hits.get(lower.as_str()).copied())
            .collect()
    };
    let mut best: Option<(u8, usize, usize)> = None; // (rank, covered, index)
    for (index, message) in doc.messages.iter().enumerate() {
        let spans = words(&message.text);
        let per_word: Vec<Vec<Option<Strength>>> = spans
            .iter()
            .map(|&(s, e)| lookup(&message.text[s..e]))
            .collect();
        let covered: Vec<Option<Strength>> = (0..query.terms.len())
            .map(|t| per_word.iter().filter_map(|w| w[t]).max())
            .collect();
        let count = covered.iter().filter(|s| s.is_some()).count();
        if count == 0 {
            continue;
        }
        let rank = if has_phrase(&per_word, query.terms.len()) {
            3
        } else if covered.iter().all(|s| *s >= Some(Strength::Prefix)) {
            2
        } else if count == query.terms.len() {
            1
        } else {
            0
        };
        if best.is_none_or(|(r, c, _)| (rank, count) > (r, c)) {
            best = Some((rank, count, index));
        }
    }

    let tier = match best {
        Some((3, _, _)) => Tier::ContentPhrase,
        Some((2, _, _)) => Tier::ContentWords,
        _ => Tier::ContentLoose,
    };
    let snippet = best.map(|(_, _, index)| {
        let message = &doc.messages[index];
        let spans = words(&message.text);
        let per_word: Vec<Vec<Option<Strength>>> = spans
            .iter()
            .map(|&(s, e)| lookup(&message.text[s..e]))
            .collect();
        let anchor = spans
            .get(densest(&per_word, query.terms.len()))
            .map_or(0, |&(s, _)| s);
        let (text, ranges) = snippet_of(&message.text, anchor, |word| {
            lookup(word).iter().any(Option::is_some)
        });
        Snippet {
            entry_id: message.entry_id.clone(),
            role: message.role,
            text,
            ranges,
        }
    });
    Some(DocMatch {
        tier,
        title_ranges,
        snippet,
    })
}

/// The word where the query's words cluster: the hit whose next few words
/// cover the most distinct query words (earliest on a tie). A message that
/// mentions `worker` in a path list long before "kanban ticket-worker"
/// opens its snippet on the latter.
fn densest(per_word: &[Vec<Option<Strength>>], terms: usize) -> usize {
    let reach = (terms * 3).max(8);
    let mut best = (0usize, 0usize); // (covered, index)
    for (index, word) in per_word.iter().enumerate() {
        if word.iter().all(Option::is_none) {
            continue;
        }
        let end = (index + reach).min(per_word.len());
        let covered = (0..terms)
            .filter(|&t| per_word[index..end].iter().any(|w| w[t].is_some()))
            .count();
        if covered > best.0 {
            best = (covered, index);
            if covered == terms {
                break;
            }
        }
    }
    best.1
}

/// Whether consecutive words match every query word in order, each at
/// prefix strength or better.
fn has_phrase(per_word: &[Vec<Option<Strength>>], terms: usize) -> bool {
    if terms < 2 {
        return per_word.iter().any(|w| w[0] >= Some(Strength::Prefix));
    }
    per_word.windows(terms).any(|window| {
        window
            .iter()
            .enumerate()
            .all(|(t, w)| w[t] >= Some(Strength::Prefix))
    })
}

/// How `term` matches `word` (both lowercase).
fn strength(term: &str, word: &str) -> Option<Strength> {
    if word == term {
        return Some(Strength::Exact);
    }
    if word.starts_with(term) {
        return Some(Strength::Prefix);
    }
    // Cheap rejections before any allocation: this runs once per
    // vocabulary word per query word.
    if term.chars().next() != word.chars().next() {
        return None;
    }
    let term_chars: Vec<char> = term.chars().collect();
    let budget = match term_chars.len() {
        0..=3 => return None,
        4..=7 => 1,
        _ => 2,
    };
    let word_chars: Vec<char> = word.chars().collect();
    if word_chars.len() + budget < term_chars.len() {
        return None;
    }
    // The whole word, or its start (a typo in a word still being typed:
    // `stremlin` for `streamlines`).
    let whole = word_chars.len().abs_diff(term_chars.len()) <= budget
        && within(&term_chars, &word_chars, budget);
    let head = word_chars.len() > term_chars.len()
        && (term_chars.len().saturating_sub(budget)..=term_chars.len() + budget)
            .filter(|&n| n <= word_chars.len() && n > 0)
            .any(|n| within(&term_chars, &word_chars[..n], budget));
    (whole || head).then_some(Strength::Fuzzy)
}

/// Optimal string alignment distance between `a` and `b` is at most
/// `budget` (a swap of neighbours counts as one edit).
fn within(a: &[char], b: &[char], budget: usize) -> bool {
    let (n, m) = (a.len(), b.len());
    if n.abs_diff(m) > budget {
        return false;
    }
    let mut prev2 = vec![0usize; m + 1];
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        cur[0] = i;
        let mut row_min = cur[0];
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut d = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d = d.min(prev2[j - 2] + 1);
            }
            cur[j] = d;
            row_min = row_min.min(d);
        }
        if row_min > budget {
            return false;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m] <= budget
}

/// Byte spans of the words in `text`: runs of alphanumeric characters.
fn words(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = None;
    for (at, ch) in text.char_indices() {
        if ch.is_alphanumeric() {
            start.get_or_insert(at);
        } else if let Some(s) = start.take() {
            spans.push((s, at));
        }
    }
    if let Some(s) = start {
        spans.push((s, text.len()));
    }
    spans
}

/// UTF-16 ranges of the words in `text` that `hit` accepts.
fn mark(text: &str, hit: impl Fn(&str) -> bool) -> Vec<Range16> {
    let mut ranges = Vec::new();
    let mut units = 0usize;
    let mut cursor = 0usize;
    for (start, end) in words(text) {
        units += text[cursor..start].encode_utf16().count();
        let len = text[start..end].encode_utf16().count();
        if hit(&text[start..end]) {
            ranges.push([units, units + len]);
        }
        units += len;
        cursor = end;
    }
    ranges
}

/// Characters of context kept before the first match.
const SNIPPET_BEFORE: usize = 40;
/// Total characters a snippet aims for.
const SNIPPET_LENGTH: usize = 160;

/// A window of `text` opening a little before byte offset `first` (a word
/// start), whitespace collapsed to single spaces, with the accepted words
/// marked.
fn snippet_of(text: &str, first: usize, hit: impl Fn(&str) -> bool) -> (String, Vec<Range16>) {
    let spans = words(text);

    // Walk back SNIPPET_BEFORE characters, then forward to a word start.
    let mut start = text[..first]
        .char_indices()
        .rev()
        .nth(SNIPPET_BEFORE.saturating_sub(1))
        .map_or(0, |(at, _)| at);
    if start > 0 {
        start = spans
            .iter()
            .map(|&(s, _)| s)
            .find(|&s| s >= start && s <= first)
            .unwrap_or(first);
    }
    let mut end = text[start..]
        .char_indices()
        .nth(SNIPPET_LENGTH)
        .map_or(text.len(), |(at, _)| start + at);
    if end < text.len() {
        // Do not cut a word in half: end where the last whole word ends.
        end = spans
            .iter()
            .map(|&(_, e)| e)
            .rfind(|&e| e > start && e <= end)
            .unwrap_or(end);
    }
    // The match the window opens on is always inside it, even when that
    // word alone runs past SNIPPET_LENGTH (a long path or identifier); the
    // reader truncates the row, never the reason it matched.
    if let Some(&(_, anchor_end)) = spans.iter().find(|&&(s, _)| s == first) {
        end = end.max(anchor_end);
    }

    let marked: Vec<(usize, usize)> = spans
        .iter()
        .copied()
        .filter(|&(s, e)| s >= start && e <= end && hit(&text[s..e]))
        .collect();

    let mut out = String::new();
    let mut ranges: Vec<Range16> = Vec::new();
    let mut units = 0usize;
    if start > 0 {
        out.push('…');
        units += 1;
    }
    let syntax = markdown_syntax(text);
    let mut last_space = true;
    let mut open: Option<usize> = None;
    let mut marks = marked.iter().peekable();
    for (offset, ch) in text[start..end].char_indices() {
        let at = start + offset;
        if syntax.contains(&at) {
            continue;
        }
        if let Some(&&(s, _)) = marks.peek()
            && s == at
        {
            open = Some(units);
        }
        if ch.is_whitespace() {
            if !last_space {
                out.push(' ');
                units += 1;
            }
            last_space = true;
        } else {
            out.push(ch);
            units += ch.len_utf16();
            last_space = false;
        }
        if let Some(&&(_, e)) = marks.peek()
            && e == at + ch.len_utf8()
        {
            if let Some(from) = open.take() {
                ranges.push([from, units]);
            }
            marks.next();
        }
    }
    let trimmed = out.trim_end().len();
    out.truncate(trimmed);
    if end < text.len() {
        out.push('…');
    }
    (out, ranges)
}

/// Byte offsets of the markdown syntax a snippet drops, so it reads as
/// prose: every backtick, `**`/`__`-style runs of two or more `*`, and at a
/// line's start the heading `#`s, quote `>`s and list bullets (`-`, `*`,
/// `+` followed by a space).
fn markdown_syntax(text: &str) -> HashSet<usize> {
    let mut drop = HashSet::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut line_start = true;
    let mut i = 0;
    while i < chars.len() {
        let (at, ch) = chars[i];
        if ch == '\n' {
            line_start = true;
            i += 1;
            continue;
        }
        if line_start && (ch == ' ' || ch == '\t') {
            i += 1;
            continue;
        }
        if line_start {
            line_start = false;
            let next = chars.get(i + 1).map(|&(_, c)| c);
            match ch {
                '#' | '>' => {
                    while let Some(&(at, c)) = chars.get(i)
                        && c == ch
                    {
                        drop.insert(at);
                        i += 1;
                    }
                    continue;
                }
                '-' | '*' | '+' if next == Some(' ') => {
                    drop.insert(at);
                    i += 1;
                    continue;
                }
                _ => {}
            }
        }
        if ch == '`' {
            drop.insert(at);
        } else if ch == '*' {
            let run = chars[i..].iter().take_while(|&&(_, c)| c == '*').count();
            if run >= 2 {
                for &(at, _) in &chars[i..i + run] {
                    drop.insert(at);
                }
            }
            i += run;
            continue;
        }
        i += 1;
    }
    drop
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(text: &str) -> Query {
        Query::parse(text)
            .expect("query within limits")
            .expect("query has words")
    }

    fn doc(messages: &[(&str, SnippetRole)]) -> SearchDoc {
        let mut doc = SearchDoc::default();
        for (index, (text, role)) in messages.iter().enumerate() {
            for (s, e) in words(text) {
                doc.vocabulary.insert(text[s..e].to_lowercase());
            }
            doc.messages.push(DocMessage {
                entry_id: format!("e{index}"),
                role: *role,
                text: (*text).to_string(),
            });
        }
        doc
    }

    fn slice16(text: &str, [from, to]: Range16) -> String {
        let units: Vec<u16> = text.encode_utf16().collect();
        String::from_utf16(&units[from..to]).expect("valid utf-16 slice")
    }

    #[test]
    fn strength_covers_exact_prefix_and_typos() {
        assert_eq!(strength("login", "login"), Some(Strength::Exact));
        assert_eq!(strength("stream", "streamlines"), Some(Strength::Prefix));
        assert_eq!(strength("stremline", "streamline"), Some(Strength::Fuzzy));
        assert_eq!(strength("stremlin", "streamlines"), Some(Strength::Fuzzy));
        assert_eq!(strength("teh", "the"), None, "short words never fuzz");
        assert_eq!(strength("recieve", "receive"), Some(Strength::Fuzzy));
        assert_eq!(strength("xogin", "login"), None, "first letter must hold");
        assert_eq!(strength("database", "dashboard"), None);
    }

    #[test]
    fn title_hits_rank_by_how_closely_they_match() {
        let empty = SearchDoc::default();
        let m = match_doc(&query("setup ade"), "Streamline the setup ADE", &empty).unwrap();
        assert_eq!(m.tier, Tier::TitlePhrase);
        let m = match_doc(&query("ade setup"), "Streamline the setup ADE", &empty).unwrap();
        assert_eq!(m.tier, Tier::TitleWords);
        let m = match_doc(&query("stremline"), "Streamline the setup", &empty).unwrap();
        assert_eq!(m.tier, Tier::TitleFuzzy);
        assert_eq!(
            slice16("Streamline the setup", m.title_ranges[0]),
            "Streamline"
        );
        assert!(match_doc(&query("kanban"), "Streamline the setup", &empty).is_none());
    }

    #[test]
    fn content_hits_carry_a_marked_snippet() {
        let d = doc(&[
            ("hello there", SnippetRole::User),
            (
                "The onboarding wizard now asks for keys first, then the browser.",
                SnippetRole::Assistant,
            ),
        ]);
        let m = match_doc(&query("onboarding wizard"), "Misc", &d).unwrap();
        assert_eq!(m.tier, Tier::ContentPhrase);
        let snippet = m.snippet.unwrap();
        assert_eq!(snippet.entry_id, "e1");
        assert_eq!(snippet.role, SnippetRole::Assistant);
        let marked: Vec<String> = snippet
            .ranges
            .iter()
            .map(|r| slice16(&snippet.text, *r))
            .collect();
        assert_eq!(marked, ["onboarding", "wizard"]);

        let m = match_doc(&query("browser keys"), "Misc", &d).unwrap();
        assert_eq!(m.tier, Tier::ContentWords);
        let m = match_doc(&query("onbaording"), "Misc", &d).unwrap();
        assert_eq!(m.tier, Tier::ContentLoose);
        assert!(match_doc(&query("kanban"), "Misc", &d).is_none());
    }

    #[test]
    fn snippets_open_where_the_words_cluster() {
        let filler = "lorem ".repeat(40);
        let text = format!(
            "paths: ade-worker-design, worker-ui {filler} use the kanban ticket-worker skill"
        );
        let d = doc(&[(text.as_str(), SnippetRole::User)]);
        let m = match_doc(&query("kanban ticket worker"), "Misc", &d).unwrap();
        let snippet = m.snippet.unwrap();
        assert!(
            snippet.text.contains("kanban ticket-worker"),
            "{}",
            snippet.text
        );
        assert!(!snippet.text.contains("ade-worker"), "{}", snippet.text);
    }

    #[test]
    fn words_may_split_between_title_and_messages() {
        let d = doc(&[("we fixed the redirect", SnippetRole::User)]);
        let m = match_doc(&query("login redirect"), "Login page", &d).unwrap();
        assert_eq!(m.tier, Tier::ContentLoose);
        assert_eq!(m.snippet.unwrap().entry_id, "e0");
    }

    #[test]
    fn snippets_collapse_whitespace_and_cut_long_text() {
        let long = format!(
            "{}\n\n  the needle sits here   {}",
            "word ".repeat(60),
            "tail ".repeat(60)
        );
        let at = long.find("needle").unwrap();
        let (text, ranges) = snippet_of(&long, at, |w| w == "needle");
        assert!(text.starts_with('…'), "{text}");
        assert!(text.ends_with('…'), "{text}");
        assert!(!text.contains("  "), "{text}");
        assert_eq!(ranges.len(), 1);
        assert_eq!(slice16(&text, ranges[0]), "needle");
        assert!(text.chars().count() <= SNIPPET_LENGTH + 2);
    }

    #[test]
    fn snippets_read_as_prose_not_markdown() {
        let text = "## Where to start\n- `onboarding.yaml`: the **setup** flow\n> quoted 2 * 3";
        let (out, ranges) = snippet_of(text, 0, |w| w == "onboarding");
        assert_eq!(
            out,
            "Where to start onboarding.yaml: the setup flow quoted 2 * 3"
        );
        assert_eq!(slice16(&out, ranges[0]), "onboarding");
    }

    #[test]
    fn the_anchor_word_stays_in_the_window_however_long() {
        let text = format!("before needle{}", "x".repeat(200));
        let at = text.find("needle").unwrap();
        let (out, ranges) = snippet_of(&text, at, |w| w.starts_with("needle"));
        assert_eq!(ranges.len(), 1, "{out}");
        assert!(slice16(&out, ranges[0]).starts_with("needle"));
    }

    #[test]
    fn queries_past_the_limits_are_refused() {
        assert!(Query::parse(&"a".repeat(MAX_QUERY_CHARS + 1)).is_err());
        let many = vec!["word"; MAX_QUERY_TERMS + 1].join(" ");
        assert!(Query::parse(&many).is_err());
        let most = vec!["word"; MAX_QUERY_TERMS].join(" ");
        assert!(Query::parse(&most).unwrap().is_some());
        assert!(Query::parse("  ...  ").unwrap().is_none());
    }

    #[test]
    fn ranges_count_utf16_units() {
        let (text, ranges) = snippet_of("🚀 ship the rocket", 0, |w| w == "rocket");
        assert_eq!(slice16(&text, ranges[0]), "rocket");
        let ranges = mark("🚀 Rocket launch", |w| w == "Rocket");
        assert_eq!(slice16("🚀 Rocket launch", ranges[0]), "Rocket");
    }

    #[test]
    fn from_entries_keeps_user_and_assistant_text_only() {
        let entries: Vec<SessionEntry> = serde_json::from_value(serde_json::json!([
            {
                "kind": "message", "id": "u1", "parent_id": null, "timestamp": 1,
                "message": { "role": "user", "timestamp": 1,
                    "content": [{ "type": "text", "text": "find the streamline chat" }] }
            },
            {
                "kind": "message", "id": "a1", "parent_id": "u1", "timestamp": 2,
                "message": { "role": "assistant", "timestamp": 2, "model": "m", "provider": "p",
                    "stop_reason": "end",
                    "content": [
                        { "type": "thinking", "text": "secretword" },
                        { "type": "text", "text": "Here it is." }
                    ] }
            }
        ]))
        .expect("entries deserialize");
        let doc = SearchDoc::from_entries(&entries);
        assert_eq!(doc.messages.len(), 2);
        assert!(doc.vocabulary.contains("streamline"));
        assert!(!doc.vocabulary.contains("secretword"));
    }
}
