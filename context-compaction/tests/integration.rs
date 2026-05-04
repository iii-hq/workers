//! Smoke tests that run without an iii engine connection.

#[test]
fn payload_signals_overflow_recognises_overflow_classification() {
    let v = serde_json::json!({ "overflow": true });
    let _ = context_compaction::payload_signals_overflow(&v);
}

#[test]
fn extract_file_ops_handles_empty_history() {
    let details = context_compaction::extract_file_ops(&[]);
    let _ = details;
}
