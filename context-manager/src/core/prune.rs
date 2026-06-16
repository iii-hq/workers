//! Function-output pruning (context-manager.md § context::prune):
//! replace verbose `function_result` outputs with placeholders, newest
//! to oldest, freeing tokens outside a protected recent window.
//!
//! Structural invariant (§ Structural invariants): **prune replaces,
//! never removes** — the message, its `function_call_id` linkage, and
//! the message order all survive; only the content is rewritten to a
//! single text placeholder carrying the freed size.
//!
//! Eligibility (ported from harness `context-compaction/prune.ts`):
//! - the most recent two user turns are never touched;
//! - outputs inside the newest `protect_recent_tokens` window are kept;
//! - `protected_functions` are never pruned;
//! - outputs whose text is at or under `max_output_chars` are not
//!   "verbose" and stay (pruning them frees almost nothing);
//! - when everything prunable frees under `min_free_tokens`, nothing is
//!   touched at all (a no-op beats a destroyed-but-still-over context).

use crate::core::estimate::Estimator;
use crate::types::{AgentMessage, ContentBlock, Role};

/// Recent user turns that are always exempt, independent of the token
/// window (prior-art constant, not operator-tunable).
const PROTECTED_USER_TURNS: usize = 2;

#[derive(Debug, Clone)]
pub struct PruneParams {
    /// Newest function-output tokens kept verbatim.
    pub protect_recent_tokens: u64,
    /// Skip the whole pass when it would free less than this.
    pub min_free_tokens: u64,
    /// Outputs at or under this many chars are not considered verbose.
    pub max_output_chars: usize,
    /// `function_id`s whose outputs are never pruned.
    pub protected_functions: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PruneStats {
    /// Estimated tokens freed.
    pub pruned_tokens: u64,
    /// Number of outputs replaced with placeholders.
    pub pruned_parts: u64,
    /// Number of prunable outputs examined (excludes protected
    /// functions and the always-exempt recent turns).
    pub scanned_parts: u64,
}

/// The spec's placeholder shape: `[output pruned: was ~N tokens]`.
pub fn placeholder(tokens: u64) -> String {
    format!("[output pruned: was ~{tokens} tokens]")
}

fn text_of(blocks: &[ContentBlock]) -> String {
    let mut out = String::new();
    for block in blocks {
        if let ContentBlock::Text { text } = block {
            out.push_str(text);
        }
    }
    out
}

/// Rewrite verbose outputs in place. Returns the stats; `messages` is
/// mutated only when the pass actually runs (the `min_free_tokens`
/// guard fires before any rewrite).
pub fn prune(
    messages: &mut [AgentMessage],
    params: &PruneParams,
    estimator: &dyn Estimator,
) -> PruneStats {
    let mut scanned: u64 = 0;
    let mut window_tokens: u64 = 0;
    let mut user_turns = 0usize;
    // (message index, estimated tokens of its text content)
    let mut queue: Vec<(usize, u64)> = Vec::new();

    for idx in (0..messages.len()).rev() {
        let message = &messages[idx];
        if message.role() == Role::User {
            user_turns += 1;
            continue;
        }
        if user_turns < PROTECTED_USER_TURNS {
            continue;
        }
        let AgentMessage::FunctionResult {
            function_id,
            content,
            ..
        } = message
        else {
            continue;
        };
        if params.protected_functions.iter().any(|f| f == function_id) {
            continue;
        }

        let text = text_of(content);
        let tokens = estimator.text(&text);
        scanned += 1;
        window_tokens += tokens;
        if window_tokens <= params.protect_recent_tokens {
            continue;
        }
        if text.len() <= params.max_output_chars {
            continue;
        }
        queue.push((idx, tokens));
    }

    // Net tokens freed: each pruned output is replaced by a placeholder
    // that itself costs a few tokens, so the real saving is the original
    // size minus that placeholder. Gauge `min_free_tokens` on the net.
    let pruned_tokens: u64 = queue
        .iter()
        .map(|(_, tokens)| tokens.saturating_sub(estimator.text(&placeholder(*tokens))))
        .sum();
    if pruned_tokens < params.min_free_tokens {
        return PruneStats {
            pruned_tokens: 0,
            pruned_parts: 0,
            scanned_parts: scanned,
        };
    }

    for (idx, tokens) in &queue {
        messages[*idx].set_content(vec![ContentBlock::Text {
            text: placeholder(*tokens),
        }]);
    }

    PruneStats {
        pruned_tokens,
        pruned_parts: queue.len() as u64,
        scanned_parts: scanned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::estimate::HeuristicEstimator;
    use serde_json::json;

    fn user(text: &str, ts: i64) -> AgentMessage {
        serde_json::from_value(json!({
            "role": "user", "content": [{ "type": "text", "text": text }], "timestamp": ts
        }))
        .unwrap()
    }

    fn result(function_id: &str, chars: usize, ts: i64) -> AgentMessage {
        serde_json::from_value(json!({
            "role": "function_result", "function_call_id": format!("c{ts}"),
            "function_id": function_id,
            "content": [{ "type": "text", "text": "x".repeat(chars) }],
            "timestamp": ts
        }))
        .unwrap()
    }

    fn params() -> PruneParams {
        PruneParams {
            protect_recent_tokens: 100,
            min_free_tokens: 1,
            max_output_chars: 100,
            protected_functions: vec![],
        }
    }

    /// History: old verbose output, then two user turns (exempt zone).
    fn history() -> Vec<AgentMessage> {
        vec![
            user("first", 1),
            result("shell::run", 8_000, 2), // ~2000 tokens, old
            user("second", 3),
            result("shell::run", 8_000, 4), // inside the 2-user-turn exemption
            user("third", 5),
        ]
    }

    #[test]
    fn prunes_old_verbose_output_and_keeps_recent_turns() {
        let mut messages = history();
        let stats = prune(&mut messages, &params(), &HeuristicEstimator);
        assert_eq!(stats.pruned_parts, 1);
        // Net of the placeholder written back: 2000 minus the tokens of
        // "[output pruned: was ~2000 tokens]" (33 chars / 4 = 8).
        assert_eq!(stats.pruned_tokens, 1_992);
        // Oldest output replaced...
        assert_eq!(
            messages[1].content(),
            &[ContentBlock::Text {
                text: placeholder(2_000)
            }]
        );
        // ...the one inside the last two user turns untouched.
        assert_eq!(text_of(messages[3].content()).len(), 8_000);
    }

    #[test]
    fn replaces_but_never_removes() {
        let mut messages = history();
        let before = messages.len();
        prune(&mut messages, &params(), &HeuristicEstimator);
        assert_eq!(messages.len(), before);
        let AgentMessage::FunctionResult {
            function_call_id, ..
        } = &messages[1]
        else {
            panic!("message kind changed");
        };
        assert_eq!(function_call_id, "c2");
    }

    #[test]
    fn min_free_guard_skips_everything() {
        let mut messages = history();
        let mut p = params();
        p.min_free_tokens = 1_000_000;
        let stats = prune(&mut messages, &p, &HeuristicEstimator);
        assert_eq!(stats.pruned_parts, 0);
        assert_eq!(stats.pruned_tokens, 0);
        assert_eq!(stats.scanned_parts, 1);
        assert_eq!(text_of(messages[1].content()).len(), 8_000);
    }

    #[test]
    fn protected_functions_are_exempt() {
        let mut messages = history();
        let mut p = params();
        p.protected_functions = vec!["shell::run".into()];
        let stats = prune(&mut messages, &p, &HeuristicEstimator);
        assert_eq!(stats.pruned_parts, 0);
        assert_eq!(stats.scanned_parts, 0);
    }

    #[test]
    fn small_outputs_are_not_verbose() {
        let mut messages = vec![
            user("first", 1),
            result("shell::run", 50, 2), // tiny, under max_output_chars
            user("second", 3),
            user("third", 4),
        ];
        let mut p = params();
        p.protect_recent_tokens = 0;
        let stats = prune(&mut messages, &p, &HeuristicEstimator);
        assert_eq!(stats.pruned_parts, 0);
        assert_eq!(stats.scanned_parts, 1);
        assert_eq!(text_of(messages[1].content()).len(), 50);
    }

    #[test]
    fn protect_window_counts_newest_first() {
        // Two old outputs; the window covers the newer one only.
        let mut messages = vec![
            user("a", 1),
            result("f", 4_000, 2), // older: outside window once newer fills it
            result("f", 4_000, 3), // newer: inside the 1000-token window
            user("b", 4),
            user("c", 5),
        ];
        let mut p = params();
        p.protect_recent_tokens = 1_000;
        let stats = prune(&mut messages, &p, &HeuristicEstimator);
        assert_eq!(stats.pruned_parts, 1);
        assert_eq!(text_of(messages[2].content()).len(), 4_000); // newer kept
        assert!(text_of(messages[1].content()).starts_with("[output pruned"));
    }

    #[test]
    fn idempotent_re_prune_is_a_no_op() {
        let mut messages = history();
        let first = prune(&mut messages, &params(), &HeuristicEstimator);
        assert_eq!(first.pruned_parts, 1);
        let second = prune(&mut messages, &params(), &HeuristicEstimator);
        // The placeholder is tiny, so nothing is verbose any more.
        assert_eq!(second.pruned_parts, 0);
    }
}
