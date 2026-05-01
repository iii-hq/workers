//! Smoke tests that run without an iii engine connection.

#[test]
fn config_from_env_requires_secret() {
    // SAFETY: tests run in process; remove the var if a previous test set it.
    std::env::remove_var("AUTH_HMAC_SECRET");
    let res = auth_rbac::AuthRbacConfig::from_env();
    assert!(
        res.is_err(),
        "expected from_env to fail without AUTH_HMAC_SECRET"
    );
}

#[test]
fn config_from_env_succeeds_with_secret() {
    std::env::set_var("AUTH_HMAC_SECRET", "test-secret-please-rotate");
    let cfg = auth_rbac::AuthRbacConfig::from_env().expect("should parse");
    assert!(!cfg.secret.is_empty());
    std::env::remove_var("AUTH_HMAC_SECRET");
}

#[test]
fn role_satisfies_hierarchy() {
    use auth_rbac::Role;
    assert!(auth_rbac::role_satisfies(Role::Owner, Role::Admin));
    assert!(auth_rbac::role_satisfies(Role::Admin, Role::Member));
    assert!(auth_rbac::role_satisfies(Role::Member, Role::Viewer));
    assert!(!auth_rbac::role_satisfies(Role::Viewer, Role::Member));
}
