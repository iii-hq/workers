//! Smoke tests that run without an iii engine connection.

#[test]
fn library_exports_register_entry_point() {
    // Compile-time check that the symbol is publicly wired.
    let _ = &shell_filesystem::register_with_iii;
}
