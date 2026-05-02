//! Smoke tests that run without an iii engine connection.

#[test]
fn library_exports_register_entry_point() {
    let _ = &provider_bedrock::register_with_iii;
}
