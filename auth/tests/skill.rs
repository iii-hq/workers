#[test]
fn skill_starts_with_heading_and_lists_functions() {
    let body = include_str!("../skills/index.md");
    assert!(body.starts_with("# auth\n"));
    assert!(body.contains("auth::validate"));
    assert!(body.contains("auth::server_metadata"));
    assert!(body.contains("auth::resource_metadata"));
    assert!(body.contains("auth::register"));
    assert!(body.contains("auth::jwks"));
    assert!(body.contains("auth::jwks_rotate"));
    assert!(body.contains("auth::token"));
    assert!(body.contains("auth::introspect"));
    assert!(body.contains("auth::revoke"));
}
