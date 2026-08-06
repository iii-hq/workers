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
//!   touched at all. `context::assemble` may subsequently use the
//!   unconditional emergency pass to enforce its hard budget.

use crate::core::estimate::Estimator;
use crate::types::{AgentMessage, ContentBlock, Role};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Recent user turns that are always exempt, independent of the token
/// window (prior-art constant, not operator-tunable).
const PROTECTED_USER_TURNS: usize = 2;

/// Maximum number of Unicode scalar values copied from an oversized
/// result into its emergency reference.
const EMERGENCY_PREVIEW_CHARS: usize = 160;

const EMERGENCY_RETRIEVAL_HINT: &str =
    "The original result remains in the session transcript; retrieve it by function_call_id.";

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
    prune_impl(messages, None, params, estimator)
}

/// [`prune`] that also keeps a caller-held per-message size memo in
/// sync: `sizes[i]` is re-estimated for every rewritten message, so the
/// memo stays exactly what a from-scratch recount would produce.
pub fn prune_with_sizes(
    messages: &mut [AgentMessage],
    sizes: &mut [u64],
    params: &PruneParams,
    estimator: &dyn Estimator,
) -> PruneStats {
    debug_assert_eq!(messages.len(), sizes.len());
    prune_impl(messages, Some(sizes), params, estimator)
}

fn prune_impl(
    messages: &mut [AgentMessage],
    mut sizes: Option<&mut [u64]>,
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
        if let Some(sizes) = sizes.as_deref_mut() {
            // Re-estimate the rewritten message — not freed-token
            // arithmetic — so the memo matches a from-scratch recount.
            sizes[*idx] = estimator.message(&messages[*idx]);
        }
    }

    PruneStats {
        pruned_tokens,
        pruned_parts: queue.len() as u64,
        scanned_parts: scanned,
    }
}

/// Unconditionally reduce the largest function results until at least
/// `required_tokens` have been freed or no result can shrink further.
///
/// Unlike [`prune`], this emergency pass has no recent-turn window,
/// protected-function list, size threshold, or minimum-free guard. It
/// preserves every message and the call/result identity fields while
/// replacing both rendered content and opaque details with a bounded,
/// deterministic reference to the original transcript entry.
pub fn emergency_reduce(
    messages: &mut [AgentMessage],
    required_tokens: u64,
    estimator: &dyn Estimator,
) -> PruneStats {
    emergency_reduce_impl(messages, None, required_tokens, estimator)
}

/// [`emergency_reduce`] that keeps a caller-held per-message size memo
/// in sync (see [`prune_with_sizes`]).
pub fn emergency_reduce_with_sizes(
    messages: &mut [AgentMessage],
    sizes: &mut [u64],
    required_tokens: u64,
    estimator: &dyn Estimator,
) -> PruneStats {
    debug_assert_eq!(messages.len(), sizes.len());
    emergency_reduce_impl(messages, Some(sizes), required_tokens, estimator)
}

fn emergency_reduce_impl(
    messages: &mut [AgentMessage],
    mut sizes: Option<&mut [u64]>,
    required_tokens: u64,
    estimator: &dyn Estimator,
) -> PruneStats {
    let mut candidates: Vec<(usize, u64)> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| matches!(message, AgentMessage::FunctionResult { .. }))
        .map(|(idx, message)| (idx, estimator.message(message)))
        .collect();

    // Stable tie-break by transcript order keeps the reduction fully
    // deterministic when multiple results have the same estimate.
    candidates.sort_by(|(left_idx, left_tokens), (right_idx, right_tokens)| {
        right_tokens
            .cmp(left_tokens)
            .then_with(|| left_idx.cmp(right_idx))
    });

    let mut stats = PruneStats {
        scanned_parts: candidates.len() as u64,
        ..PruneStats::default()
    };

    for (idx, original_tokens) in candidates {
        if stats.pruned_tokens >= required_tokens {
            break;
        }

        let replacement = emergency_reference(&messages[idx], original_tokens);
        let replacement_tokens = estimator.message(&replacement);
        let freed = original_tokens.saturating_sub(replacement_tokens);
        if freed == 0 {
            continue;
        }

        messages[idx] = replacement;
        if let Some(sizes) = sizes.as_deref_mut() {
            sizes[idx] = replacement_tokens;
        }
        stats.pruned_tokens = stats.pruned_tokens.saturating_add(freed);
        stats.pruned_parts += 1;
    }

    stats
}

