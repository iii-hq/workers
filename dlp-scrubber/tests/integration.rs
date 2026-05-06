//! Smoke tests that run without an iii engine connection.

use serde_json::json;

#[test]
fn scrub_text_redacts_openai_key() {
    let key = format!("sk-{}", "0".repeat(40));
    let out = dlp_scrubber::scrub_text(&format!("apikey={key}"));
    assert!(out.contains("[REDACTED:openai]"));
}

#[test]
fn scrub_result_value_keeps_non_text_blocks_intact() {
    let v = json!({
        "content": [{ "type": "image", "data": "binary" }],
        "details": {},
    });
    assert_eq!(dlp_scrubber::scrub_result_value(&v), v);
}
