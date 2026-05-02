//! Smoke tests that run without an iii engine connection.

#[test]
fn library_exports_register_entry_point() {
    let _ = &provider_cli::register_with_iii;
}

#[test]
fn cli_shapes_table_is_non_empty() {
    assert!(!provider_cli::CLI_SHAPES.is_empty());
}