fn emergency_reference(message: &AgentMessage, original_tokens: u64) -> AgentMessage {
    let AgentMessage::FunctionResult {
        function_call_id,
        function_id,
        content,
        details,
        is_error,
        timestamp,
    } = message
    else {
        unreachable!("emergency references are only built for function results");
    };

    let serialized = serde_json::to_vec(message)
        .expect("serializing an AgentMessage containing serde_json::Value cannot fail");
    let sha256 = format!("{:x}", Sha256::digest(&serialized));
    let preview = emergency_preview(content, details);
    let reference = json!({
        "kind": "function_result_reference",
        "function_id": function_id,
        "function_call_id": function_call_id,
        "original_bytes": serialized.len(),
        "original_estimated_tokens": original_tokens,
        "sha256": sha256,
        "preview": preview,
        "retrieval_hint": EMERGENCY_RETRIEVAL_HINT,
    });
    let rendered = format!(
        "[function result reduced for context budget] {}",
        serde_json::to_string(&reference)
            .expect("serializing a function-result reference cannot fail")
    );

    AgentMessage::FunctionResult {
        function_call_id: function_call_id.clone(),
        function_id: function_id.clone(),
        content: vec![ContentBlock::Text { text: rendered }],
        details: json!({ "context_reference": reference }),
        is_error: *is_error,
        timestamp: *timestamp,
    }
}

fn emergency_preview(content: &[ContentBlock], details: &Value) -> String {
    let text = text_of(content);
    let source = if text.is_empty() {
        serde_json::to_string(details).unwrap_or_else(|_| "<unavailable>".into())
    } else {
        text
    };
    let mut chars = source.chars();
    let mut preview: String = chars.by_ref().take(EMERGENCY_PREVIEW_CHARS).collect();
    if chars.next().is_some() {
        preview.push('\u{2026}');
    }
    preview
}

/// Serialized `details` larger than this on a capped result are replaced
/// with a bounded reference. `details` never crosses the provider wire
/// (see estimate.rs), so this bounds worker-to-worker payloads, not tokens.
const MAX_CAPPED_DETAILS_BYTES: usize = 2_048;

