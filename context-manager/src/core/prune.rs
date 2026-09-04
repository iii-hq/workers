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
//! - the configured number of most recent user turns are never touched;
//! - outputs inside the newest `protect_recent_tokens` window are kept;
//! - `protected_functions` are never pruned;
//! - outputs outside that window are eligible when they exceed
//!   `max_output_chars` or have reached `decay_user_turns` age;
//! - when everything prunable frees under `min_free_tokens`, nothing is
//!   touched at all. `context::assemble` may subsequently use the
//!   unconditional emergency pass to enforce its hard budget.

use crate::core::estimate::{Estimator, IMAGE_TOKEN_BUDGET};
use crate::types::{AgentMessage, ContentBlock, Role};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Maximum number of Unicode scalar values copied from an oversized
/// result into its emergency reference.
const EMERGENCY_PREVIEW_CHARS: usize = 160;

const EMERGENCY_RETRIEVAL_HINT: &str =
    "The original result remains in the session transcript; retrieve it by function_call_id.";

#[derive(Debug, Clone)]
pub struct PruneParams {
    /// Newest function-output tokens kept verbatim.
    pub protect_recent_tokens: u64,
    /// Prune outputs after this many subsequent user turns; `0` disables decay.
    pub decay_user_turns: usize,
    /// Most recent user turns exempt from pruning; `0` disables this exemption.
    pub protected_user_turns: usize,
    /// Skip the whole pass when it would free less than this.
    pub min_free_tokens: u64,
    /// Larger outputs are immediately eligible outside protection.
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

/// The placeholder written over a pruned output. Names the source
/// function and the recovery path (re-call it) — the transcript keeps
/// the full result, but the model-facing view does not.
pub fn placeholder(function_id: &str, tokens: u64) -> String {
    format!("[output of {function_id} pruned: was ~{tokens} tokens; re-call it if still needed]")
}

fn is_prune_placeholder(blocks: &[ContentBlock], function_id: &str) -> bool {
    let [ContentBlock::Text { text }] = blocks else {
        return false;
    };
    let prefix = format!("[output of {function_id} pruned: was ~");
    text.strip_prefix(&prefix)
        .and_then(|rest| rest.strip_suffix(" tokens; re-call it if still needed]"))
        .is_some_and(|tokens| {
            !tokens.is_empty() && tokens.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn text_of(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn image_tokens(blocks: &[ContentBlock]) -> u64 {
    blocks.iter().fold(0, |tokens, block| {
        let nested = match block {
            ContentBlock::Image { .. } => IMAGE_TOKEN_BUDGET,
            ContentBlock::FunctionResult { content, .. } => image_tokens(content),
            _ => 0,
        };
        tokens.saturating_add(nested)
    })
}

fn result_tokens(blocks: &[ContentBlock], estimator: &dyn Estimator) -> u64 {
    estimator
        .text(&text_of(blocks))
        .saturating_add(image_tokens(blocks))
}

#[derive(Debug, Clone, Copy)]
enum ResultLocation {
    Message(usize),
    Inline { message: usize, block: usize },
}

impl ResultLocation {
    fn message(self) -> usize {
        match self {
            Self::Message(message) | Self::Inline { message, .. } => message,
        }
    }
}

fn set_result_content(
    messages: &mut [AgentMessage],
    location: ResultLocation,
    content: Vec<ContentBlock>,
) {
    match location {
        ResultLocation::Message(message) => messages[message].set_content(content),
        ResultLocation::Inline { message, block } => {
            let ContentBlock::FunctionResult {
                content: current, ..
            } = &mut messages[message].content_mut()[block]
            else {
                unreachable!("result location changed during reduction");
            };
            *current = content;
        }
    }
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
    let function_ids = inline_function_ids(messages);
    let mut scanned: u64 = 0;
    let mut window_tokens: u64 = 0;
    let mut user_turns = 0usize;
    let mut queue: Vec<(ResultLocation, u64, String)> = Vec::new();

    for idx in (0..messages.len()).rev() {
        let message = &messages[idx];
        if message.role() == Role::User && !message.has_function_result_block() {
            user_turns += 1;
            continue;
        }
        if user_turns < params.protected_user_turns {
            continue;
        }
        let mut consider =
            |location: ResultLocation, function_id: &str, content: &[ContentBlock]| {
                if params
                    .protected_functions
                    .iter()
                    .any(|protected| protected == function_id)
                {
                    return;
                }
                let text = text_of(content);
                let tokens = result_tokens(content, estimator);
                scanned += 1;
                window_tokens = window_tokens.saturating_add(tokens);
                let verbose = text.len() > params.max_output_chars;
                let decay_enabled = params.decay_user_turns > 0;
                let aged = decay_enabled && user_turns >= params.decay_user_turns;
                let already_pruned = decay_enabled && is_prune_placeholder(content, function_id);
                let age_only_saves_tokens =
                    !aged || verbose || tokens > estimator.text(&placeholder(function_id, tokens));
                if window_tokens > params.protect_recent_tokens
                    && !already_pruned
                    && (verbose || aged)
                    && age_only_saves_tokens
                {
                    queue.push((location, tokens, function_id.to_owned()));
                }
            };

        match message {
            AgentMessage::FunctionResult {
                function_id,
                content,
                ..
            } => consider(ResultLocation::Message(idx), function_id, content),
            _ => {
                for (block, content_block) in message.content().iter().enumerate().rev() {
                    let ContentBlock::FunctionResult { content, .. } = content_block else {
                        continue;
                    };
                    let location = ResultLocation::Inline {
                        message: idx,
                        block,
                    };
                    if let Some(function_id) = function_ids.get(&(idx, block)) {
                        consider(location, function_id, content);
                    }
                }
            }
        }
    }

    // Net tokens freed: each pruned output is replaced by a placeholder
    // that itself costs a few tokens, so the real saving is the original
    // size minus that placeholder. Gauge `min_free_tokens` on the net.
    let pruned_tokens: u64 = queue
        .iter()
        .map(|(_, tokens, function_id)| {
            tokens.saturating_sub(estimator.text(&placeholder(function_id, *tokens)))
        })
        .sum();
    if pruned_tokens < params.min_free_tokens {
        return PruneStats {
            pruned_tokens: 0,
            pruned_parts: 0,
            scanned_parts: scanned,
        };
    }

    for (location, tokens, function_id) in &queue {
        set_result_content(
            messages,
            *location,
            vec![ContentBlock::Text {
                text: placeholder(function_id, *tokens),
            }],
        );
        if let Some(sizes) = sizes.as_deref_mut() {
            // Re-estimate the rewritten message — not freed-token
            // arithmetic — so the memo matches a from-scratch recount.
            let message = location.message();
            sizes[message] = estimator.message(&messages[message]);
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
/// replacing rendered content (and message-level opaque details) with a
/// bounded, deterministic reference to the original transcript entry.
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
    let function_ids = inline_function_ids(messages);
    let mut candidates: Vec<(ResultLocation, u64, usize)> = Vec::new();
    let mut order = 0;
    for (message_idx, message) in messages.iter().enumerate() {
        match message {
            AgentMessage::FunctionResult { .. } => {
                candidates.push((
                    ResultLocation::Message(message_idx),
                    estimator.message(message),
                    order,
                ));
                order += 1;
            }
            _ => {
                for (block_idx, block) in message.content().iter().enumerate() {
                    if let ContentBlock::FunctionResult { content, .. } = block {
                        candidates.push((
                            ResultLocation::Inline {
                                message: message_idx,
                                block: block_idx,
                            },
                            result_tokens(content, estimator),
                            order,
                        ));
                        order += 1;
                    }
                }
            }
        }
    }

    // Stable tie-break by transcript order keeps the reduction fully
    // deterministic when multiple results have the same estimate.
    candidates.sort_by(
        |(_, left_tokens, left_order), (_, right_tokens, right_order)| {
            right_tokens
                .cmp(left_tokens)
                .then_with(|| left_order.cmp(right_order))
        },
    );

    let mut stats = PruneStats {
        scanned_parts: candidates.len() as u64,
        ..PruneStats::default()
    };

    for (location, original_tokens, _) in candidates {
        if stats.pruned_tokens >= required_tokens {
            break;
        }

        match location {
            ResultLocation::Message(message) => {
                let replacement = emergency_reference(&messages[message], original_tokens);
                let replacement_tokens = estimator.message(&replacement);
                let freed = original_tokens.saturating_sub(replacement_tokens);
                if freed == 0 {
                    continue;
                }

                messages[message] = replacement;
                if let Some(sizes) = sizes.as_deref_mut() {
                    sizes[message] = replacement_tokens;
                }
                stats.pruned_tokens = stats.pruned_tokens.saturating_add(freed);
                stats.pruned_parts += 1;
            }
            ResultLocation::Inline { message, block } => {
                let before = sizes
                    .as_deref()
                    .map(|sizes| sizes[message])
                    .unwrap_or_else(|| estimator.message(&messages[message]));
                let replacement = emergency_inline_reference(
                    &messages[message].content()[block],
                    function_ids.get(&(message, block)).map(String::as_str),
                    original_tokens,
                );
                let original =
                    std::mem::replace(&mut messages[message].content_mut()[block], replacement);
                let after = estimator.message(&messages[message]);
                let freed = before.saturating_sub(after);
                if freed == 0 {
                    messages[message].content_mut()[block] = original;
                    continue;
                }

                if let Some(sizes) = sizes.as_deref_mut() {
                    sizes[message] = after;
                }
                stats.pruned_tokens = stats.pruned_tokens.saturating_add(freed);
                stats.pruned_parts += 1;
            }
        }
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

fn emergency_inline_reference(
    block: &ContentBlock,
    function_id: Option<&str>,
    original_tokens: u64,
) -> ContentBlock {
    let ContentBlock::FunctionResult {
        function_call_id,
        content,
        is_error,
    } = block
    else {
        unreachable!("emergency references are only built for function results");
    };
    let serialized = serde_json::to_vec(block).expect("serializing a ContentBlock cannot fail");
    let sha256 = format!("{:x}", Sha256::digest(&serialized));
    let mut reference = json!({
        "kind": "function_result_reference",
        "function_call_id": function_call_id,
        "original_bytes": serialized.len(),
        "original_estimated_tokens": original_tokens,
        "sha256": sha256,
        "preview": emergency_preview(content, &Value::Null),
        "retrieval_hint": EMERGENCY_RETRIEVAL_HINT,
    });
    if let Some(function_id) = function_id {
        reference["function_id"] = json!(function_id);
    }
    let rendered = format!(
        "[function result reduced for context budget] {}",
        serde_json::to_string(&reference)
            .expect("serializing a function-result reference cannot fail")
    );

    ContentBlock::FunctionResult {
        function_call_id: function_call_id.clone(),
        content: vec![ContentBlock::Text { text: rendered }],
        is_error: *is_error,
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

fn inline_function_ids(messages: &[AgentMessage]) -> HashMap<(usize, usize), String> {
    let mut calls: HashMap<&str, &str> = HashMap::new();
    let mut results = HashMap::new();
    for (message, row) in messages.iter().enumerate() {
        for (block, content) in row.content().iter().enumerate() {
            match content {
                ContentBlock::FunctionCall {
                    id, function_id, ..
                } => {
                    calls.insert(id, function_id);
                }
                ContentBlock::FunctionResult {
                    function_call_id, ..
                } => {
                    if let Some(function_id) = calls.get(function_call_id.as_str()) {
                        results.insert((message, block), (*function_id).to_owned());
                    }
                }
                _ => {}
            }
        }
    }
    results
}

fn capped_result_content(
    content: &[ContentBlock],
    function_id: Option<&str>,
    max_result_tokens: u64,
    estimator: &dyn Estimator,
) -> Option<(Vec<ContentBlock>, u64)> {
    let text = text_of(content);
    let tokens = result_tokens(content, estimator);
    if tokens <= max_result_tokens {
        return None;
    }
    let function_id = function_id.unwrap_or("the function");

    let full_marker = format!(
        "\n[…result capped: was ~{tokens} tokens; middle omitted; re-call {function_id} with narrower arguments if the omitted middle is needed]\n"
    );
    let marker = [full_marker.as_str(), "[cap]", "…", ""]
        .into_iter()
        .find(|candidate| estimator.text(candidate) <= max_result_tokens)
        .unwrap_or_default();

    let target_chars = (text.len() as u64) * max_result_tokens * 9 / (tokens * 10);
    let keep_chars = (target_chars as usize).saturating_sub(marker.len());
    let head_budget = keep_chars * 6 / 10;
    let tail_budget = keep_chars - head_budget;
    let head_end = floor_char_boundary(&text, head_budget);
    let tail_start =
        ceil_char_boundary(&text, text.len().saturating_sub(tail_budget)).max(head_end);
    let capped = format!("{}{}{}", &text[..head_end], marker, &text[tail_start..]);

    Some((vec![ContentBlock::Text { text: capped }], tokens))
}

/// Unconditionally rewrite any single function result whose content estimates
/// over `max_result_tokens` to a bounded head + marker + tail view
/// (context-manager.md § context::assemble). Applies to every result — any
/// age, protected or not, error or not: it is a generous ceiling, like the
/// emergency pass, not a policy prune. The rewrite reserves the marker's
/// own bytes out of a 90%-of-cap budget *before* splitting head/tail, so
/// head + marker + tail together target 90% of the cap — not 90% plus the
/// marker on top — and the output re-estimates under the threshold even
/// for small caps or long `function_id`s: the pass is idempotent without a
/// fixpoint loop, and deterministic (no call-varying content) so identical
/// histories assemble byte-identically across calls.
pub fn cap_results_with_sizes(
    messages: &mut [AgentMessage],
    sizes: &mut [u64],
    max_result_tokens: u64,
    estimator: &dyn Estimator,
) -> CapStats {
    debug_assert_eq!(messages.len(), sizes.len());
    let function_ids = inline_function_ids(messages);
    let mut stats = CapStats::default();
    for idx in 0..messages.len() {
        let before = sizes[idx];
        let mut capped_parts = 0;
        match &mut messages[idx] {
            AgentMessage::FunctionResult {
                function_id,
                content,
                details,
                ..
            } => {
                if let Some((capped, tokens)) =
                    capped_result_content(content, Some(function_id), max_result_tokens, estimator)
                {
                    *content = capped;
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
                    capped_parts = 1;
                }
            }
            message => {
                for (block_idx, block) in message.content_mut().iter_mut().enumerate() {
                    let ContentBlock::FunctionResult { content, .. } = block else {
                        continue;
                    };
                    if let Some((capped, _)) = capped_result_content(
                        content,
                        function_ids.get(&(idx, block_idx)).map(String::as_str),
                        max_result_tokens,
                        estimator,
                    ) {
                        *content = capped;
                        capped_parts += 1;
                    }
                }
            }
        }

        if capped_parts > 0 {
            sizes[idx] = estimator.message(&messages[idx]);
            stats.capped_tokens = stats
                .capped_tokens
                .saturating_add(before.saturating_sub(sizes[idx]));
            stats.capped_parts += capped_parts;
        }
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
            decay_user_turns: 0,
            protected_user_turns: 2,
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
    fn decay_disabled_preserves_legacy_literal_output_and_stats() {
        let mut messages = history();
        let stats = prune(&mut messages, &params(), &HeuristicEstimator);
        assert_eq!(
            stats,
            PruneStats {
                pruned_tokens: 1_982,
                pruned_parts: 1,
                scanned_parts: 1,
            }
        );
        assert_eq!(
            messages[1].content(),
            &[ContentBlock::Text {
                text: "[output of shell::run pruned: was ~2000 tokens; re-call it if still needed]"
                    .into()
            }]
        );
        assert_eq!(text_of(messages[3].content()).len(), 8_000);
    }

    #[test]
    fn decay_prunes_at_the_configured_age_not_one_turn_before() {
        let mut messages = vec![
            user("first", 1),
            result("shell::run", 400, 2),
            user("second", 3),
        ];
        let mut p = params();
        p.protect_recent_tokens = 0;
        p.max_output_chars = 10_000;
        p.decay_user_turns = 2;
        p.protected_user_turns = 0;

        let before_boundary = prune(&mut messages, &p, &HeuristicEstimator);
        assert_eq!(before_boundary.pruned_parts, 0);
        assert_eq!(text_of(messages[1].content()).len(), 400);

        messages.push(user("third", 4));
        let at_boundary = prune(&mut messages, &p, &HeuristicEstimator);
        assert_eq!(at_boundary.pruned_parts, 1);
        assert_eq!(
            text_of(messages[1].content()),
            "[output of shell::run pruned: was ~100 tokens; re-call it if still needed]"
        );
    }

    #[test]
    fn decay_applies_below_and_at_the_verbose_threshold() {
        let mut messages = vec![
            user("first", 1),
            result("below", 396, 2),
            result("at", 400, 3),
            user("second", 4),
            user("third", 5),
        ];
        let mut p = params();
        p.protect_recent_tokens = 0;
        p.max_output_chars = 400;
        p.decay_user_turns = 2;
        p.protected_user_turns = 0;

        let stats = prune(&mut messages, &p, &HeuristicEstimator);

        assert_eq!(stats.pruned_parts, 2);
        assert!(text_of(messages[1].content()).starts_with("[output of below pruned"));
        assert!(text_of(messages[2].content()).starts_with("[output of at pruned"));
    }

    #[test]
    fn decay_still_honors_the_newest_token_window() {
        let mut messages = vec![
            user("first", 1),
            result("older", 400, 2),
            result("newer", 400, 3),
            user("second", 4),
            user("third", 5),
        ];
        let mut p = params();
        p.protect_recent_tokens = 100;
        p.max_output_chars = 10_000;
        p.decay_user_turns = 2;
        p.protected_user_turns = 0;

        let stats = prune(&mut messages, &p, &HeuristicEstimator);

        assert_eq!(stats.pruned_parts, 1);
        assert!(text_of(messages[1].content()).starts_with("[output of older pruned"));
        assert_eq!(text_of(messages[2].content()).len(), 400);
    }

    #[test]
    fn zero_protected_turns_removes_only_the_turn_exemption() {
        let original = vec![user("first", 1), result("shell::run", 8_000, 2)];
        let mut protected = original.clone();
        let mut p = params();
        p.protect_recent_tokens = 0;
        let protected_stats = prune(&mut protected, &p, &HeuristicEstimator);
        assert_eq!(protected_stats.scanned_parts, 0);
        assert_eq!(protected, original);

        p.protected_user_turns = 0;
        let mut unprotected = original;
        let unprotected_stats = prune(&mut unprotected, &p, &HeuristicEstimator);
        assert_eq!(unprotected_stats.pruned_parts, 1);
        assert!(text_of(unprotected[1].content()).starts_with("[output of shell::run pruned"));
    }

    #[test]
    fn decay_honors_protected_functions_and_minimum_free_tokens() {
        let original = vec![
            user("first", 1),
            result("keep::me", 400, 2),
            user("second", 3),
        ];
        let mut p = params();
        p.protect_recent_tokens = 0;
        p.max_output_chars = 10_000;
        p.decay_user_turns = 1;
        p.protected_user_turns = 0;
        p.protected_functions = vec!["keep::me".into()];

        let mut protected = original.clone();
        let protected_stats = prune(&mut protected, &p, &HeuristicEstimator);
        assert_eq!(protected_stats.scanned_parts, 0);
        assert_eq!(protected, original);

        p.protected_functions.clear();
        p.min_free_tokens = 1_000;
        let mut below_minimum = original.clone();
        let minimum_stats = prune(&mut below_minimum, &p, &HeuristicEstimator);
        assert_eq!(minimum_stats.scanned_parts, 1);
        assert_eq!(minimum_stats.pruned_parts, 0);
        assert_eq!(minimum_stats.pruned_tokens, 0);
        assert_eq!(below_minimum, original);
    }

    #[test]
    fn decay_does_not_expand_tiny_or_equal_cost_outputs() {
        let mut tiny = vec![user("first", 1), result("tiny", 1, 2), user("second", 3)];
        let mut p = params();
        p.protect_recent_tokens = 0;
        p.max_output_chars = 10_000;
        p.decay_user_turns = 1;
        p.protected_user_turns = 0;
        p.min_free_tokens = 0;

        let tiny_stats = prune(&mut tiny, &p, &HeuristicEstimator);
        assert_eq!(tiny_stats.pruned_parts, 0);
        assert_eq!(text_of(tiny[1].content()), "x");

        struct EqualCostEstimator;
        impl Estimator for EqualCostEstimator {
            fn kind(&self) -> crate::core::estimate::EstimatorKind {
                crate::core::estimate::EstimatorKind::Heuristic
            }

            fn message(&self, _message: &AgentMessage) -> u64 {
                10
            }

            fn text(&self, _text: &str) -> u64 {
                10
            }

            fn function(&self, _function: &crate::types::AgentFunction) -> u64 {
                0
            }
        }

        let mut equal = vec![user("first", 1), result("equal", 40, 2), user("second", 3)];
        let equal_stats = prune(&mut equal, &p, &EqualCostEstimator);
        assert_eq!(equal_stats.pruned_parts, 0);
        assert_eq!(text_of(equal[1].content()).len(), 40);
    }

    #[test]
    fn decay_skips_only_exact_canonical_prune_placeholders() {
        let canonical = placeholder("shell::run", 1_000);
        let marker_like = format!("{canonical} ordinary trailing output {}", "x".repeat(100));
        let mut messages: Vec<AgentMessage> = serde_json::from_value(json!([
            { "role": "user", "content": [{ "type": "text", "text": "first" }], "timestamp": 1 },
            {
                "role": "function_result", "function_call_id": "c1", "function_id": "shell::run",
                "content": [{ "type": "text", "text": canonical }], "timestamp": 2
            },
            {
                "role": "function_result", "function_call_id": "c2", "function_id": "shell::run",
                "content": [{ "type": "text", "text": marker_like }], "timestamp": 3
            },
            { "role": "user", "content": [{ "type": "text", "text": "second" }], "timestamp": 4 }
        ]))
        .unwrap();
        let canonical_before = messages[1].clone();
        let mut p = params();
        p.protect_recent_tokens = 0;
        p.max_output_chars = 10_000;
        p.decay_user_turns = 1;
        p.protected_user_turns = 0;

        let stats = prune(&mut messages, &p, &HeuristicEstimator);

        assert_eq!(stats.pruned_parts, 1);
        assert_eq!(messages[1], canonical_before);
        assert!(text_of(messages[2].content()).starts_with("[output of shell::run pruned"));
    }

    #[test]
    fn inline_result_wrappers_do_not_increment_age_and_keep_siblings() {
        let original: Vec<AgentMessage> = serde_json::from_value(json!([
            {
                "role": "assistant",
                "content": [{
                    "type": "function_call", "id": "c1", "function_id": "shell::run",
                    "arguments": { "command": "status" }
                }],
                "stop_reason": "function_call", "model": "m", "provider": "p", "timestamp": 1
            },
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": "before" },
                    {
                        "type": "function_result", "function_call_id": "c1", "is_error": true,
                        "content": [{ "type": "text", "text": "x".repeat(400) }]
                    },
                    { "type": "text", "text": "after" }
                ],
                "timestamp": 2
            },
            { "role": "user", "content": [{ "type": "text", "text": "next" }], "timestamp": 3 },
            { "role": "user", "content": [{ "type": "text", "text": "again" }], "timestamp": 4 }
        ]))
        .unwrap();
        let mut p = params();
        p.protect_recent_tokens = 0;
        p.max_output_chars = 10_000;
        p.protected_user_turns = 0;
        p.decay_user_turns = 3;

        let mut too_young = original.clone();
        assert_eq!(
            prune(&mut too_young, &p, &HeuristicEstimator).pruned_parts,
            0
        );
        assert_eq!(too_young, original);

        p.decay_user_turns = 2;
        let mut at_age = original;
        let stats = prune(&mut at_age, &p, &HeuristicEstimator);
        assert_eq!(stats.pruned_parts, 1);
        assert_eq!(text_of(&at_age[1].content()[0..1]), "before");
        assert_eq!(text_of(&at_age[1].content()[2..3]), "after");
        let ContentBlock::FunctionResult {
            function_call_id,
            content,
            is_error,
        } = &at_age[1].content()[1]
        else {
            panic!("inline result block changed kind");
        };
        assert_eq!(function_call_id, "c1");
        assert_eq!(*is_error, Some(true));
        assert!(text_of(content).starts_with("[output of shell::run pruned"));
    }

    #[test]
    fn decay_preserves_message_metadata_and_keeps_size_memo_exact() {
        let mut messages: Vec<AgentMessage> = serde_json::from_value(json!([
            { "role": "user", "content": [{ "type": "text", "text": "first" }], "timestamp": 1 },
            {
                "role": "function_result", "function_call_id": "call-7", "function_id": "fs::read",
                "content": [{ "type": "text", "text": "x".repeat(400) }],
                "details": { "path": "/tmp/example", "line": 7 }, "is_error": true, "timestamp": 2
            },
            { "role": "user", "content": [{ "type": "text", "text": "second" }], "timestamp": 3 }
        ]))
        .unwrap();
        let mut sizes = sizes_of(&messages);
        let mut p = params();
        p.protect_recent_tokens = 0;
        p.max_output_chars = 10_000;
        p.decay_user_turns = 1;
        p.protected_user_turns = 0;

        let stats = prune_with_sizes(&mut messages, &mut sizes, &p, &HeuristicEstimator);

        assert_eq!(stats.pruned_parts, 1);
        let AgentMessage::FunctionResult {
            function_call_id,
            function_id,
            details,
            is_error,
            timestamp,
            ..
        } = &messages[1]
        else {
            panic!("message result changed kind");
        };
        assert_eq!(function_call_id, "call-7");
        assert_eq!(function_id, "fs::read");
        assert_eq!(details, &json!({ "path": "/tmp/example", "line": 7 }));
        assert!(*is_error);
        assert_eq!(*timestamp, 2);
        assert_eq!(sizes, sizes_of(&messages));
    }

    #[test]
    fn synthetic_long_session_saves_medium_results_with_decay_four() {
        let mut history = Vec::new();
        let mut timestamp = 0;
        for turn in 0..10 {
            timestamp += 1;
            history.push(user(&format!("turn {turn}"), timestamp));
            timestamp += 1;
            history.push(result("medium::lookup", 2_400, timestamp));
            if turn == 0 {
                timestamp += 1;
                history.push(result("keep::me", 2_400, timestamp));
            }
        }

        let mut off = history.clone();
        let mut off_sizes = sizes_of(&off);
        let mut off_params = params();
        off_params.protect_recent_tokens = 600;
        off_params.max_output_chars = 10_000;
        off_params.protected_user_turns = 2;
        off_params.decay_user_turns = 0;
        off_params.protected_functions = vec!["keep::me".into()];
        let off_stats =
            prune_with_sizes(&mut off, &mut off_sizes, &off_params, &HeuristicEstimator);

        let mut decayed = history.clone();
        let mut decayed_sizes = sizes_of(&decayed);
        let mut decay_params = off_params;
        decay_params.decay_user_turns = 4;
        let decay_stats = prune_with_sizes(
            &mut decayed,
            &mut decayed_sizes,
            &decay_params,
            &HeuristicEstimator,
        );

        assert_eq!(off_stats.pruned_parts, 0);
        assert!(decay_stats.pruned_parts > 0);
        assert!(decayed_sizes.iter().sum::<u64>() < off_sizes.iter().sum::<u64>());
        assert_eq!(decayed[2], history[2]);
        assert_eq!(decayed.last(), history.last());
        assert_eq!(decayed_sizes, sizes_of(&decayed));
    }

    #[test]
    fn prune_resolves_reused_inline_call_ids_positionally() {
        let mut messages: Vec<AgentMessage> = serde_json::from_value(json!([
            { "role": "user", "content": [{ "type": "text", "text": "task" }], "timestamp": 1 },
            {
                "role": "assistant",
                "content": [{
                    "type": "function_call", "id": "c1",
                    "function_id": "shell::run", "arguments": {}
                }],
                "stop_reason": "function_call", "model": "m", "provider": "p",
                "timestamp": 2
            },
            {
                "role": "user",
                "content": [{
                    "type": "function_result", "function_call_id": "c1",
                    "content": [{ "type": "text", "text": "x".repeat(8_000) }]
                }],
                "timestamp": 3
            },
            {
                "role": "assistant", "content": [{ "type": "text", "text": "used" }],
                "stop_reason": "end", "model": "m", "provider": "p", "timestamp": 4
            },
            { "role": "user", "content": [{ "type": "text", "text": "next" }], "timestamp": 5 },
            { "role": "user", "content": [{ "type": "text", "text": "done" }], "timestamp": 6 },
            {
                "role": "assistant",
                "content": [{
                    "type": "function_call", "id": "c1",
                    "function_id": "later::call", "arguments": {}
                }],
                "stop_reason": "function_call", "model": "m", "provider": "p",
                "timestamp": 7
            }
        ]))
        .unwrap();
        let mut protected_messages = messages.clone();
        let mut protected_sizes = sizes_of(&protected_messages);
        let mut protected_params = params();
        protected_params.protected_functions = vec!["shell::run".into()];

        let protected_stats = prune_with_sizes(
            &mut protected_messages,
            &mut protected_sizes,
            &protected_params,
            &HeuristicEstimator,
        );

        assert_eq!(protected_stats.pruned_parts, 0);
        assert_eq!(protected_messages, messages);
        let mut sizes = sizes_of(&messages);

        let stats = prune_with_sizes(&mut messages, &mut sizes, &params(), &HeuristicEstimator);

        assert_eq!(stats.pruned_parts, 1);
        let ContentBlock::FunctionResult {
            function_call_id,
            content,
            ..
        } = &messages[2].content()[0]
        else {
            panic!("inline result block changed kind");
        };
        assert_eq!(function_call_id, "c1");
        assert_eq!(
            text_of(content),
            "[output of shell::run pruned: was ~2000 tokens; re-call it if still needed]"
        );
        assert_eq!(sizes, sizes_of(&messages));
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
        assert!(text_of(messages[1].content()).starts_with("[output of f pruned"));
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
    fn emergency_reduces_inline_results_without_replacing_the_host_message() {
        let mut messages: Vec<AgentMessage> = serde_json::from_value(json!([
            {
                "role": "assistant",
                "content": [{
                    "type": "function_call", "id": "c1",
                    "function_id": "shell::run", "arguments": {}
                }],
                "stop_reason": "function_call", "model": "m", "provider": "p",
                "timestamp": 1
            },
            {
                "role": "user",
                "content": [
                    {
                        "type": "function_result", "function_call_id": "c1",
                        "content": [{ "type": "text", "text": "x".repeat(100_000) }]
                    },
                    { "type": "text", "text": "keep this sibling" }
                ],
                "timestamp": 2
            }
        ]))
        .unwrap();
        let mut sizes = sizes_of(&messages);

        let stats = emergency_reduce_with_sizes(&mut messages, &mut sizes, 1, &HeuristicEstimator);

        assert_eq!(stats.pruned_parts, 1);
        assert_eq!(messages[1].role(), Role::User);
        assert_eq!(
            messages[1].content()[1],
            ContentBlock::Text {
                text: "keep this sibling".into()
            }
        );
        let ContentBlock::FunctionResult {
            function_call_id,
            content,
            ..
        } = &messages[1].content()[0]
        else {
            panic!("inline result block changed kind");
        };
        assert_eq!(function_call_id, "c1");
        assert!(text_of(content).starts_with("[function result reduced"));
        assert!(text_of(content).contains("\"function_id\":\"shell::run\""));
        assert_eq!(sizes, sizes_of(&messages));
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
            "[…result capped: was ~50000 tokens; middle omitted; re-call engine::traces::list with narrower arguments if the omitted middle is needed]"
        ));
        // Head and tail of the original both survive.
        assert!(text.starts_with('x'));
        assert!(text.ends_with('x'));
        // Size memo matches a from-scratch recount.
        assert_eq!(sizes, sizes_of(&messages));
    }

    #[test]
    fn cap_preserves_provider_separators_between_text_blocks() {
        let mut messages = vec![serde_json::from_value(json!({
            "role": "function_result",
            "function_call_id": "c1",
            "function_id": "shell::run",
            "content": [
                { "type": "text", "text": "header" },
                { "type": "text", "text": "x".repeat(100_000) }
            ],
            "timestamp": 1
        }))
        .unwrap()];
        let mut sizes = sizes_of(&messages);

        cap_results_with_sizes(&mut messages, &mut sizes, 20_000, &HeuristicEstimator);

        assert!(text_of(messages[0].content()).starts_with("header\nx"));
    }

    #[test]
    fn cap_counts_image_cost_in_the_result_ceiling() {
        let mut messages = vec![serde_json::from_value(json!({
            "role": "function_result",
            "function_call_id": "c1",
            "function_id": "browser::screenshots",
            "content": (0..5).map(|_| json!({
                "type": "image", "mime": "image/png", "data": "AAAA"
            })).collect::<Vec<_>>(),
            "timestamp": 1
        }))
        .unwrap()];
        let mut sizes = sizes_of(&messages);

        let stats = cap_results_with_sizes(&mut messages, &mut sizes, 20_000, &HeuristicEstimator);

        assert_eq!(stats.capped_parts, 1);
        assert!(text_of(messages[0].content()).contains("result capped"));
        assert_eq!(sizes, sizes_of(&messages));
    }

    #[test]
    fn cap_rewrites_oversized_inline_results_without_breaking_pairing() {
        let mut messages: Vec<AgentMessage> = serde_json::from_value(json!([
            {
                "role": "assistant",
                "content": [{
                    "type": "function_call", "id": "c1",
                    "function_id": "shell::run", "arguments": {}
                }],
                "stop_reason": "function_call", "model": "m", "provider": "p",
                "timestamp": 1
            },
            {
                "role": "user",
                "content": [{
                    "type": "function_result", "function_call_id": "c1",
                    "content": [{ "type": "text", "text": "x".repeat(100_000) }]
                }],
                "timestamp": 2
            }
        ]))
        .unwrap();
        let mut sizes = sizes_of(&messages);

        let stats = cap_results_with_sizes(&mut messages, &mut sizes, 20_000, &HeuristicEstimator);

        assert_eq!(stats.capped_parts, 1);
        let ContentBlock::FunctionResult {
            function_call_id,
            content,
            ..
        } = &messages[1].content()[0]
        else {
            panic!("inline result block changed kind");
        };
        assert_eq!(function_call_id, "c1");
        assert!(text_of(content).contains("re-call shell::run"));
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
    fn cap_skips_result_exactly_at_the_threshold() {
        // The skip condition is `tokens <= max_result_tokens`: 80_000
        // chars / 4 == 20_000 tokens exactly, so the boundary itself
        // must be left untouched, not just values strictly under it.
        let mut messages = vec![result("engine::traces::list", 80_000, 1)];
        let mut sizes = sizes_of(&messages);
        let stats = cap_results_with_sizes(&mut messages, &mut sizes, 20_000, &HeuristicEstimator);
        assert_eq!(stats.capped_parts, 0);
        assert_eq!(text_of(messages[0].content()).len(), 80_000);
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
        // Size memo matches a from-scratch recount.
        assert_eq!(sizes, sizes_of(std::slice::from_ref(&oversized)));

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
        // Size memo matches a from-scratch recount.
        assert_eq!(sizes, sizes_of(std::slice::from_ref(&denied)));
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
        let marker_end = text.find("if the omitted middle is needed]\n").unwrap()
            + "if the omitted middle is needed]\n".len();
        let tail = &text[marker_end..];
        // 60/40 split of the kept budget, within rounding slack.
        let ratio = head.len() as f64 / (head.len() + tail.len()) as f64;
        assert!((0.55..=0.65).contains(&ratio), "head ratio was {ratio}");
    }

    #[test]
    fn cap_holds_the_threshold_at_a_small_cap() {
        // Regression: the marker's own byte cost must come out of the kept
        // budget, or a small cap leaves the rewritten result still over
        // the threshold (the marker's fixed cost can exceed the 10% margin
        // once the cap itself is small).
        let mut messages = vec![result("f::g", 200_000, 1)];
        let mut sizes = sizes_of(&messages);
        let stats = cap_results_with_sizes(&mut messages, &mut sizes, 200, &HeuristicEstimator);
        assert_eq!(stats.capped_parts, 1);
        let text = text_of(messages[0].content());
        assert!(HeuristicEstimator.text(&text) <= 200);
    }

    #[test]
    fn cap_holds_the_threshold_below_the_full_marker_cost() {
        let mut messages = vec![result("function::with::a::long::identifier", 200_000, 1)];
        let mut sizes = sizes_of(&messages);

        cap_results_with_sizes(&mut messages, &mut sizes, 1, &HeuristicEstimator);

        let text = text_of(messages[0].content());
        assert!(!text.is_empty());
        assert!(HeuristicEstimator.text(&text) <= 1);
    }

    #[test]
    fn cap_holds_the_threshold_with_a_long_function_id_and_is_idempotent() {
        // A longer function_id makes the marker's fixed cost bigger still;
        // the budget reservation has to account for it regardless.
        let mut messages = vec![result(
            "engine::observability::traces::list-with-a-very-long-descriptive-name",
            200_000,
            1,
        )];
        let mut sizes = sizes_of(&messages);
        let stats = cap_results_with_sizes(&mut messages, &mut sizes, 300, &HeuristicEstimator);
        assert_eq!(stats.capped_parts, 1);
        let text = text_of(messages[0].content());
        assert!(HeuristicEstimator.text(&text) <= 300);

        // Idempotent in the same regime that used to break it: a second
        // pass finds the result already at or under the cap.
        let second = cap_results_with_sizes(&mut messages, &mut sizes, 300, &HeuristicEstimator);
        assert_eq!(second.capped_parts, 0);
    }

    /// Steps one cap+prune replay across 20 turns: each turn appends a
    /// user message and three mid-size `engine::functions::info` results
    /// (`mid_chars` each) to the untrimmed raw history; turn `whale_a_turn`
    /// additionally lands a 340_000-char traces-list whale and turn
    /// `whale_b_turn` a 300_000-char state::get whale (both sized from the
    /// evidence session, console-5293cd86…). Every step clones `raw` and
    /// re-derives cap+prune from the clone with the shipped defaults,
    /// exactly like `context::assemble` (whose output is never persisted)
    /// — it never mutates one working vector across steps, or the bound
    /// would look better than it is. Asserts the worst single-step total
    /// against the caller's `ceiling` (always a fixed literal at the call
    /// site) and returns that worst total.
    fn replay_totals(
        mid_chars: usize,
        whale_a_turn: usize,
        whale_b_turn: usize,
        ceiling: u64,
    ) -> u64 {
        // Read "the shipped defaults" rather than re-typing them, so a
        // config-default change actually moves this test.
        let shipped = crate::config::WorkerConfig::default();
        let defaults = PruneParams {
            protect_recent_tokens: shipped.protect_recent_tokens,
            decay_user_turns: shipped.decay_user_turns,
            protected_user_turns: shipped.protected_user_turns,
            min_free_tokens: shipped.min_free_tokens,
            max_output_chars: shipped.max_output_chars,
            protected_functions: vec![],
        };
        let mut raw: Vec<AgentMessage> = Vec::new();
        let mut ts = 0i64;
        let mut worst_total = 0u64;
        for turn in 0..20 {
            ts += 1;
            raw.push(user(&format!("request {turn}"), ts));
            for _ in 0..3 {
                ts += 1;
                raw.push(result("engine::functions::info", mid_chars, ts));
            }
            if turn == whale_a_turn {
                ts += 1;
                raw.push(result("engine::traces::list", 340_000, ts));
            }
            if turn == whale_b_turn {
                ts += 1;
                raw.push(result("state::get", 300_000, ts));
            }

            // One assemble step: re-derive from raw, never persist.
            let mut working = raw.clone();
            let mut sizes = sizes_of(&working);
            cap_results_with_sizes(
                &mut working,
                &mut sizes,
                shipped.max_result_tokens,
                &HeuristicEstimator,
            );
            prune_with_sizes(&mut working, &mut sizes, &defaults, &HeuristicEstimator);
            // Message-history subtotal only: real assemble also adds
            // system-prompt, tool-schema, and request-overhead tokens.
            let total: u64 = sizes.iter().sum();
            worst_total = worst_total.max(total);
        }
        assert!(
            worst_total <= ceiling,
            "steady-state context reached {worst_total} tokens (ceiling {ceiling})"
        );
        worst_total
    }

    /// Worst-case mid-result load, deliberately heavier than the evidence
    /// session (console-5293cd86…) — NOT a replay of it. Three
    /// 2_500-token (10_000-char) mid-size results per turn is roughly 5x
    /// that session's actual non-whale mass (~600 tokens/result; see
    /// `evidence_session_replay_stays_bounded` below for the real ratio).
    /// It reuses the evidence session's whale timing and sizes — an
    /// ~85k-token traces dump at turn 5, an ~75k-token state::get at turn
    /// 12 — as a stress load on top of that heavier baseline, to prove
    /// cap+prune hold even above any load actually observed.
    ///
    /// This fixture's own untouched raw total (no cap, no prune) reaches
    /// 313_097 tokens by turn 19; prune alone with no cap still reaches
    /// 138_594 (the traces whale sits unclipped at ~85k inside the
    /// always-exempt last-2-turns zone, where prune's window can't reach
    /// it). With both passes the worst step is turn 13 at 75_653 tokens,
    /// which decomposes exactly as:
    /// ```text
    ///   33_343  last-2-user-turns zone (unconditionally exempt): the
    ///           just-landed state::get whale capped to 18_041 (~90% of
    ///           the 20k cap) + 3 same-turn mid results + the prior
    ///           turn's 3 mid results + 2 user messages
    /// + 40_799  protect_recent_tokens window: 16 not-yet-aged-out mid
    ///           results at 2_544 tokens each (message-level estimate;
    ///           the window's own 40_000-token budget is measured on
    ///           text-only tokens, 2_500 each, so the same window holds
    ///           slightly more at the message level) + 5 free-riding
    ///           user messages the window never charges for
    /// + 1_511   residue: 1_378 tokens across the 21 placeholders
    ///           already collapsed above + 133 tokens of aged-region
    ///           user messages (past both the recent-turn and window
    ///           zones, never charged against either budget)
    /// = 75_653.
    /// ```
    ///
    /// 78_000 is a fixed ceiling with headroom over that
    /// verified 75_653 (for incidental token-count drift from unrelated
    /// wording changes elsewhere in this file), while staying far below
    /// both the >100k range that would indicate cap or prune regressing
    /// and this fixture's own 313_097 raw / 138_594 no-cap figures above.
    #[test]
    fn heavy_mid_result_load_stays_bounded() {
        replay_totals(10_000, 5, 12, 78_000);
    }

    /// Evidence-session replay (console-5293cd86…): the session's actual
    /// ratio of mid-size tool output to its two whales — three ~600-token
    /// (~2_400-char) `engine::functions::info` results per turn, against
    /// an ~85k-token traces dump at turn 5 and an ~75k-token state::get at
    /// turn 12, in a 20-turn session (see `heavy_mid_result_load_stays_
    /// bounded` above for a deliberately heavier stress variant with the
    /// same whale timing). With the shipped defaults the model-facing
    /// total stays inside the spec's ~30-50k steady-state band: the mean
    /// total across turns 5..20 (the first whale lands at turn 5, the
    /// second only at turn 12, so turns 5-11 run with one whale) measures
    /// 41_663 tokens — comfortably under the spec's <= 1/3-of-original
    /// criterion applied to this session's real ~190k peak, and nothing
    /// like the 75_653 stress figure above. The worst single step is
    /// 63_392; 66_000 is a fixed ceiling with the same small headroom
    /// rationale as the stress variant's.
    #[test]
    fn evidence_session_replay_stays_bounded() {
        replay_totals(2_400, 5, 12, 66_000);
    }
}
