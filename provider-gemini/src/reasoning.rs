//! thinking_level → the Interactions `generation_config.thinking_level`.
//!
//! An unsupported level is a 400 upstream, not a clamp: "Thinking level
//! THINKING_LEVEL_MINIMAL is not supported for this model. Please retry with
//! other thinking level." (the same for `xhigh`). So the level is clamped here
//! into the row's range, and an unlisted id is never sent `minimal` or `xhigh`.
use llm_router::types::model::ThinkingLevel;
use llm_router::types::model::ThinkingLevel::{High, Low, Medium, Minimal, Xhigh};

const WITH_XHIGH: &[ThinkingLevel] = &[Low, Medium, High, Xhigh];
const WITH_MINIMAL: &[ThinkingLevel] = &[Minimal, Low, Medium, High];
const DEFAULT_LEVELS: &[ThinkingLevel] = &[Low, Medium, High];

/// Rows checked live per id; anything else gets `DEFAULT_LEVELS`.
const ROWS: &[(&str, &[ThinkingLevel])] = &[
    ("gemini-4-neon", WITH_XHIGH),
    ("gemini-4-hydrogen", WITH_XHIGH),
    ("gemini-3.6-flash", WITH_MINIMAL),
    ("gemini-3.5-flash", WITH_MINIMAL),
    ("gemini-3.5-flash-lite", WITH_MINIMAL),
    ("gemini-3.1-flash-lite", WITH_MINIMAL),
    ("gemini-3-flash-preview", WITH_MINIMAL),
];

/// The row whose id equals `model` or is continued by it with `-`; the
/// longest id wins.
pub(crate) fn longest_row<'a, T>(rows: &'a [(&'a str, T)], model: &str) -> Option<&'a T> {
    rows.iter()
        .filter(|(id, _)| {
            model
                .strip_prefix(id)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
        })
        .max_by_key(|(id, _)| id.len())
        .map(|(_, v)| v)
}

/// Levels `model` accepts, ascending.
pub fn supported_levels(model: &str) -> &'static [ThinkingLevel] {
    longest_row(ROWS, model).copied().unwrap_or(DEFAULT_LEVELS)
}

pub fn wire_level(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Off | Minimal => "minimal",
        Low => "low",
        Medium => "medium",
        High => "high",
        Xhigh => "xhigh",
    }
}

/// The wire `thinking_level` for `level` on `model`, plus an optional
/// report-and-continue warning. `None` omits the field.
pub fn thinking_level_for(
    level: Option<ThinkingLevel>,
    model: &str,
) -> (Option<&'static str>, Option<String>) {
    let Some(level) = level else {
        return (None, None);
    };
    let levels = supported_levels(model);
    let (lowest, highest) = (levels[0], levels[levels.len() - 1]);
    if level == ThinkingLevel::Off {
        // The Interactions schema has no off switch.
        let wire = wire_level(lowest);
        let warning = format!("thinking cannot be switched off on {model}; ran at {wire}");
        return (Some(wire), Some(warning));
    }
    let clamped = level.clamp(lowest, highest);
    let wire = wire_level(clamped);
    let warning = (clamped != level).then(|| {
        format!(
            "thinking_level {} is not supported on {model}; ran at {wire}",
            wire_level(level)
        )
    });
    (Some(wire), warning)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_match_the_live_checks() {
        for id in ["gemini-4-neon", "gemini-4-hydrogen"] {
            assert_eq!(supported_levels(id), WITH_XHIGH, "{id}");
        }
        for id in [
            "gemini-3.6-flash",
            "gemini-3.5-flash",
            "gemini-3.5-flash-lite",
            "gemini-3.1-flash-lite",
            "gemini-3.1-flash-lite-preview",
            "gemini-3-flash-preview",
        ] {
            assert_eq!(supported_levels(id), WITH_MINIMAL, "{id}");
        }
        for id in [
            "gemini-4-carbon",
            "gemini-4-argon",
            "gemini-3.8-flash",
            "gemini-3.7-flash",
            "gemini-3.1-pro-preview",
            "gemini-flash-latest",
            "gemini-3.9-flash",
            "gemini-3.6-flashy",
        ] {
            assert_eq!(supported_levels(id), DEFAULT_LEVELS, "{id}");
        }
    }

    #[test]
    fn none_omits_the_field() {
        assert_eq!(thinking_level_for(None, "gemini-3.8-flash"), (None, None));
    }

    #[test]
    fn levels_inside_the_range_pass_unchanged() {
        assert_eq!(
            thinking_level_for(Some(Minimal), "gemini-3.1-flash-lite-preview"),
            (Some("minimal"), None)
        );
        assert_eq!(
            thinking_level_for(Some(Xhigh), "gemini-4-hydrogen"),
            (Some("xhigh"), None)
        );
        assert_eq!(
            thinking_level_for(Some(Medium), "gemini-3.8-flash"),
            (Some("medium"), None)
        );
    }

    #[test]
    fn unsupported_levels_clamp_with_a_warning() {
        let (wire, warn) = thinking_level_for(Some(Minimal), "gemini-3.8-flash");
        assert_eq!(wire, Some("low"));
        assert!(warn.unwrap().contains("ran at low"));
        let (wire, warn) = thinking_level_for(Some(Xhigh), "gemini-4-carbon");
        assert_eq!(wire, Some("high"));
        assert!(warn.is_some());
        assert_eq!(
            thinking_level_for(Some(Minimal), "gemini-4-hydrogen").0,
            Some("low")
        );
        assert_eq!(
            thinking_level_for(Some(Xhigh), "gemini-3.6-flash").0,
            Some("high")
        );
    }

    #[test]
    fn unknown_ids_never_get_minimal_or_xhigh() {
        assert_eq!(
            thinking_level_for(Some(Minimal), "gemini-3.9-flash").0,
            Some("low")
        );
        assert_eq!(
            thinking_level_for(Some(Xhigh), "gemini-3.9-flash").0,
            Some("high")
        );
    }

    #[test]
    fn off_runs_at_the_lowest_level_and_warns() {
        let (wire, warn) = thinking_level_for(Some(ThinkingLevel::Off), "gemini-3.6-flash");
        assert_eq!(wire, Some("minimal"));
        assert_eq!(
            warn.as_deref(),
            Some("thinking cannot be switched off on gemini-3.6-flash; ran at minimal")
        );
        let (wire, _) = thinking_level_for(Some(ThinkingLevel::Off), "gemini-3.8-flash");
        assert_eq!(wire, Some("low"));
    }
}
