use approval_gate::permissions::{parse_rules_from_config, Decision, Permissions};
use approval_gate::types::PermissionMode;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct RepositoryPermissions {
    rules: Vec<Value>,
}

fn repository_permissions() -> Permissions {
    permissions_from_yaml(include_str!("../../iii-permissions.yaml"))
}

fn permissions_from_yaml(source: &str) -> Permissions {
    let config: RepositoryPermissions =
        serde_yaml::from_str(source).expect("repository permissions should be valid YAML");
    let specs = parse_rules_from_config(&Value::Array(config.rules));
    Permissions::compile(&specs).expect("repository permission rules should compile")
}

#[test]
fn mesh_reads_are_allowed_in_root_and_worker_permissions() {
    for permissions in [
        repository_permissions(),
        permissions_from_yaml(include_str!("../../mesh/iii-permissions.yaml")),
    ] {
        for id in ["mesh::status", "mesh::nodes::list", "mesh::nodes::get"] {
            assert!(
                matches!(
                    permissions.check(id, &json!({}), PermissionMode::Manual),
                    Decision::Allow { .. }
                ),
                "{id} should be allowed"
            );
        }
    }
}

#[test]
fn mesh_internal_handlers_are_denied_in_root_and_worker_permissions() {
    for permissions in [
        repository_permissions(),
        permissions_from_yaml(include_str!("../../mesh/iii-permissions.yaml")),
    ] {
        for id in [
            "mesh::auth",
            "mesh::middleware",
            "mesh::on-trigger-registration",
            "mesh::on-config-change",
            "mesh::ui-content",
            "mesh::configuration-id",
            "mesh-node::status",
            "mesh-node::update",
            "mesh-node::workers::apply",
            "mesh-node::drives::apply",
            "laptop::mesh-node::status",
            "laptop::mesh-node::update",
            "laptop::mesh-node::workers::apply",
            "laptop::mesh-node::drives::apply",
            "tenant::laptop::mesh-node::update",
        ] {
            assert!(
                matches!(
                    permissions.check(id, &json!({}), PermissionMode::Manual),
                    Decision::Deny { .. }
                ),
                "{id} should be denied"
            );
        }
    }
}

#[test]
fn remote_shell_internals_are_denied_in_root_and_worker_permissions() {
    for permissions in [
        repository_permissions(),
        permissions_from_yaml(include_str!("../../mesh/iii-permissions.yaml")),
    ] {
        for id in [
            "laptop::shell::pty::sessions",
            "laptop::shell::pty::open",
            "laptop::shell::scm::commit-message",
            "laptop::shell::ui-state::get",
            "laptop::shell::config-status",
            "laptop::shell::kill-session-jobs",
            "laptop::shell::on-config-change",
            "tenant::laptop::shell::pty::open",
        ] {
            assert!(
                matches!(
                    permissions.check(id, &json!({}), PermissionMode::Manual),
                    Decision::Deny { .. }
                ),
                "{id} should be denied"
            );
        }
    }
}

#[test]
fn remote_tools_and_mesh_management_need_approval() {
    for permissions in [
        repository_permissions(),
        permissions_from_yaml(include_str!("../../mesh/iii-permissions.yaml")),
    ] {
        for id in [
            "laptop::shell::exec",
            "laptop::coder::read-file",
            "tenant::coder::read-file",
            "mesh::nodes::remove",
            "mesh::join::code",
            "mesh::join::cycle",
            "mesh::join::token",
            "mesh::join::revoke",
        ] {
            assert!(
                matches!(
                    permissions.check(id, &json!({}), PermissionMode::Manual),
                    Decision::NeedsApproval
                ),
                "{id} should need approval"
            );
        }
    }
}

