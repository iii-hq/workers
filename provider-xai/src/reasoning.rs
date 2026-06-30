//! thinking_level → Chat Completions `reasoning_effort`, per grok family.
//! Each branch traces to xAI's documented behaviour — a wrong effort string
//! fails the whole request.
//!
//! xAI specifics (verified empirically against api.x.ai, 2026-06):
//!   - `grok-4.3` accepts the full ladder `none`/`low`/`medium`/`high`/`xhigh`.
//!   - `grok-3-mini` accepts `low`/`high` only.
//!   - every other current model — `grok-build`, the `grok-4.20-*` snapshots
//!     (reasoning, non-reasoning, multi-agent) — returns HTTP 400
//!     "does not support parameter reasoningEffort", so the param is omitted.
use llm_router::types::model::ThinkingLevel;

/// Full effort vocabulary in ascending order (superset across families).
const EFFORT_ORDER: [&str; 6] = ["none", "minimal", "low", "medium", "high", "xhigh"];

/// Reasoning model detection: the catalog's `supports_thinking` flag wins;
/// id-pattern fallback for models the catalog doesn't know. grok-4.x and
/// grok-3-mini reason; grok-3 (non-mini) and grok-build do not.
pub fn is_reasoning_model(model: &str, catalog_supports_thinking: Option<bool>) -> bool {
    if let Some(flag) = catalog_supports_thinking {
        return flag;
    }
    let id = model.to_ascii_lowercase();
    id.starts_with("grok-4") || id.starts_with("grok-3-mini") || id.contains("code-fast")
}

/// Efforts the model family accepts; empty = don't send the param. Verified
/// against api.x.ai: `grok-4.3` takes the full ladder, `grok-3-mini` takes
/// `low`/`high`, and every other current model rejects the param with a 400.
fn supported_efforts(model: &str) -> &'static [&'static str] {
    let id = model.to_ascii_lowercase();
    if id.starts_with("grok-4.3") {
        return &["none", "low", "medium", "high", "xhigh"];
    }
    if id.starts_with("grok-3-mini") {
        return &["low", "high"];
    }
    &[]
}

fn level_str(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Minimal => "minimal",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::Xhigh => "xhigh",
    }
}

/// Effort for a reasoning model: the requested level when the family supports
/// it, else the nearest supported effort below (then above). `None` when the
/// family takes no effort param or no level was requested.
pub fn reasoning_effort_for(level: Option<ThinkingLevel>, model: &str) -> Option<&'static str> {
    let ladder = supported_efforts(model);
    if ladder.is_empty() {
        return None;
    }
    let want = level_str(level?);
    if ladder.contains(&want) {
        return Some(want);
    }
    let want_idx = EFFORT_ORDER.iter().position(|e| *e == want)?;
    if let Some(&c) = EFFORT_ORDER[..want_idx]
        .iter()
        .rev()
        .find(|&&c| ladder.contains(&c))
    {
        return Some(c);
    }
    EFFORT_ORDER[want_idx + 1..]
        .iter()
        .find(|&&c| ladder.contains(&c))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_flag_wins_over_id_pattern() {
        assert!(is_reasoning_model("weird-model", Some(true)));
        assert!(!is_reasoning_model("grok-4", Some(false)));
        assert!(is_reasoning_model("grok-4", None));
        assert!(is_reasoning_model("grok-3-mini", None));
        assert!(!is_reasoning_model("grok-3", None));
        assert!(!is_reasoning_model("grok-build-0.1", None));
    }

    #[test]
    fn effort_param_per_model_matches_api() {
        // grok-4.3 takes the full ladder
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "grok-4.3"),
            Some("high")
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Xhigh), "grok-4.3"),
            Some("xhigh")
        );
        // grok-3-mini takes low/high
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Low), "grok-3-mini"),
            Some("low")
        );
        // the grok-4.20-* snapshots and grok-build reject the param entirely
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "grok-4.20-0309-reasoning"),
            None
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "grok-4.20-multi-agent-0309"),
            None
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "grok-build-0.1"),
            None
        );
    }

    #[test]
    fn unsupported_level_degrades_to_nearest_supported() {
        // grok-3-mini has no medium → nearest below is low
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Medium), "grok-3-mini"),
            Some("low")
        );
        // no minimal → nearest above is low
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Minimal), "grok-3-mini"),
            Some("low")
        );
        // no xhigh → nearest below is high
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Xhigh), "grok-3-mini"),
            Some("high")
        );
    }

    #[test]
    fn absent_level_omits_the_param() {
        assert_eq!(reasoning_effort_for(None, "grok-3-mini"), None);
    }
}
