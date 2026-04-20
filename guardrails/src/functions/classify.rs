use std::sync::Arc;

use iii_sdk::IIIError;
use regex::Regex;
use serde_json::Value;

use crate::checks::{check_injection, check_length, check_pii, check_secrets, classify_risk};
use crate::config::GuardrailsConfig;

pub async fn handle(
    config: Arc<GuardrailsConfig>,
    compiled_patterns: Arc<Vec<(String, Regex)>>,
    compiled_secrets: Arc<Vec<(String, Regex)>>,
    payload: Value,
) -> Result<Value, IIIError> {
    let text = payload
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: text".to_string()))?
        .to_string();

    let pii_matches = check_pii(&text, &compiled_patterns);
    let injection_matches = check_injection(&text, &config.injection_keywords);
    let secret_matches = check_secrets(&text, &compiled_secrets);
    let within_input = check_length(&text, config.max_input_length);

    let pii_count: usize = pii_matches.iter().map(|m| m.count).sum();
    let secret_count: usize = secret_matches.iter().map(|m| m.count).sum();

    let mut categories: Vec<&str> = Vec::new();
    if pii_count > 0 {
        categories.push("pii");
    }
    if !injection_matches.is_empty() {
        categories.push("injection");
    }
    if secret_count > 0 {
        categories.push("secrets");
    }
    if !within_input {
        categories.push("over_length");
    }

    let risk = classify_risk(
        pii_count + secret_count,
        injection_matches.len(),
        !within_input,
    );

    let pii_types: Vec<&str> = pii_matches
        .iter()
        .map(|m| m.pattern_name.as_str())
        .collect();

    let result = serde_json::json!({
        "risk": risk,
        "categories": categories,
        "pii_types": pii_types,
        "details": {
            "pii_count": pii_count,
            "injection_count": injection_matches.len(),
            "secret_count": secret_count,
            "text_length": text.len(),
            "within_input_limit": within_input,
        },
    });

    Ok(result)
}
