use sentinel::Redactor;

#[test]
fn pem_body_only_window_never_exposes_body_and_keeps_original_lines() {
    let input =
        "before\n-----BEGIN PRIVATE KEY-----\nQUJDREVGR0g=\n-----END PRIVATE KEY-----\nafter\n";
    let (safe, hits) = Redactor::default().redact_source(input).unwrap();
    assert!(hits > 0);
    assert!(!safe.contains("QUJDREVGR0g="));
    assert_eq!(safe.lines().count(), 5);
    assert_eq!(safe.lines().next(), Some("before"));
    assert_eq!(safe.lines().nth(4), Some("after"));
    assert!(safe.lines().nth(2).unwrap().contains("redacted"));
}

#[test]
fn incomplete_or_mismatched_pem_is_refused_not_partially_returned() {
    for input in [
        "QUJDREVGR0g=\n-----END PRIVATE KEY-----\n",
        "-----BEGIN PRIVATE KEY-----\nQUJDREVGR0g=\n",
        "-----BEGIN PRIVATE KEY-----\nQUJDREVGR0g=\n-----END RSA PRIVATE KEY-----",
        "-----BEGIN PRIVATE KEY---\nQUJDREVGR0g=",
        "QUJDREVGR0g=\n-----END PRIVATE KEY---",
    ] {
        assert!(
            Redactor::default().redact_source(input).is_err(),
            "incomplete synthetic PEM must fail closed"
        );
    }
}

#[test]
fn operator_multiline_pattern_preserves_source_line_positions() {
    let redactor = Redactor::new(&["(?s)SENSITIVE_START.*?SENSITIVE_END".into()]).unwrap();
    let (safe, _) = redactor
        .redact_source("first\nSENSITIVE_START\nsynthetic-private\nSENSITIVE_END\nlast")
        .unwrap();
    assert_eq!(safe.lines().count(), 5);
    assert_eq!(safe.lines().nth(4), Some("last"));
    assert!(!safe.contains("synthetic-private"));
}
