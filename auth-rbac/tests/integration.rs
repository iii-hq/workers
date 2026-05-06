//! Smoke tests that run without an iii engine connection.

#[test]
fn role_satisfies_hierarchy() {
    use auth_rbac::Role;
    assert!(auth_rbac::role_satisfies(Role::Owner, Role::Admin));
    assert!(auth_rbac::role_satisfies(Role::Admin, Role::Member));
    assert!(auth_rbac::role_satisfies(Role::Member, Role::Viewer));
    assert!(!auth_rbac::role_satisfies(Role::Viewer, Role::Member));
}
