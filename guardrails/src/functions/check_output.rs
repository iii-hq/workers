use std::sync::Arc;

use iii_sdk::{IIIError, III};
use regex::Regex;
use serde_json::Value;

use crate::checks::{check_length, check_pii, check_secrets, classify_risk};
use crate::config::GuardrailsConfig;
use crate::state;

pub async fn handle(
    iii: Arc<III>,
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

    let context = payload
        .get("context")
        .cloned()
        .unwrap_or(serde_json::json!({}));

    let pii_matches = check_pii(&text, &compiled_patterns);
    let secret_matches = check_secrets(&text, &compiled_secrets);
    let within_length = check_length(&text, config.max_output_length);

    let pii_count: usize = pii_matches.iter().map(|m| m.count).sum();
    let secret_count: usize = secret_matches.iter().map(|m| m.count).sum();
    let risk = classify_risk(pii_count + secret_count, 0, !within_length);
    let passed = risk == "none" || risk == "low";

    let check_id = format!(
        "chk-out-{}-{}",
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

    let secrets_json: Vec<Value> = secret_matches
        .iter()
        .map(|m| {
            serde_json::json!({
                "pattern_name": m.pattern_name,
                "count": m.count,
            })
        })
        .collect();

    let result = serde_json::json!({
        "passed": passed,
        "risk": risk,
        "pii": pii_json,
        "secrets": secrets_json,
        "over_length": !within_length,
        "check_id": check_id,
    });

    let audit_record = serde_json::json!({
        "check_id": check_id,
        "type": "output",
        "risk": risk,
        "passed": passed,
        "pii_count": pii_count,
        "secret_count": secret_count,
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
