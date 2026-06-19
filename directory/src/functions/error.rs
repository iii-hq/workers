//! Prose-first, self-correcting error messages for `directory::*` handlers.
//!
//! Over the iii bus a handler error is a plain string (`IIIError::Handler`),
//! which the engine re-wraps as
//! `{ code: "invocation_failed", message: "handler error: <string>" }`.
//! Any JSON we put in that string therefore arrives DOUBLE-escaped and is
//! effectively unreadable to an LLM agent — it would have to strip two
//! prefixes and parse two JSON layers to reach a `fix` block. So instead of a
//! structured envelope these builders emit a single self-sufficient sentence
//! the agent can act on in ONE read:
//!
//! ```text
//! D110 not_found: skill "database/query" does not exist. \
//!   Did you mean: database/iii-database/query, database/index. \
//!   Next: call directory::skills::list to browse skill ids; \
//!   or directory::skills::index to see the per-worker overview.
//! ```
//!
//! The leading `<code>` token (e.g. `D110`) and the `not_found` /
//! `invalid_input` class word stay stable and greppable, so a non-LLM consumer
//! can still branch on them without parsing natural language.

use std::fmt::Write as _;

/// A ranked candidate for a missed lookup. `title` / `kind` / `score` are kept
/// for the skills ranker and its tests; the rendered message uses `id` only
/// (the agent retries with the id).
#[derive(Debug, Clone)]
pub struct SuggestEntry {
    pub id: String,
    pub title: String,
    pub kind: Option<String>,
    /// Ranking score (`shared_segments * 100 - levenshtein`). Higher is closer.
    pub score: i32,
}

/// A "call this next" pointer rendered into the recovery sentence.
#[derive(Debug, Clone, Copy)]
pub struct NextAction {
    pub function: &'static str,
    pub why: &'static str,
}

impl NextAction {
    pub const fn new(function: &'static str, why: &'static str) -> Self {
        Self { function, why }
    }
}

/// Build a prose "not found" recovery message.
///
/// * `code`       — stable token, e.g. `"D110"`.
/// * `kind`       — what was looked up, e.g. `"skill"` / `"prompt"` / `"worker"`.
/// * `missed`     — the id/name the caller asked for.
/// * `candidates` — ranked closest ids/names (may be empty).
/// * `next`       — ordered "call this next" pointers (may be empty).
pub fn not_found_message(
    code: &str,
    kind: &str,
    missed: &str,
    candidates: &[String],
    next: &[NextAction],
) -> String {
    let mut msg = format!("{code} not_found: {kind} {missed:?} does not exist.");
    if !candidates.is_empty() {
        let _ = write!(msg, " Did you mean: {}.", candidates.join(", "));
    }
    append_next(&mut msg, next);
    msg
}

/// Build a prose "invalid input" recovery message (a bad argument, not a miss).
///
/// `problem` should be a complete sentence (it is emitted verbatim after the
/// class word). Example: `invalid_input_message("D111", "id may not be empty.", &[...])`.
pub fn invalid_input_message(code: &str, problem: &str, next: &[NextAction]) -> String {
    let mut msg = format!("{code} invalid_input: {problem}");
    append_next(&mut msg, next);
    msg
}

fn append_next(msg: &mut String, next: &[NextAction]) {
    if next.is_empty() {
        return;
    }
    let parts: Vec<String> = next
        .iter()
        .map(|a| format!("call {} to {}", a.function, a.why))
        .collect();
    let _ = write!(msg, " Next: {}.", parts.join("; or "));
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKILL_NEXT: &[NextAction] = &[
        NextAction::new("directory::skills::list", "browse skill ids"),
        NextAction::new("directory::skills::index", "see the per-worker overview"),
    ];

    #[test]
    fn not_found_carries_code_class_id_and_next_actions() {
        let msg = not_found_message("D110", "skill", "sandbox/create", &[], SKILL_NEXT);
        assert!(msg.starts_with("D110 not_found:"), "got: {msg}");
        assert!(msg.contains("\"sandbox/create\""), "got: {msg}");
        assert!(msg.contains("directory::skills::list"), "got: {msg}");
        assert!(msg.contains("directory::skills::index"), "got: {msg}");
        // No candidates -> no misleading "Did you mean".
        assert!(!msg.contains("Did you mean"), "got: {msg}");
    }

    #[test]
    fn not_found_lists_candidates_in_order_when_present() {
        let candidates = vec!["sandbox/index".to_string(), "sandbox/exec".to_string()];
        let msg = not_found_message("D110", "skill", "sandbox/create", &candidates, SKILL_NEXT);
        assert!(
            msg.contains("Did you mean: sandbox/index, sandbox/exec."),
            "got: {msg}"
        );
        // Candidates come before the Next: pointer.
        assert!(
            msg.find("Did you mean").unwrap() < msg.find("Next:").unwrap(),
            "got: {msg}"
        );
    }

    #[test]
    fn invalid_input_carries_code_and_next() {
        let next = &[NextAction::new("directory::skills::list", "see valid ids")];
        let msg = invalid_input_message("D111", "id may not be empty.", next);
        assert!(
            msg.starts_with("D111 invalid_input: id may not be empty."),
            "got: {msg}"
        );
        assert!(
            msg.contains("Next: call directory::skills::list to see valid ids."),
            "got: {msg}"
        );
    }
}
