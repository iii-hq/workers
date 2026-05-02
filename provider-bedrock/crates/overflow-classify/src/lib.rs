//! Detect provider context-overflow errors and classify HTTP error kinds.
//!
//! The regex catalog is re-derived from each provider's public API documentation.
//! Patterns cover the surface area we have observed empirically; new patterns are
//! added when a provider's overflow shape changes.

use harness_types::ErrorKind;
use once_cell::sync::Lazy;
use regex::RegexSet;

/// Patterns that indicate a context-window overflow.
///
/// Provider notes:
/// - Anthropic Messages API: "prompt is too long: X tokens > Y maximum"; HTTP 413 "request_too_large".
/// - OpenAI Chat Completions / Responses: "exceeds the context window".
/// - Google Gemini: "input token count.*exceeds the maximum".
/// - Amazon Bedrock: "input is too long for requested model".
/// - xAI Grok: "maximum prompt length is N".
/// - Groq: "reduce the length of the messages".
/// - OpenRouter: "maximum context length is N tokens".
/// - llama.cpp server: "exceeds the available context size".
/// - LM Studio: "greater than the context length".
/// - GitHub Copilot: "exceeds the limit of N".
/// - MiniMax: "context window exceeds limit".
/// - Kimi for Coding: "exceeded model token limit".
/// - Mistral: "too large for model with N maximum context length".
/// - z.ai: surfaces non-standard finish_reason "model_context_window_exceeded".
/// - Ollama: "prompt too long; exceeded.*context length".
/// - Cerebras: HTTP 400/413 status code with no body in some deployments.
/// - Generic fallbacks for less-common deployments.
const OVERFLOW_PATTERNS: &[&str] = &[
    r"(?i)prompt is too long",
    r"(?i)request_too_large",
    r"(?i)input is too long for requested model",
    r"(?i)exceeds the context window",
    r"(?i)input token count.*exceeds the maximum",
    r"(?i)maximum prompt length is \d+",
    r"(?i)reduce the length of the messages",
    r"(?i)maximum context length is \d+ tokens",
    r"(?i)exceeds the limit of \d+",
    r"(?i)exceeds the available context size",
    r"(?i)greater than the context length",
    r"(?i)context window exceeds limit",
    r"(?i)exceeded model token limit",
    r"(?i)too large for model with \d+ maximum context length",
    r"(?i)model_context_window_exceeded",
    r"(?i)prompt too long; exceeded (?:max )?context length",
    r"(?i)context[_ ]length[_ ]exceeded",
    r"(?i)too many tokens",
    r"(?i)token limit exceeded",
    r"(?i)^4(?:00|13)\s*(?:status code)?\s*\(no body\)",
];

/// Patterns that match an OVERFLOW_PATTERN but indicate a non-overflow error.
/// Ordering matters: these win when both sets match.
const NON_OVERFLOW_PATTERNS: &[&str] = &[
    r"(?i)^(Throttling error|Service unavailable):",
    r"(?i)rate limit",
    r"(?i)too many requests",
];

static OVERFLOW_SET: Lazy<RegexSet> =
    Lazy::new(|| RegexSet::new(OVERFLOW_PATTERNS).expect("overflow patterns compile"));
static NON_OVERFLOW_SET: Lazy<RegexSet> =
    Lazy::new(|| RegexSet::new(NON_OVERFLOW_PATTERNS).expect("non-overflow patterns compile"));

/// True when the error text indicates a context-window overflow.
///
/// Returns false if the error matches an overflow pattern but is excluded by a
/// non-overflow pattern (rate-limit and throttling shapes).
pub fn is_overflow(error_text: &str) -> bool {
    if NON_OVERFLOW_SET.is_match(error_text) {
        return false;
    }
    OVERFLOW_SET.is_match(error_text)
}

