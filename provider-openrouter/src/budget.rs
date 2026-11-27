//! Output-token budget fitting for OpenRouter.
//!
//! OpenRouter checks two things against `max_tokens` before generating:
//!
//! * **Credits.** It reserves `prompt + max_tokens` at the model's price and
//!   answers 402 "This request requires more credits, or fewer max_tokens.
//!   You requested up to N tokens, but can only afford M" when the balance
//!   cannot cover the reservation — even if the real completion would cost
//!   cents.
//! * **Context.** `prompt + max_tokens` must fit the context window, else 400
//!   "maximum context length is C tokens. However, you requested about T
//!   tokens (… in the output)".
//!
//! The router forwards the model's catalog ceiling when no budget is
//! configured, and OpenRouter's listing advertises ceilings close to the
//! whole window for some models (Gemini 4: 943,718 of 1,048,576). Sent as-is,
//! that reserves dollars per turn and overflows the window as soon as the
//! prompt passes ~10% of it. This module clamps the budget to the room the
//! prompt leaves before sending, and derives a smaller budget from either
//! rejection so the request is retried instead of failing.
use serde_json::Value;

/// Never shrink a budget below this: a reply this small is still useful, and
/// below it the upstream error is the more honest answer.
pub const MIN_OUTPUT_TOKENS: u64 = 1024;

/// Conservative chars-per-token ratio for the prompt estimate (real ratios
/// run ~3.5-4.5 for English and code; JSON framing is counted too, so the
/// estimate errs high and the clamp errs on the safe side).
const CHARS_PER_TOKEN: u64 = 3;

/// Flat token charge for an inline (base64 `data:`) attachment instead of
/// its encoded length, which would wildly overstate an image's cost.
const INLINE_ATTACHMENT_TOKENS: u64 = 2_000;

/// Headroom kept free of the window for tokenizer drift.
fn safety_margin(context_window: u64) -> u64 {
    (context_window / 50).max(256)
}

fn count_chars(v: &Value, chars: &mut u64, attachments: &mut u64) {
    match v {
        Value::String(s) => {
            if s.starts_with("data:") {
                *attachments += 1;
            } else {
                *chars += s.len() as u64;
            }
        }
        Value::Array(items) => items
            .iter()
            .for_each(|i| count_chars(i, chars, attachments)),
        Value::Object(map) => map.iter().for_each(|(k, v)| {
            *chars += k.len() as u64;
            count_chars(v, chars, attachments);
        }),
        other => *chars += other.to_string().len() as u64,
    }
}

/// Rough upper-bound token count of what the request body sends as input:
/// `messages` and `tools`.
pub fn estimate_prompt_tokens(body: &Value) -> u64 {
    let (mut chars, mut attachments) = (0u64, 0u64);
    for key in ["messages", "tools"] {
        if let Some(v) = body.get(key) {
            count_chars(v, &mut chars, &mut attachments);
        }
    }
    chars.div_ceil(CHARS_PER_TOKEN) + attachments * INLINE_ATTACHMENT_TOKENS
}

/// The budget clamped to the room the prompt leaves in the window, floored at
/// [`MIN_OUTPUT_TOKENS`] (never raised above `requested`).
pub fn fit_to_context(requested: u64, context_window: u64, prompt_tokens: u64) -> u64 {
    let room = context_window
        .saturating_sub(prompt_tokens)
        .saturating_sub(safety_margin(context_window));
    requested.min(room.max(MIN_OUTPUT_TOKENS))
}

