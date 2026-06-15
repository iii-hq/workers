//! Token-aware verbatim-tail selection for compaction. Pure logic.
//!
//! Ported from harness `context-compaction/selection.ts`, hardened for
//! the spec's structural invariant (context-manager.md § Structural
//! invariants): a `function_call` and its `function_result` always land
//! on the same side of the head/tail boundary, so the tail may only
//! start at a *safe cut* — a user or assistant message (never a
//! `function_result` message, and never a user message carrying inline
//! `function_result` blocks).

use crate::core::estimate::Estimator;
use crate::types::{AgentMessage, Role};

/// One conversation turn: starts at a user message, ends right before
/// the next one (exclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Turn {
    pub start: usize,
    pub end: usize,
}

/// Head/tail split. `tail_start_index` is `None` when nothing could be
/// safely kept verbatim — the whole history is the head (and the spec's
/// `tail_start_index` is `null`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub head_len: usize,
    pub tail_start_index: Option<usize>,
}

/// Partition into turns; messages before the first user message belong
/// to no turn (they are always head material).
pub fn turns(messages: &[AgentMessage]) -> Vec<Turn> {
    let mut result: Vec<Turn> = Vec::new();
    for (i, m) in messages.iter().enumerate() {
        if m.role() == Role::User {
            result.push(Turn {
                start: i,
                end: messages.len(),
            });
        }
    }
    for i in 0..result.len().saturating_sub(1) {
        result[i].end = result[i + 1].start;
    }
    result
}

/// A boundary the tail may start at without orphaning a result:
/// user/assistant messages only, and the user message must not carry
/// inline `function_result` blocks (providers reject results whose
/// call landed in the summarised head).
fn is_safe_cut(message: &AgentMessage) -> bool {
    match message.role() {
        Role::Assistant => true,
        Role::User => !message.has_function_result_block(),
        Role::FunctionResult | Role::Custom => false,
    }
}

/// Find a partial tail inside `turn` that fits `budget`, scanning from
/// the oldest in-turn position forward; only safe cuts qualify.
fn split_turn(
    messages: &[AgentMessage],
    turn: Turn,
    budget: u64,
    estimator: &dyn Estimator,
) -> Option<usize> {
    if budget == 0 || turn.end.saturating_sub(turn.start) <= 1 {
        return None;
    }
    for start in (turn.start + 1)..turn.end {
        if !is_safe_cut(&messages[start]) {
            continue;
        }
        let size: u64 = messages[start..turn.end]
            .iter()
            .map(|m| estimator.message(m))
            .sum();
        if size > budget {
            continue;
        }
        return Some(start);
    }
    None
}