/// Stats from the unconditional per-result cap pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CapStats {
    /// Estimated tokens freed (message-estimate delta).
    pub capped_tokens: u64,
    /// Number of results rewritten.
    pub capped_parts: u64,
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Unconditionally rewrite any single function result whose text estimates
/// over `max_result_tokens` to a bounded head + marker + tail view
/// (context-manager.md § context::assemble). Applies to every result — any
/// age, protected or not, error or not: it is a generous ceiling, like the
/// emergency pass, not a policy prune. The rewrite targets 90% of the cap
/// so the output re-estimates under the threshold: the pass is idempotent
/// without a fixpoint loop, and deterministic (no call-varying content) so
/// identical histories assemble byte-identically across calls.
pub fn cap_results_with_sizes(
    messages: &mut [AgentMessage],
    sizes: &mut [u64],
    max_result_tokens: u64,
    estimator: &dyn Estimator,
) -> CapStats {
    debug_assert_eq!(messages.len(), sizes.len());
    let mut stats = CapStats::default();
    for idx in 0..messages.len() {
        let AgentMessage::FunctionResult {
            function_id,
            content,
            ..
        } = &messages[idx]
        else {
            continue;
        };
        let text = text_of(content);
        let tokens = estimator.text(&text);
        if tokens <= max_result_tokens {
            continue;
        }
        let function_id = function_id.clone();

        // Chars kept: scale the text down to 90% of the cap, preserving the
        // text's own chars-per-token ratio. u64 math: len ≤ ~16MB, cap ≤
        // ~10^6 — the product stays far under u64::MAX.
        let keep_chars = ((text.len() as u64) * max_result_tokens * 9 / (tokens * 10)) as usize;
        let head_budget = keep_chars * 6 / 10;
        let tail_budget = keep_chars - head_budget;
        let head_end = floor_char_boundary(&text, head_budget);
        let tail_start =
            ceil_char_boundary(&text, text.len().saturating_sub(tail_budget)).max(head_end);
        let marker = format!(
            "\n[…result capped: was ~{tokens} tokens; middle omitted; re-call {function_id} for the full data]\n"
        );
        let capped = format!("{}{}{}", &text[..head_end], marker, &text[tail_start..]);

        let AgentMessage::FunctionResult {
            content, details, ..
        } = &mut messages[idx]
        else {
            unreachable!("matched FunctionResult above");
        };
        *content = vec![ContentBlock::Text { text: capped }];
        let denied = details.get("status").and_then(Value::as_str) == Some("denied");
        if !denied
            && serde_json::to_string(&*details)
                .map(|s| s.len())
                .unwrap_or(0)
                > MAX_CAPPED_DETAILS_BYTES
        {
            *details = json!({
                "context_capped": { "original_estimated_tokens": tokens }
            });
        }

        let before = sizes[idx];
        sizes[idx] = estimator.message(&messages[idx]);
        stats.capped_tokens = stats
            .capped_tokens
            .saturating_add(before.saturating_sub(sizes[idx]));
        stats.capped_parts += 1;
    }
    stats
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

    #[test]
    fn emergency_reduces_latest_protected_result_and_preserves_pairing() {
        let call: AgentMessage = serde_json::from_value(json!({
            "role": "assistant",
            "content": [{
                "type": "function_call", "id": "c2",
                "function_id": "protected::lookup", "arguments": {}
            }],
            "stop_reason": "function_call", "model": "m", "provider": "p", "timestamp": 1
        }))
        .unwrap();
        let latest = result("protected::lookup", 40_000, 2);
        let mut messages = vec![call.clone(), latest];

        let mut normal_params = params();
        normal_params.protected_functions = vec!["protected::lookup".into()];
        normal_params.min_free_tokens = u64::MAX;
        assert_eq!(
            prune(&mut messages, &normal_params, &HeuristicEstimator).pruned_parts,
            0
        );

        let before_len = messages.len();
        let stats = emergency_reduce(&mut messages, 1, &HeuristicEstimator);
        assert_eq!(stats.pruned_parts, 1);
        assert_eq!(messages.len(), before_len);
        assert_eq!(messages[0], call);

        let AgentMessage::FunctionResult {
            function_call_id,
            function_id,
            details,
            ..
        } = &messages[1]
        else {
            panic!("result message kind changed");
        };
        assert_eq!(function_call_id, "c2");
        assert_eq!(function_id, "protected::lookup");
        assert_eq!(details["context_reference"]["function_call_id"], "c2");
        assert_eq!(
            details["context_reference"]["function_id"],
            "protected::lookup"
        );
    }

    #[test]
    fn emergency_reference_bounds_content_and_details() {
        let mut message: AgentMessage = serde_json::from_value(json!({
            "role": "function_result",
            "function_call_id": "call-large",
            "function_id": "shell::run",
            "content": [{ "type": "text", "text": "content".repeat(100_000) }],
            "details": { "stdout": "details".repeat(100_000) },
            "is_error": false,
            "timestamp": 9
        }))
        .unwrap();

        emergency_reduce(std::slice::from_mut(&mut message), 1, &HeuristicEstimator);

        let serialized = serde_json::to_string(&message).unwrap();
        assert!(
            serialized.len() < 2_000,
            "reference was {} bytes",
            serialized.len()
        );
        let AgentMessage::FunctionResult {
            content, details, ..
        } = &message
        else {
            panic!("result message kind changed");
        };
        let rendered = text_of(content);
        assert!(rendered.contains("original_estimated_tokens"));
        assert!(rendered.contains("original_bytes"));
        assert!(rendered.contains("session transcript"));
        assert!(
            details["context_reference"]["preview"]
                .as_str()
                .unwrap()
                .chars()
                .count()
                <= EMERGENCY_PREVIEW_CHARS + 1
        );
        assert!(details["context_reference"]["sha256"]
            .as_str()
            .is_some_and(|hash| hash.len() == 64));
    }

    #[test]
    fn emergency_reference_hash_is_deterministic_for_original_message() {
        let original: AgentMessage = serde_json::from_value(json!({
            "role": "function_result",
            "function_call_id": "hash-call",
            "function_id": "fs::read",
            "content": [{ "type": "text", "text": "deterministic payload".repeat(1_000) }],
            "details": { "path": "/tmp/example" },
            "is_error": false,
            "timestamp": 7
        }))
        .unwrap();
        let expected = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&original).unwrap())
        );
        let mut first = original.clone();
        let mut second = original;

        emergency_reduce(std::slice::from_mut(&mut first), 1, &HeuristicEstimator);
        emergency_reduce(std::slice::from_mut(&mut second), 1, &HeuristicEstimator);

        assert_eq!(first, second);
        for reduced in [&first, &second] {
            let AgentMessage::FunctionResult { details, .. } = reduced else {
                panic!("result message kind changed");
            };
            assert_eq!(details["context_reference"]["sha256"], expected);
        }
    }

    fn sizes_of(messages: &[AgentMessage]) -> Vec<u64> {
        messages
            .iter()
            .map(|m| HeuristicEstimator.message(m))
            .collect()
    }

    #[test]
    fn prune_with_sizes_keeps_the_memo_equal_to_a_recount() {
        let mut messages = history();
        let mut sizes = sizes_of(&messages);
        let stats = prune_with_sizes(&mut messages, &mut sizes, &params(), &HeuristicEstimator);
        assert_eq!(stats.pruned_parts, 1);
        assert_eq!(sizes, sizes_of(&messages));
    }

    #[test]
    fn emergency_reduce_with_sizes_keeps_the_memo_equal_to_a_recount() {
        let mut messages = vec![
            result("small", 4_000, 1),
            result("largest", 40_000, 2),
            result("middle", 20_000, 3),
        ];
        let mut sizes = sizes_of(&messages);
        let stats =
            emergency_reduce_with_sizes(&mut messages, &mut sizes, 5_000, &HeuristicEstimator);
        assert!(stats.pruned_parts >= 1);
        assert_eq!(sizes, sizes_of(&messages));
    }

    #[test]
    fn with_sizes_variants_match_the_plain_passes() {
        let mut plain = history();
        let mut memoed = history();
        let mut sizes = sizes_of(&memoed);
        let plain_stats = prune(&mut plain, &params(), &HeuristicEstimator);
        let memo_stats = prune_with_sizes(&mut memoed, &mut sizes, &params(), &HeuristicEstimator);
        assert_eq!(plain_stats, memo_stats);
        assert_eq!(plain, memoed);
    }

    #[test]
    fn emergency_reduces_largest_results_only_as_needed() {
        let mut messages = vec![
            result("small", 4_000, 1),
            result("largest", 40_000, 2),
            result("middle", 20_000, 3),
        ];

        let stats = emergency_reduce(&mut messages, 1, &HeuristicEstimator);

        assert_eq!(stats.pruned_parts, 1);
        assert_eq!(text_of(messages[0].content()).len(), 4_000);
        assert!(text_of(messages[1].content()).starts_with("[function result reduced"));
        assert_eq!(text_of(messages[2].content()).len(), 20_000);
    }

    #[test]
    fn cap_reduces_oversized_result_to_head_marker_tail() {
        // 200_000 chars ≈ 50_000 tokens; cap at 20_000.
        let mut messages = vec![result("engine::traces::list", 200_000, 1)];
        let mut sizes = sizes_of(&messages);
        let stats = cap_results_with_sizes(&mut messages, &mut sizes, 20_000, &HeuristicEstimator);
        assert_eq!(stats.capped_parts, 1);
        assert!(stats.capped_tokens > 0);
        let text = text_of(messages[0].content());
        // Rewritten text estimates under the cap (90% target + marker).
        assert!(HeuristicEstimator.text(&text) <= 20_000);
        assert!(text.contains(
            "[…result capped: was ~50000 tokens; middle omitted; re-call engine::traces::list for the full data]"
        ));
        // Head and tail of the original both survive.
        assert!(text.starts_with('x'));
        assert!(text.ends_with('x'));
        // Size memo matches a from-scratch recount.
        assert_eq!(sizes, sizes_of(&messages));
    }

    #[test]
    fn cap_skips_results_at_or_under_the_threshold() {
        let mut messages = vec![result("shell::run", 8_000, 1)]; // ~2000 tokens
        let mut sizes = sizes_of(&messages);
        let stats = cap_results_with_sizes(&mut messages, &mut sizes, 20_000, &HeuristicEstimator);
        assert_eq!(stats.capped_parts, 0);
        assert_eq!(stats.capped_tokens, 0);
        assert_eq!(text_of(messages[0].content()).len(), 8_000);
    }

    #[test]
    fn cap_is_idempotent_and_deterministic() {
        let mut once = vec![result("f::g", 200_000, 1)];
        let mut sizes_once = sizes_of(&once);
        cap_results_with_sizes(&mut once, &mut sizes_once, 20_000, &HeuristicEstimator);
        let after_first = text_of(once[0].content());

        // Second pass: under threshold now, untouched.
        let stats = cap_results_with_sizes(&mut once, &mut sizes_once, 20_000, &HeuristicEstimator);
        assert_eq!(stats.capped_parts, 0);
        assert_eq!(text_of(once[0].content()), after_first);

        // Same input capped independently yields byte-identical output.
        let mut twice = vec![result("f::g", 200_000, 1)];
        let mut sizes_twice = sizes_of(&twice);
        cap_results_with_sizes(&mut twice, &mut sizes_twice, 20_000, &HeuristicEstimator);
        assert_eq!(text_of(twice[0].content()), after_first);
    }

    #[test]
    fn cap_preserves_pairing_and_message_count() {
        let mut messages = vec![user("go", 1), result("engine::traces::list", 200_000, 2)];
        let mut sizes = sizes_of(&messages);
        let before = messages.len();
        cap_results_with_sizes(&mut messages, &mut sizes, 20_000, &HeuristicEstimator);
        assert_eq!(messages.len(), before);
        let AgentMessage::FunctionResult {
            function_call_id,
            function_id,
            ..
        } = &messages[1]
        else {
            panic!("message kind changed");
        };
        assert_eq!(function_call_id, "c2"); // result() builds call id "c{ts}"
        assert_eq!(function_id, "engine::traces::list");
    }

    #[test]
    fn cap_applies_to_error_results_and_ignores_no_protection_list() {
        // Cap has no protected-function or is_error exemption by design.
        let mut message: AgentMessage = serde_json::from_value(json!({
            "role": "function_result",
            "function_call_id": "c9",
            "function_id": "protected::lookup",
            "content": [{ "type": "text", "text": "e".repeat(200_000) }],
            "is_error": true,
            "timestamp": 9
        }))
        .unwrap();
        let mut sizes = vec![HeuristicEstimator.message(&message)];
        let stats = cap_results_with_sizes(
            std::slice::from_mut(&mut message),
            &mut sizes,
            20_000,
            &HeuristicEstimator,
        );
        assert_eq!(stats.capped_parts, 1);
    }

    #[test]
    fn cap_bounds_oversized_details_but_keeps_denied_envelopes() {
        let mut oversized: AgentMessage = serde_json::from_value(json!({
            "role": "function_result",
            "function_call_id": "c1",
            "function_id": "coder::read-file",
            "content": [{ "type": "text", "text": "y".repeat(200_000) }],
            "details": { "blob": "z".repeat(10_000) },
            "is_error": false,
            "timestamp": 1
        }))
        .unwrap();
        let mut sizes = vec![HeuristicEstimator.message(&oversized)];
        cap_results_with_sizes(
            std::slice::from_mut(&mut oversized),
            &mut sizes,
            20_000,
            &HeuristicEstimator,
        );
        let AgentMessage::FunctionResult { details, .. } = &oversized else {
            panic!("kind changed");
        };
        assert!(details["context_capped"]["original_estimated_tokens"].is_u64());

        // A denied envelope's details survive even on an oversized result.
        let mut denied: AgentMessage = serde_json::from_value(json!({
            "role": "function_result",
            "function_call_id": "c2",
            "function_id": "state::get",
            "content": [{ "type": "text", "text": "y".repeat(200_000) }],
            "details": { "status": "denied", "blob": "z".repeat(10_000) },
            "is_error": true,
            "timestamp": 2
        }))
        .unwrap();
        let mut sizes = vec![HeuristicEstimator.message(&denied)];
        cap_results_with_sizes(
            std::slice::from_mut(&mut denied),
            &mut sizes,
            20_000,
            &HeuristicEstimator,
        );
        let AgentMessage::FunctionResult { details, .. } = &denied else {
            panic!("kind changed");
        };
        assert_eq!(details["status"], "denied");
        assert_eq!(details["blob"].as_str().unwrap().len(), 10_000);
    }

    #[test]
    fn cap_splits_head_sixty_tail_forty_and_respects_char_boundaries() {
        // Multibyte text: every char is 3 bytes; slicing must not panic.
        let mut messages = vec![{
            let text: String = "€".repeat(120_000); // 360_000 bytes ≈ 90_000 tokens
            serde_json::from_value(json!({
                "role": "function_result",
                "function_call_id": "c1",
                "function_id": "f::g",
                "content": [{ "type": "text", "text": text }],
                "timestamp": 1
            }))
            .unwrap()
        }];
        let mut sizes = sizes_of(&messages);
        let stats = cap_results_with_sizes(&mut messages, &mut sizes, 20_000, &HeuristicEstimator);
        assert_eq!(stats.capped_parts, 1);
        let text = text_of(messages[0].content());
        assert!(HeuristicEstimator.text(&text) <= 20_000);
        let marker_start = text.find("\n[…result capped").unwrap();
        let head = &text[..marker_start];
        let marker_end = text.find("for the full data]\n").unwrap() + "for the full data]\n".len();
        let tail = &text[marker_end..];
        // 60/40 split of the kept budget, within rounding slack.
        let ratio = head.len() as f64 / (head.len() + tail.len()) as f64;
        assert!((0.55..=0.65).contains(&ratio), "head ratio was {ratio}");
    }
}
