//! Smoke tests that run without an iii engine connection.

#[test]
fn library_exports_subscribe_entry_point() {
    let _ = &audit_log::subscribe_audit_log;
}
