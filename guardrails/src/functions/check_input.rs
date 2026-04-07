use std::sync::Arc;

use iii_sdk::{IIIError, III};
use regex::Regex;
use serde_json::Value;

use crate::checks::{check_injection, check_length, check_pii, classify_risk};
use crate::config::GuardrailsConfig;
use crate::state;

pub async fn handle(
    iii: Arc<III>,
    config: Arc<GuardrailsConfig>,
    compiled_patterns: Arc<Vec<(String, Regex)>>,
    payload: Value,
) -> Result<Value, IIIError> {
    let text = payload
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: text".to_string()))?
        .to_string();

    let context = payload.get("context").cloned().unwrap_or(serde_json::json!({}));

    let pii_matches = check_pii(&text, &compiled_patterns);
    let injection_matches = check_injection(&text, &config.injection_keywords);
    let within_length = check_length(&text, config.max_input_length);

    let pii_count: usize = pii_matches.iter().map(|m| m.count).sum();
    let risk = classify_risk(pii_count, injection_matches.len(), !within_length);
    let passed = risk == "none" || risk == "low";

    let check_id = format!(
        "chk-in-{}-{}",
        chrono::Utc::now().timestamp_millis(),
        &text.len()
    );

    let pii_json: Vec<Value> = pii_matches
        .iter()
        .map(|m| {
            serde_json::json!({
                "pattern_name": m.pattern_name,
                "count": m.count,
            })
        })
        .collect();

    let injection_json: Vec<Value> = injection_matches
        .iter()
        .map(|m| {
            serde_json::json!({
                "keyword": m.keyword,
                "position": m.position,
            })
        })
        .collect();

    let result = serde_json::json!({
        "passed": passed,
        "risk": risk,
        "pii": pii_json,
        "injections": injection_json,
        "over_length": !within_length,
        "check_id": check_id,
    });

    let audit_record = serde_json::json!({
        "check_id": check_id,
        "type": "input",
        "risk": risk,
        "passed": passed,
        "pii_count": pii_count,
        "injection_count": injection_matches.len(),
        "over_length": !within_length,
        "text_length": text.len(),
        "context": context,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });

    if let Err(e) = state::state_set(&iii, "guardrails:checks", &check_id, audit_record).await {
        tracing::warn!(error = %e, check_id = %check_id, "failed to store audit record");
    }

    Ok(result)
}
