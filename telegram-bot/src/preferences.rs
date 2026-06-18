//! Per-chat preference resolution (KV overrides merged with worker config).

use crate::config::{ThinkingLevel, Verbosity, WorkerConfig};
use crate::deps::Deps;
use crate::kv;

pub async fn effective_verbosity(deps: &Deps, chat_id: i64, cfg: &WorkerConfig) -> Verbosity {
    if let Some(v) = kv::chat_verbosity_override(deps, chat_id).await {
        return v;
    }
    cfg.verbosity
}

pub async fn effective_thinking_level(
    deps: &Deps,
    chat_id: i64,
    cfg: &WorkerConfig,
) -> Option<ThinkingLevel> {
    if let Some(l) = kv::chat_thinking_level_override(deps, chat_id).await {
        return Some(l);
    }
    cfg.default_thinking_level
}

pub fn thinking_level_wire(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Minimal => "minimal",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::Xhigh => "xhigh",
    }
}

pub fn parse_thinking_level(s: &str) -> Option<ThinkingLevel> {
    match s.to_lowercase().as_str() {
        "off" | "none" => None,
        "minimal" => Some(ThinkingLevel::Minimal),
        "low" => Some(ThinkingLevel::Low),
        "medium" => Some(ThinkingLevel::Medium),
        "high" => Some(ThinkingLevel::High),
        "xhigh" | "x-high" => Some(ThinkingLevel::Xhigh),
        _ => None,
    }
}

pub fn parse_verbosity(s: &str) -> Option<Verbosity> {
    match s.to_lowercase().as_str() {
        "none" => Some(Verbosity::None),
        "minimal" => Some(Verbosity::Minimal),
        "high" => Some(Verbosity::High),
        "debug" => Some(Verbosity::Debug),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_thinking_levels() {
        assert!(parse_thinking_level("off").is_none());
        assert_eq!(parse_thinking_level("medium"), Some(ThinkingLevel::Medium));
        assert_eq!(parse_thinking_level("xhigh"), Some(ThinkingLevel::Xhigh));
    }

    #[test]
    fn parse_verbosity_levels() {
        assert_eq!(parse_verbosity("none"), Some(Verbosity::None));
        assert_eq!(parse_verbosity("debug"), Some(Verbosity::Debug));
        assert!(parse_verbosity("invalid").is_none());
    }

    #[test]
    fn thinking_level_wire_values() {
        assert_eq!(thinking_level_wire(ThinkingLevel::High), "high");
    }
}
