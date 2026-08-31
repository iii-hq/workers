//! Token estimation. v1 ships the serialized non-image chars/4 heuristic plus
//! a fixed image allowance behind a trait so a real tokenizer can slot in per
//! model later; responses report which estimator ran.

use crate::types::{AgentFunction, AgentMessage, ContentBlock, Role};

pub(crate) const IMAGE_TOKEN_BUDGET: u64 = 4_096;

fn clear_image_payloads(blocks: &mut [ContentBlock]) -> u64 {
    blocks.iter_mut().fold(0u64, |count, block| {
        let found = match block {
            ContentBlock::Image { data, .. } => {
                data.clear();
                1
            }
            ContentBlock::FunctionResult { content, .. } => clear_image_payloads(content),
            _ => 0,
        };
        count.saturating_add(found)
    })
}

/// Which estimator produced a count — `count-tokens` echoes this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EstimatorKind {
    Tokenizer,
    Heuristic,
}

pub trait Estimator: Send + Sync {
    fn kind(&self) -> EstimatorKind;

    /// Tokens of one message (serialized non-image form plus the fixed
    /// per-image allowance; structure and metadata still weigh in).
    fn message(&self, message: &AgentMessage) -> u64;

    /// Tokens of a bare string (system prompts, summaries).
    fn text(&self, text: &str) -> u64;

    /// Tokens of one invocation-schema entry.
    fn function(&self, function: &AgentFunction) -> u64;
}

/// `serialized non-image JSON chars / 4` plus 4,096 tokens per image —
/// deterministic and model-independent.
pub struct HeuristicEstimator;

impl Estimator for HeuristicEstimator {
    fn kind(&self) -> EstimatorKind {
        EstimatorKind::Heuristic
    }

    fn message(&self, message: &AgentMessage) -> u64 {
        let mut wire_view = message.clone();
        if let AgentMessage::FunctionResult { details, .. } = &mut wire_view {
            if !details.is_null()
                && details.get("status").and_then(serde_json::Value::as_str) != Some("denied")
            {
                *details = serde_json::Value::Null;
            }
        }

        let image_count = clear_image_payloads(wire_view.content_mut());
        let chars = serde_json::to_string(&wire_view)
            .map(|serialized| serialized.len())
            .unwrap_or(0);

        // ponytail: fixed media allowance; use model-aware accounting only if measured
        // provider usage exceeds the existing reserved-token cushion.
        ((chars / 4) as u64).saturating_add(image_count.saturating_mul(IMAGE_TOKEN_BUDGET))
    }

    fn text(&self, text: &str) -> u64 {
        (text.len() / 4) as u64
    }

    fn function(&self, function: &AgentFunction) -> u64 {
        let chars = serde_json::to_string(function)
            .map(|s| s.len())
            .unwrap_or(0);
        (chars / 4) as u64
    }
}

/// Estimator selection. v1: always the heuristic; a future tokenizer
/// keys off the resolved model here.
pub fn estimator_for_model(_model_id: &str) -> &'static dyn Estimator {
    static HEURISTIC: HeuristicEstimator = HeuristicEstimator;
    &HEURISTIC
}

/// Sum of message estimates.
pub fn estimate_messages(est: &dyn Estimator, messages: &[AgentMessage]) -> u64 {
    messages.iter().map(|m| est.message(m)).sum()
}

/// Per-role breakdown (`count-tokens.by_role`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ByRole {
    pub user: u64,
    pub assistant: u64,
    pub function_result: u64,
    pub custom: u64,
}

pub fn estimate_by_role(est: &dyn Estimator, messages: &[AgentMessage]) -> ByRole {
    let mut by_role = ByRole::default();
    for m in messages {
        let tokens = est.message(m);
        match m.role() {
            Role::User => by_role.user += tokens,
            Role::Assistant => by_role.assistant += tokens,
            Role::FunctionResult => by_role.function_result += tokens,
            Role::Custom => by_role.custom += tokens,
        }
    }
    by_role
}