/// First run of ASCII digits (commas allowed) right after `marker`.
fn number_after(text: &str, marker: &str) -> Option<u64> {
    let start = text.find(marker)? + marker.len();
    let digits: String = text[start..]
        .chars()
        .skip_while(|c| c.is_whitespace())
        .take_while(|c| c.is_ascii_digit() || *c == ',')
        .filter(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// The run of digits immediately preceding `marker`.
fn number_before(text: &str, marker: &str) -> Option<u64> {
    let end = text.find(marker)?;
    let rev: String = text[..end]
        .chars()
        .rev()
        .skip_while(|c| c.is_whitespace())
        .take_while(|c| c.is_ascii_digit() || *c == ',')
        .filter(char::is_ascii_digit)
        .collect();
    rev.chars().rev().collect::<String>().parse().ok()
}

/// Why a smaller budget is being retried, for the final message's warnings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shrink {
    Credits { affordable: u64 },
    Context { context_window: u64, input: u64 },
}

/// From an upstream rejection, the smaller `max_tokens` that should succeed,
/// or `None` when the error is not a budget problem, carries no usable
/// numbers, or the fitted budget would not be smaller / would fall below
/// [`MIN_OUTPUT_TOKENS`].
pub fn shrink_from_error(status: u16, body: &str, current: u64) -> Option<(u64, Shrink)> {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.to_string());
    let lower = message.to_lowercase();

    let (next, why) = if status == 402 || lower.contains("can only afford") {
        let affordable = number_after(&lower, "can only afford")?;
        // Leave a little slack: the affordability figure is computed from a
        // prompt estimate on OpenRouter's side too.
        let next = affordable.saturating_sub(affordable / 20);
        (next, Shrink::Credits { affordable })
    } else if lower.contains("maximum context length") {
        let context_window = number_after(&lower, "maximum context length is")?;
        let total = number_after(&lower, "requested about")?;
        let output = number_before(&lower, "in the output").unwrap_or(current);
        let input = total.saturating_sub(output);
        let next = context_window
            .saturating_sub(input)
            .saturating_sub(safety_margin(context_window));
        (
            next,
            Shrink::Context {
                context_window,
                input,
            },
        )
    } else {
        return None;
    };
    (next >= MIN_OUTPUT_TOKENS && next < current).then_some((next, why))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn estimate_counts_text_and_flat_charges_inline_images() {
        let body = json!({
            "messages": [
                { "role": "user", "content": "a".repeat(3000) },
                { "role": "user", "content": [
                    { "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{}", "A".repeat(1_000_000)) } }
                ] }
            ],
            "max_tokens": 999_999
        });
        let est = estimate_prompt_tokens(&body);
        assert!(est >= 1000 + INLINE_ATTACHMENT_TOKENS, "{est}");
        assert!(
            est < 1100 + INLINE_ATTACHMENT_TOKENS,
            "base64 not counted: {est}"
        );
    }

    #[test]
    fn fit_clamps_to_the_room_left_and_never_raises() {
        // Gemini 4 shape: ceiling 943,718 in a 1,048,576 window, 300k prompt.
        let fitted = fit_to_context(943_718, 1_048_576, 300_000);
        assert_eq!(fitted, 1_048_576 - 300_000 - 1_048_576 / 50);
        // small prompt, small request: untouched
        assert_eq!(fit_to_context(8192, 1_048_576, 10_000), 8192);
        // prompt already fills the window: floor, upstream decides
        assert_eq!(
            fit_to_context(943_718, 1_048_576, 2_000_000),
            MIN_OUTPUT_TOKENS
        );
        // floor never raises a smaller request
        assert_eq!(fit_to_context(500, 4096, 4096), 500);
    }

    #[test]
    fn credits_rejection_retries_with_the_affordable_budget() {
        let body = r#"{"error":{"code":402,"message":"This request requires more credits, or fewer max_tokens. You requested up to 943718 tokens, but can only afford 51234. To increase, visit https://openrouter.ai/settings/credits and upgrade to a paid account"}}"#;
        let (next, why) = shrink_from_error(402, body, 943_718).unwrap();
        assert_eq!(next, 51_234 - 51_234 / 20);
        assert_eq!(why, Shrink::Credits { affordable: 51_234 });
    }

    #[test]
    fn genuinely_empty_balance_is_not_retried() {
        let body = r#"{"error":{"code":402,"message":"Insufficient credits. Add more using https://openrouter.ai/settings/credits"}}"#;
        assert_eq!(shrink_from_error(402, body, 943_718), None);
        let body = r#"{"error":{"code":402,"message":"You requested up to 8192 tokens, but can only afford 300."}}"#;
        assert_eq!(shrink_from_error(402, body, 8192), None, "below the floor");
    }

    #[test]
    fn context_rejection_retries_with_the_room_left() {
        let body = r#"{"error":{"code":400,"message":"This endpoint's maximum context length is 1048576 tokens. However, you requested about 1143718 tokens (180000 of text input, 20000 of tool input, 943718 in the output). Please reduce the length of either one, or use the \"middle-out\" transform to compress your prompt automatically."}}"#;
        let (next, why) = shrink_from_error(400, body, 943_718).unwrap();
        assert_eq!(
            why,
            Shrink::Context {
                context_window: 1_048_576,
                input: 200_000
            }
        );
        assert_eq!(next, 1_048_576 - 200_000 - 1_048_576 / 50);
    }

    #[test]
    fn prompt_alone_overflowing_is_not_retried() {
        let body = r#"{"error":{"code":400,"message":"This endpoint's maximum context length is 131072 tokens. However, you requested about 140000 tokens (139000 of text input, 1000 in the output)."}}"#;
        assert_eq!(shrink_from_error(400, body, 1000), None);
    }

    #[test]
    fn unrelated_errors_are_not_budget_problems() {
        let body = r#"{"error":{"code":429,"message":"Rate limited"}}"#;
        assert_eq!(shrink_from_error(429, body, 8192), None);
        assert_eq!(shrink_from_error(400, "bad request", 8192), None);
    }
}
