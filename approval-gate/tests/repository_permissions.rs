use approval_gate::permissions::{parse_rules_from_config, Decision, Permissions};
use approval_gate::types::PermissionMode;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct RepositoryPermissions {
    rules: Vec<Value>,
}

fn repository_permissions() -> Permissions {
    let config: RepositoryPermissions =
        serde_yaml::from_str(include_str!("../../iii-permissions.yaml"))
            .expect("repository permissions should be valid YAML");
    let specs = parse_rules_from_config(&Value::Array(config.rules));
    Permissions::compile(&specs).expect("repository permission rules should compile")
}

#[test]
fn configuration_list_is_allowed() {
    assert!(matches!(
        repository_permissions().check("configuration::list", &json!({}), PermissionMode::Manual),
        Decision::Allow { .. }
    ));
}

#[test]
fn configuration_schema_is_allowed() {
    assert!(matches!(
        repository_permissions().check("configuration::schema", &json!({}), PermissionMode::Manual),
        Decision::Allow { .. }
    ));
}

#[test]
fn configuration_get_needs_approval() {
    assert!(matches!(
        repository_permissions().check(
            "configuration::get",
            &json!({ "id": "database", "raw": true }),
            PermissionMode::Manual
        ),
        Decision::NeedsApproval
    ));
}

#[test]
fn configuration_set_is_denied() {
    assert!(matches!(
        repository_permissions().check("configuration::set", &json!({}), PermissionMode::Manual),
        Decision::Deny { .. }
    ));
}

#[test]
fn configuration_register_is_denied() {
    assert!(matches!(
        repository_permissions().check(
            "configuration::register",
            &json!({}),
            PermissionMode::Manual
        ),
        Decision::Deny { .. }
    ));
}
