//! Worker configuration. Pure helpers reading env vars with sane defaults.

const TRIGGER_TOKENS_ENV: &str = "COMPACT_TRIGGER_TOKENS";
const KEEP_RECENT_TURNS_ENV: &str = "COMPACT_KEEP_RECENT_TURNS";
const SUMMARIZER_PROVIDER_ENV: &str = "COMPACT_SUMMARIZER_PROVIDER";
const SUMMARIZER_MODEL_ENV: &str = "COMPACT_SUMMARIZER_MODEL";

const DEFAULT_TRIGGER_TOKENS: u64 = 60_000;
const DEFAULT_KEEP_RECENT_TURNS: usize = 3;

/// Token-count threshold above which compaction fires. Reads from
/// `COMPACT_TRIGGER_TOKENS`; falls back to a sensible default.
pub fn trigger_tokens() -> u64 {
    std::env::var(TRIGGER_TOKENS_ENV)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_TRIGGER_TOKENS)
}

/// Number of trailing turns to keep verbatim. Older turns become summary
/// input.
pub fn keep_recent_turns() -> usize {
    std::env::var(KEEP_RECENT_TURNS_ENV)
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_KEEP_RECENT_TURNS)
}

/// Optional provider override for the summariser call. `None` means
/// "use whatever the orchestrator's run_request had".
pub fn summarizer_provider() -> Option<String> {
    std::env::var(SUMMARIZER_PROVIDER_ENV)
        .ok()
        .filter(|s| !s.is_empty())
}

/// Optional model override for the summariser call.
pub fn summarizer_model() -> Option<String> {
    std::env::var(SUMMARIZER_MODEL_ENV)
        .ok()
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_within_safe_range() {
        // Don't assert exact equality because env vars may be set in
        // CI/dev — just bound the values.
        let t = trigger_tokens();
        assert!(t >= 1_000, "trigger threshold must be at least 1k tokens");
        assert!(t <= 1_000_000, "trigger threshold must be sane");

        let k = keep_recent_turns();
        assert!(k >= 1, "must keep at least 1 recent turn");
        assert!(k <= 100, "100 recent turns is already an over-budget cap");
    }
}
