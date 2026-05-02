//! Smoke tests that run without an iii engine connection.

#[tokio::test]
async fn scan_secrets_on_empty_input_yields_empty_report() {
    let report = session_corpus::scan_secrets("").await.expect("scan ok");
    assert!(report.matches.is_empty());
}

#[test]
fn redact_with_no_secrets_returns_input_unchanged() {
    let input = "hello world";
    let out = session_corpus::redact(input, &[], &[]).expect("redact ok");
    assert_eq!(out, input);
}