/// Select the verbatim tail: keep up to the last `tail_turns` whole
/// turns that fit `budget` (newest first); when a whole turn does not
/// fit, fall back to a safe partial cut inside it. Everything before
/// the kept tail is the head to summarise.
pub fn select(
    messages: &[AgentMessage],
    budget: u64,
    tail_turns: usize,
    estimator: &dyn Estimator,
) -> Selection {
    let whole_head = Selection {
        head_len: messages.len(),
        tail_start_index: None,
    };
    if tail_turns == 0 {
        return whole_head;
    }
    let all = turns(messages);
    if all.is_empty() {
        return whole_head;
    }
    let recent = &all[all.len().saturating_sub(tail_turns)..];

    let mut total: u64 = 0;
    let mut keep: Option<usize> = None;
    for turn in recent.iter().rev() {
        let size: u64 = messages[turn.start..turn.end]
            .iter()
            .map(|m| estimator.message(m))
            .sum();
        if total + size <= budget {
            total += size;
            // A turn whose user message carries inline function_result
            // blocks is not a safe boundary; accumulate it and let an
            // older turn (or a split) cover it — the tail is a suffix,
            // so the unsafe turn still lands tail-side with its call.
            if is_safe_cut(&messages[turn.start]) {
                keep = Some(turn.start);
            }
            continue;
        }
        let remaining = budget.saturating_sub(total);
        if let Some(split) = split_turn(messages, *turn, remaining, estimator) {
            keep = Some(split);
        }
        break;
    }

    match keep {
        None | Some(0) => whole_head,
        Some(start) => Selection {
            head_len: start,
            tail_start_index: Some(start),
        },
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

    fn assistant(text: &str, ts: i64) -> AgentMessage {
        serde_json::from_value(json!({
            "role": "assistant", "content": [{ "type": "text", "text": text }],
            "stop_reason": "end", "model": "m", "provider": "p", "timestamp": ts
        }))
        .unwrap()
    }

    fn assistant_call(call_id: &str, ts: i64) -> AgentMessage {
        serde_json::from_value(json!({
            "role": "assistant",
            "content": [{ "type": "function_call", "id": call_id, "function_id": "f::g",
                          "arguments": {} }],
            "stop_reason": "function_call", "model": "m", "provider": "p", "timestamp": ts
        }))
        .unwrap()
    }

    fn result(call_id: &str, chars: usize, ts: i64) -> AgentMessage {
        serde_json::from_value(json!({
            "role": "function_result", "function_call_id": call_id, "function_id": "f::g",
            "content": [{ "type": "text", "text": "x".repeat(chars) }], "timestamp": ts
        }))
        .unwrap()
    }

    #[test]
    fn turns_start_at_user_messages() {
        let messages = vec![
            user("a", 1),
            assistant("r", 2),
            user("b", 3),
            assistant("r", 4),
        ];
        let t = turns(&messages);
        assert_eq!(
            t,
            vec![Turn { start: 0, end: 2 }, Turn { start: 2, end: 4 }]
        );
    }

    #[test]
    fn keeps_whole_recent_turns_within_budget() {
        let messages = vec![
            user("old question", 1),
            assistant("old answer", 2),
            user("recent question", 3),
            assistant("recent answer", 4),
        ];
        let sel = select(&messages, 1_000, 1, &HeuristicEstimator);
        assert_eq!(sel.tail_start_index, Some(2));
        assert_eq!(sel.head_len, 2);
    }

    #[test]
    fn zero_tail_turns_summarises_everything() {
        let messages = vec![user("a", 1), assistant("b", 2)];
        let sel = select(&messages, 1_000, 0, &HeuristicEstimator);
        assert_eq!(sel.tail_start_index, None);
        assert_eq!(sel.head_len, 2);
    }

    #[test]
    fn tail_covering_everything_means_no_compaction_head() {
        // Both turns fit: keep.start == 0 → whole history is "head",
        // i.e. the caller sees nothing to summarise... actually the
        // selection reports the whole history as head (no tail), which
        // compact turns into tokens_before over everything.
        let messages = vec![user("a", 1), assistant("b", 2)];
        let sel = select(&messages, u64::MAX, 2, &HeuristicEstimator);
        assert_eq!(sel.tail_start_index, None);
        assert_eq!(sel.head_len, 2);
    }

    #[test]
    fn split_inside_oversized_turn_never_cuts_at_a_function_result() {
        // One huge turn: user, assistant(call), result(big), assistant.
        // The only cut that fits the budget straddles the result — the
        // split must land on the closing assistant message instead.
        let messages = vec![
            user("q", 1),
            assistant_call("c1", 2),
            result("c1", 4_000, 3),
            assistant("done", 4),
        ];
        let sel = select(&messages, 100, 1, &HeuristicEstimator);
        // Tail = just the final assistant message (index 3): the cut at
        // index 2 (the function_result) is unsafe even though it fits.
        assert_eq!(sel.tail_start_index, Some(3));
    }

    #[test]
    fn user_message_with_inline_result_block_is_not_a_cut() {
        let inline_result_user: AgentMessage = serde_json::from_value(json!({
            "role": "user",
            "content": [{ "type": "function_result", "function_call_id": "c9", "content": [] }],
            "timestamp": 5
        }))
        .unwrap();
        assert!(!is_safe_cut(&inline_result_user));
        assert!(is_safe_cut(&user("plain", 1)));
        assert!(is_safe_cut(&assistant("a", 2)));
        assert!(!is_safe_cut(&result("c", 10, 3)));
    }

    #[test]
    fn unsafe_turn_start_defers_to_an_older_safe_turn() {
        // The newest turn starts at a user message carrying an inline
        // function_result whose call sits in the previous turn. Cutting
        // there would orphan the result, so the kept tail must start at
        // the older (safe) turn and carry both turns together.
        let inline_result_user: AgentMessage = serde_json::from_value(json!({
            "role": "user",
            "content": [{ "type": "function_result", "function_call_id": "c7", "content": [] }],
            "timestamp": 5
        }))
        .unwrap();
        let messages = vec![
            user("old", 1),          // 0: turn 1
            assistant("r0", 2),      // 1
            user("follow", 3),       // 2: turn 2 (safe)
            assistant_call("c7", 4), // 3
            inline_result_user,      // 4: turn 3 (UNSAFE start)
            assistant("done", 5),    // 5
        ];
        let sel = select(&messages, 1_000, 2, &HeuristicEstimator);
        assert_eq!(
            sel.tail_start_index,
            Some(2),
            "must cut at the safe turn 2 start"
        );
    }

    #[test]
    fn nothing_fits_summarises_everything() {
        let messages = vec![
            user("q1", 1),
            assistant("a1", 2),
            user("q2", 3),
            assistant("a2", 4),
        ];
        let sel = select(&messages, 0, 2, &HeuristicEstimator);
        assert_eq!(sel.tail_start_index, None);
        assert_eq!(sel.head_len, 4);
    }

    #[test]
    fn history_without_user_messages_is_all_head() {
        let messages = vec![assistant("a", 1), assistant("b", 2)];
        let sel = select(&messages, 1_000, 2, &HeuristicEstimator);
        assert_eq!(sel.tail_start_index, None);
    }
}