/// Per-role breakdown from already-computed message sizes (`assemble`'s
/// memoized `sizes`), so callers with a size memo don't re-estimate.
pub fn by_role_from_sizes(messages: &[AgentMessage], sizes: &[u64]) -> ByRole {
    let mut by_role = ByRole::default();
    for (message, size) in messages.iter().zip(sizes) {
        match message.role() {
            Role::User => by_role.user += size,
            Role::Assistant => by_role.assistant += size,
            Role::FunctionResult => by_role.function_result += size,
            Role::Custom => by_role.custom += size,
        }
    }
    by_role
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(v: serde_json::Value) -> AgentMessage {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn heuristic_is_serialized_chars_over_four() {
        let m = msg(json!({
            "role": "user", "content": [{ "type": "text", "text": "hello" }], "timestamp": 1
        }));
        let chars = serde_json::to_string(&m).unwrap().len();
        let est = HeuristicEstimator;
        assert_eq!(est.message(&m), (chars / 4) as u64);
        assert_eq!(est.kind(), EstimatorKind::Heuristic);
    }

    #[test]
    fn text_estimate_is_chars_over_four() {
        assert_eq!(HeuristicEstimator.text("x".repeat(400).as_str()), 100);
        assert_eq!(HeuristicEstimator.text(""), 0);
    }

    #[test]
    fn function_result_details_are_not_counted() {
        // Live incident (2026-07-21, session scan-x7k2-c11-l2): 22 file reads
        // carried the full file in BOTH content and details; the double bill
        // inflated an ~88k wire request to 148_602 estimated > 148_000 usable
        // and killed the turn terminally. Details never cross the provider
        // wire, so they must not weigh in.
        let est = HeuristicEstimator;
        let fat = msg(json!({
            "role": "function_result", "function_call_id": "c", "function_id": "f",
            "content": [{ "type": "text", "text": "body" }],
            "details": { "content": "x".repeat(4000) }, "timestamp": 3
        }));
        let null_details = msg(json!({
            "role": "function_result", "function_call_id": "c", "function_id": "f",
            "content": [{ "type": "text", "text": "body" }],
            "details": null, "timestamp": 3
        }));
        assert_eq!(est.message(&fat), est.message(&null_details));
    }

    #[test]
    fn images_use_a_fixed_budget_without_base64() {
        let small = msg(json!({
            "role": "user",
            "content": [{ "type": "image", "mime": "image/png", "data": "AAAA" }],
            "timestamp": 1
        }));
        let incident = msg(json!({
            "role": "user",
            "content": [{
                "type": "image", "mime": "image/png", "data": "A".repeat(729_420)
            }],
            "timestamp": 1
        }));
        assert_eq!(
            HeuristicEstimator.message(&small),
            HeuristicEstimator.message(&incident)
        );

        let two_images = msg(json!({
            "role": "user",
            "content": [
                { "type": "image", "mime": "image/png", "data": "AAAA" },
                { "type": "function_result", "function_call_id": "c1", "content": [
                    { "type": "image", "mime": "image/jpeg", "data": "BBBB" }
                ] }
            ],
            "timestamp": 1
        }));
        let cleared = msg(json!({
            "role": "user",
            "content": [
                { "type": "image", "mime": "image/png", "data": "" },
                { "type": "function_result", "function_call_id": "c1", "content": [
                    { "type": "image", "mime": "image/jpeg", "data": "" }
                ] }
            ],
            "timestamp": 1
        }));
        let structural = (serde_json::to_string(&cleared).unwrap().len() / 4) as u64;
        assert_eq!(
            HeuristicEstimator.message(&two_images),
            structural + 2 * 4_096
        );
    }

    #[test]
    fn denied_details_envelope_is_counted() {
        // The denied envelope IS serialized into the wire body
        // ([PERMISSION_DENIED] + JSON), so it keeps weighing in.
        let est = HeuristicEstimator;
        let denied = msg(json!({
            "role": "function_result", "function_call_id": "c", "function_id": "f",
            "content": [], "details": { "status": "denied", "reason": "x".repeat(400) },
            "timestamp": 3
        }));
        let plain = msg(json!({
            "role": "function_result", "function_call_id": "c", "function_id": "f",
            "content": [], "details": { "status": "ok", "reason": "x".repeat(400) },
            "timestamp": 3
        }));
        assert!(est.message(&denied) > est.message(&plain));
    }

    #[test]
    fn by_role_partitions_every_role() {
        let messages = vec![
            msg(
                json!({ "role": "user", "content": [{ "type": "text", "text": "q" }], "timestamp": 1 }),
            ),
            msg(
                json!({ "role": "assistant", "content": [], "stop_reason": "end",
                        "model": "m", "provider": "p", "timestamp": 2 }),
            ),
            msg(
                json!({ "role": "function_result", "function_call_id": "c", "function_id": "f",
                        "content": [], "timestamp": 3 }),
            ),
            msg(json!({ "role": "custom", "custom_type": "t", "content": [], "timestamp": 4 })),
        ];
        let est = HeuristicEstimator;
        let by_role = estimate_by_role(&est, &messages);
        assert!(by_role.user > 0 && by_role.assistant > 0);
        assert!(by_role.function_result > 0 && by_role.custom > 0);
        assert_eq!(
            by_role.user + by_role.assistant + by_role.function_result + by_role.custom,
            estimate_messages(&est, &messages)
        );
    }

    #[test]
    fn by_role_from_sizes_partitions_every_role() {
        let messages = vec![
            msg(
                json!({ "role": "user", "content": [{ "type": "text", "text": "q" }], "timestamp": 1 }),
            ),
            msg(
                json!({ "role": "assistant", "content": [], "stop_reason": "end",
                        "model": "m", "provider": "p", "timestamp": 2 }),
            ),
            msg(
                json!({ "role": "function_result", "function_call_id": "c", "function_id": "f",
                        "content": [], "timestamp": 3 }),
            ),
            msg(json!({ "role": "custom", "custom_type": "t", "content": [], "timestamp": 4 })),
        ];
        let est = HeuristicEstimator;
        let sizes: Vec<u64> = messages.iter().map(|m| est.message(m)).collect();
        let by_role = by_role_from_sizes(&messages, &sizes);
        assert!(by_role.user > 0 && by_role.assistant > 0);
        assert!(by_role.function_result > 0 && by_role.custom > 0);
        assert_eq!(
            by_role.user + by_role.assistant + by_role.function_result + by_role.custom,
            sizes.iter().sum::<u64>()
        );
    }
}
