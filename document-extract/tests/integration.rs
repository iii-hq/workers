//! Smoke tests that run without an iii engine connection.

#[test]
fn document_format_round_trips() {
    let v = serde_json::to_value(document_extract::DocumentFormat::Pdf).expect("ser");
    assert_eq!(v, serde_json::json!("pdf"));
    let back: document_extract::DocumentFormat = serde_json::from_value(v).expect("de");
    assert_eq!(back, document_extract::DocumentFormat::Pdf);
}