/// Classify a streaming error by text and HTTP status into a stable kind.
///
/// Resolution order:
/// 1. Status 401 → `AuthExpired`
/// 2. Status 429 → `RateLimited`
/// 3. Overflow regex match → `ContextOverflow`
/// 4. Status 5xx (any) → `Transient`
/// 5. Status 4xx (other) → `Permanent`
/// 6. Fallback → `Transient`
pub fn classify_error(error_text: &str, http_status: Option<u16>) -> ErrorKind {
    if let Some(status) = http_status {
        match status {
            401 => return ErrorKind::AuthExpired,
            429 => return ErrorKind::RateLimited,
            _ => {}
        }
    }
    if is_overflow(error_text) {
        return ErrorKind::ContextOverflow;
    }
    match http_status {
        Some(s) if (500..600).contains(&s) => ErrorKind::Transient,
        Some(s) if (400..500).contains(&s) => ErrorKind::Permanent,
        _ => ErrorKind::Transient,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_overflow_messages() {
        assert!(is_overflow(
            "prompt is too long: 213462 tokens > 200000 maximum"
        ));
        assert!(is_overflow(
            "413 {\"error\":{\"type\":\"request_too_large\",\"message\":\"x\"}}"
        ));
    }

    #[test]
    fn openai_overflow_message() {
        assert!(is_overflow(
            "Your input exceeds the context window of this model"
        ));
    }

    #[test]
    fn google_overflow_message() {
        assert!(is_overflow(
            "The input token count (1196265) exceeds the maximum number of tokens allowed (1048575)"
        ));
    }

    #[test]
    fn xai_overflow_message() {
        assert!(is_overflow(
            "This model's maximum prompt length is 131072 but the request contains 537812 tokens"
        ));
    }

    #[test]
    fn groq_overflow_message() {
        assert!(is_overflow(
            "Please reduce the length of the messages or completion"
        ));
    }

    #[test]
    fn openrouter_overflow_message() {
        assert!(is_overflow(
            "This endpoint's maximum context length is 200000 tokens"
        ));
    }

    #[test]
    fn llama_cpp_overflow_message() {
        assert!(is_overflow(
            "the request exceeds the available context size, try increasing it"
        ));
    }

    #[test]
    fn lm_studio_overflow_message() {
        assert!(is_overflow(
            "tokens to keep from the initial prompt is greater than the context length"
        ));
    }

    #[test]
    fn copilot_overflow_message() {
        assert!(is_overflow(
            "prompt token count of 50000 exceeds the limit of 32768"
        ));
    }

    #[test]
    fn minimax_overflow_message() {
        assert!(is_overflow("invalid params, context window exceeds limit"));
    }

    #[test]
    fn kimi_overflow_message() {
        assert!(is_overflow(
            "Your request exceeded model token limit: 100000 (requested: 110000)"
        ));
    }

    #[test]
    fn mistral_overflow_message() {
        assert!(is_overflow(
            "Prompt contains 50000 tokens. This is too large for model with 32000 maximum context length"
        ));
    }

    #[test]
    fn ollama_overflow_message() {
        assert!(is_overflow(
            "prompt too long; exceeded max context length by 10000 tokens"
        ));
    }

    #[test]
    fn cerebras_no_body_overflow() {
        assert!(is_overflow("400 status code (no body)"));
        assert!(is_overflow("413 (no body)"));
    }

    #[test]
    fn generic_fallbacks() {
        assert!(is_overflow("context_length_exceeded"));
        assert!(is_overflow("token limit exceeded"));
    }

    #[test]
    fn bedrock_throttling_excluded() {
        // Bedrock throttling formats as "Throttling error: Too many tokens, please wait..."
        // which matches /too many tokens/ but is rate-limiting, not overflow.
        assert!(!is_overflow(
            "Throttling error: Too many tokens, please wait before trying again."
        ));
    }

    #[test]
    fn rate_limit_excluded() {
        assert!(!is_overflow("rate limit exceeded for this minute"));
        assert!(!is_overflow("Too many requests"));
    }

    #[test]
    fn classify_auth_expired_by_status() {
        assert_eq!(
            classify_error("anything", Some(401)),
            ErrorKind::AuthExpired
        );
    }

    #[test]
    fn classify_rate_limited_by_status() {
        assert_eq!(
            classify_error("anything", Some(429)),
            ErrorKind::RateLimited
        );
    }

    #[test]
    fn classify_context_overflow_wins_over_4xx() {
        assert_eq!(
            classify_error("prompt is too long", Some(400)),
            ErrorKind::ContextOverflow
        );
    }

    #[test]
    fn classify_transient_for_5xx() {
        assert_eq!(
            classify_error("server hiccup", Some(503)),
            ErrorKind::Transient
        );
    }

    #[test]
    fn classify_permanent_for_other_4xx() {
        assert_eq!(
            classify_error("bad request", Some(400)),
            ErrorKind::Permanent
        );
    }

    #[test]
    fn classify_transient_default() {
        assert_eq!(classify_error("network blip", None), ErrorKind::Transient);
    }
}
