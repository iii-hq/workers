//! Cover the per-tool max-bytes contract enforced on every proxied response.

use iii_mcp_engine::truncate_result;
use serde_json::json;

#[test]
fn passes_through_under_limit() {
    let v = json!({"hello": "world"});
    let out = truncate_result(v.clone(), 1024);
    assert_eq!(out, v);
}

#[test]
fn truncates_when_over_limit() {
    let big = json!({ "data": "x".repeat(4096) });
    let original_len = big.to_string().len();
    let out = truncate_result(big, 256);

    assert_eq!(out["_truncated"], json!(true));
    assert_eq!(out["_max_bytes"], json!(256));
    assert_eq!(out["_original_bytes"], json!(original_len));
    assert!(out["preview"].is_string());
    assert!(out["preview"].as_str().unwrap().len() <= 256);
}

#[test]
fn handles_max_smaller_than_marker_overhead() {
    let v = json!({ "data": "x".repeat(128) });
    let out = truncate_result(v, 16);
    assert_eq!(out["_truncated"], json!(true));
    assert_eq!(out["preview"], json!(""));
}