#[test]
fn mesh_rules_preserve_local_shell_and_coder_decisions() {
    let permissions = repository_permissions();
    for id in [
        "shell::config-status",
        "shell::pty::open",
        "shell::scm::commit-message",
    ] {
        assert!(
            matches!(
                permissions.check(id, &json!({}), PermissionMode::Manual),
                Decision::Deny { .. }
            ),
            "{id} should remain denied"
        );
    }
    for id in ["shell::pty::sessions", "coder::read-file"] {
        assert!(
            matches!(
                permissions.check(id, &json!({}), PermissionMode::Manual),
                Decision::Allow { .. }
            ),
            "{id} should remain allowed"
        );
    }
    assert!(matches!(
        permissions.check("shell::exec", &json!({}), PermissionMode::Manual),
        Decision::NeedsApproval
    ));
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

/// The new atomic initializer must not bypass the existing configuration-write restrictions.
#[test]
fn configuration_ensure_is_denied() {
    // The atomic register/seed twin must be denied wherever register is, so an
    // agent cannot bypass the schema/seed protection through the new name.
    assert!(matches!(
        repository_permissions().check("configuration::ensure", &json!({}), PermissionMode::Manual),
        Decision::Deny { .. }
    ));
}

/// `harness::on-session-deleted` is the `session::on-deleted` cleanup target, engine
/// plumbing like the other internal harness hooks. Called directly with a live
/// session id, it would wipe that session's turn record and drop its observers.
#[test]
fn harness_on_session_deleted_is_denied() {
    assert!(matches!(
        repository_permissions().check(
            "harness::on-session-deleted",
            &json!({ "session_id": "s_live" }),
            PermissionMode::Manual
        ),
        Decision::Deny { .. }
    ));
}

/// Listing templates reads the templates checkout or the IDE's own cache; it
/// writes nothing in the project, so agents call it without a prompt.
#[test]
fn coder_list_templates_is_allowed() {
    assert!(matches!(
        repository_permissions().check("coder::list-templates", &json!({}), PermissionMode::Manual),
        Decision::Allow { .. }
    ));
}

/// Scaffolding writes a whole worker package, so it stays approval-gated like
/// coder::create-file: no rule, the needs_approval default.
#[test]
fn coder_scaffold_worker_needs_approval() {
    assert!(matches!(
        repository_permissions().check(
            "coder::scaffold-worker",
            &json!({ "template": "worker-node-ade", "name": "my-worker" }),
            PermissionMode::Manual
        ),
        Decision::NeedsApproval
    ));
}

/// Read-only cross-references (kanban #1384): allowed like kanban::ticket::get.
#[test]
fn kanban_ticket_links_is_allowed() {
    assert!(matches!(
        repository_permissions().check(
            "kanban::ticket::links",
            &json!({ "id": "KAN-1" }),
            PermissionMode::Manual
        ),
        Decision::Allow { .. }
    ));
}

/// The kanban reads and the ticket working loop are allowed, including the
/// project and release lists agents use to file a ticket.
#[test]
fn kanban_reads_and_the_ticket_loop_are_allowed() {
    let permissions = repository_permissions();
    for id in [
        "kanban::config::info",
        "kanban::board::get",
        "kanban::ticket::list",
        "kanban::ticket::get",
        "kanban::ticket::links",
        "kanban::ticket::create",
        "kanban::ticket::update",
        "kanban::ticket::move",
        "kanban::project::list",
        "kanban::release::list",
        "kanban::comment::create",
        "kanban::comment::list",
        "kanban::activity::list",
        "kanban::agent::list",
    ] {
        assert!(
            matches!(
                permissions.check(id, &json!({}), PermissionMode::Manual),
                Decision::Allow { .. }
            ),
            "{id} should be allowed"
        );
    }
}

/// The deepwork orchestrator files each run under its own project: a create
/// whose `lead` is `deepwork` is allowed; any other create matches no rule and
/// waits for a person (never a blanket allow or deny).
#[test]
fn only_a_deepwork_run_project_create_is_allowed() {
    let permissions = repository_permissions();
    assert!(matches!(
        permissions.check(
            "kanban::project::create",
            &json!({"name": "deepwork: goal · a7f442", "lead": "deepwork"}),
            PermissionMode::Manual
        ),
        Decision::Allow { .. }
    ));
    for args in [json!({}), json!({"name": "Q3", "lead": "tech-lead"})] {
        assert!(
            matches!(
                permissions.check("kanban::project::create", &args, PermissionMode::Manual),
                Decision::NeedsApproval
            ),
            "{args} should wait for a person"
        );
    }
}

/// Planning (projects, releases, label conversion) and ticket deletion are the
/// human's call from the console; agents are denied, as in kanban's own policy.
/// Project creation is the one scoped exception (above).
#[test]
fn kanban_planning_and_ticket_deletion_are_denied() {
    let permissions = repository_permissions();
    for id in [
        "kanban::ticket::delete",
        "kanban::ticket::restore",
        "kanban::project::update",
        "kanban::project::archive",
        "kanban::project::restore",
        "kanban::release::create",
        "kanban::release::update",
        "kanban::release::ship",
        "kanban::release::reopen",
        "kanban::release::delete",
        "kanban::label::convert",
    ] {
        assert!(
            matches!(
                permissions.check(id, &json!({}), PermissionMode::Manual),
                Decision::Deny { .. }
            ),
            "{id} should be denied"
        );
    }
}

/// opengantry's registration hooks are bus-internal: the engine calls them when
/// a worker on the governed listener registers a function or creates an
/// observer, and agents are denied both. gantry::verify and gantry::middleware
/// match no rule and wait for a person.
#[test]
fn gantry_registration_hooks_are_denied() {
    let permissions = repository_permissions();
    for id in [
        "gantry::on-function-registration",
        "gantry::on-observer-registration",
    ] {
        assert!(
            matches!(
                permissions.check(id, &json!({}), PermissionMode::Manual),
                Decision::Deny { .. }
            ),
            "{id} should be denied"
        );
    }
    for id in ["gantry::verify", "gantry::middleware"] {
        assert!(
            matches!(
                permissions.check(id, &json!({}), PermissionMode::Manual),
                Decision::NeedsApproval
            ),
            "{id} should wait for a person"
        );
    }
}
