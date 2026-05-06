use serde::{Deserialize, Serialize};

/// Reasoning effort level requested from a model. Providers map these to native
/// shapes (token budget for budget-based models, effort tier for tier-based, ignored
/// for unsupported providers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    /// Highest tier; only supported by selected model families.
    Xhigh,
}

impl Default for ThinkingLevel {
    fn default() -> Self {
        Self::Off
    }
}

/// Per-level token budgets for token-budget-based providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ThinkingBudgets {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimal: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub medium: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high: Option<u32>,
}

/// Cryptographic signature on a span of text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextSignature {
    pub v: u8,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<TextPhase>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextPhase {
    Commentary,
    FinalAnswer,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_serialises_lowercase() {
        assert_eq!(
            serde_json::to_string(&ThinkingLevel::Xhigh).unwrap(),
            "\"xhigh\""
        );
        assert_eq!(
            serde_json::to_string(&ThinkingLevel::Off).unwrap(),
            "\"off\""
        );
    }

    #[test]
    fn budgets_omit_nones() {
        let b = ThinkingBudgets {
            low: Some(1024),
            ..Default::default()
        };
        let json = serde_json::to_string(&b).unwrap();
        assert_eq!(json, r#"{"low":1024}"#);
    }
}
