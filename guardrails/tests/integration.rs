//! Smoke tests that run without an iii engine connection.

#[test]
fn run_checks_passes_clean_text() {
    let result = guardrails::run_checks("hello world", None);
    assert!(result.allowed, "clean text should pass");
}

#[test]
fn run_checks_flags_pii_email() {
    let result = guardrails::run_checks("contact me at user@example.com", None);
    assert!(!result.allowed || !result.reasons.is_empty());
}
