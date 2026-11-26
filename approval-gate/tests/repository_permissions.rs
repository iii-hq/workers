use approval_gate::permissions::{Decision, Permissions, parse_rules_from_config};
use approval_gate::types::PermissionMode;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct RepositoryPermissions {
    rules: Vec<Value>,
}

fn repository_permissions() -> Permissions {
    permissions_from_yaml(include_str!("../../iii-permissions.yaml"))
}

fn permissions_from_yaml(source: &str) -> Permissions {
    let config: RepositoryPermissions =
        serde_yaml_ng::from_str(source).expect("repository permissions should be valid YAML");
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
fn secret_values_and_writes_are_denied_in_root_and_worker_permissions() {
    for permissions in [
        repository_permissions(),
        permissions_from_yaml(include_str!("../../secrets/iii-permissions.yaml")),
    ] {
        for id in [
            "secrets::resolve",
            "secrets::set",
            "secrets::compare-and-set",
            "secrets::delete",
            "secrets::access",
            "secrets::import",
            "secrets::on-config-change",
            "secrets::on-changed",
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

/// iii-cloud (owner decision, 26/11): the agent may do anything but deploy,
/// approve, merge, roll back or write to iii cloud. Those are the page's.
#[test]
fn iii_cloud_deploy_and_writes_are_person_only_in_root_and_worker_permissions() {
    for permissions in [
        repository_permissions(),
        permissions_from_yaml(include_str!("../../iii-cloud/iii-permissions.yaml")),
    ] {
        for id in [
            "iii-cloud::deploy",
            "iii-cloud::api::write",
            "iii-cloud::secret::provide",
            "iii-cloud::values::save",
            "iii-cloud::values::skip",
            "iii-cloud::problem::skip",
            "iii-cloud::config::import",
            "iii-cloud::override::choose",
        ] {
            assert!(
                matches!(
                    permissions.check(id, &json!({}), PermissionMode::Manual),
                    Decision::Deny { .. }
                ),
                "{id} should be denied"
            );
        }
        for id in [
            "iii-cloud::api",
            "iii-cloud::check",
            "iii-cloud::check::get",
            "iii-cloud::handoff::worktree",
            "iii-cloud::values",
            "iii-cloud::watch",
            "iii-cloud::status",
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
}

/// The rules as written, for a call in `namespace`.
fn in_ns(permissions: &Permissions, namespace: &str, id: &str) -> Decision {
    permissions.check_in(Some(namespace), id, &json!({}), PermissionMode::Manual)
}

/// What the gate answers for a call in `namespace` from an agent in `project`.
fn gate(permissions: &Permissions, namespace: &str, id: &str) -> Decision {
    permissions.check_call(
        Some(namespace),
        Some("project"),
        id,
        &json!({}),
        PermissionMode::Manual,
    )
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
            "mesh::on-observer-registration",
            "mesh::on-node-changed",
            "mesh::on-config-change",
            "mesh::ui-content",
            "mesh::configuration-id",
            "mesh-node::status",
            "mesh-node::update",
            "mesh-node::workers::apply",
            "mesh-node::drives::apply",
        ] {
            assert!(
                matches!(
                    permissions.check(id, &json!({}), PermissionMode::Manual),
                    Decision::Deny { .. }
                ),
                "{id} should be denied"
            );
            for namespace in ["laptop", "hub"] {
                assert!(
                    matches!(in_ns(&permissions, namespace, id), Decision::Deny { .. }),
                    "{id} should be denied in {namespace}"
                );
            }
        }
    }
}

/// The harness ships these rules and can run in front of a mesh that still uses prefixes.
#[test]
fn old_mesh_ids_stay_denied_by_the_root_rules() {
    let permissions = repository_permissions();
    for id in [
        "laptop::mesh-node::update",
        "tenant::laptop::mesh-node::update",
        "laptop::shell::pty::open",
        "laptop::shell::pty::sessions",
        "laptop::shell::scm::commit-message",
        "laptop::shell::ui-state::get",
        "laptop::shell::config-status",
        "laptop::shell::kill-session-jobs",
        "laptop::shell::on-config-change",
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

#[test]
fn remote_shell_internals_are_denied_in_every_namespace() {
    for permissions in [
        repository_permissions(),
        permissions_from_yaml(include_str!("../../mesh/iii-permissions.yaml")),
    ] {
        for id in [
            "shell::pty::open",
            "shell::scm::commit-message",
            "shell::ui-state::get",
            "shell::config-status",
            "shell::kill-session-jobs",
            "shell::on-config-change",
        ] {
            // The rules as written: they must hold without the old spelling.
            for namespace in ["laptop", "hub"] {
                assert!(
                    matches!(in_ns(&permissions, namespace, id), Decision::Deny { .. }),
                    "{id} should be denied in {namespace}"
                );
            }
        }
    }
}

#[test]
fn remote_tools_and_mesh_management_need_approval() {
    for permissions in [
        repository_permissions(),
        permissions_from_yaml(include_str!("../../mesh/iii-permissions.yaml")),
    ] {
        for namespace in ["laptop", "hub"] {
            assert!(
                matches!(
                    gate(&permissions, namespace, "shell::exec"),
                    Decision::NeedsApproval
                ),
                "shell::exec in {namespace} should need approval"
            );
        }
        for id in [
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

/// A bare rule matches every namespace. The mesh release notes say so.
/// `shell::pty::sessions` stays denied on another machine while the
/// `*::shell::pty::*` line is kept: it matches the call's old spelling.
#[test]
fn read_only_allows_cover_every_namespace() {
    let permissions = repository_permissions();
    for id in ["coder::read-file", "coder::search", "coder::tree"] {
        assert!(
            matches!(gate(&permissions, "laptop", id), Decision::Allow { .. }),
            "{id} should be allowed in laptop"
        );
    }
    assert!(matches!(
        gate(&permissions, "laptop", "shell::pty::sessions"),
        Decision::Deny { .. }
    ));
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

/// The MCP client page writes one allow rule per read function: the exact
/// function id, no modes, and a rule id that starts with `mcp-client:`. A
/// function with no rule is held. This pins the syntax the page depends on.
#[test]
fn page_written_rules_compile_allow_reads_and_hold_writes() {
    let rules = json!([
        "!approval::*",
        { "function": "mcp::linear::list_issues", "action": "allow", "rule_id": "mcp-client:mcp::linear::list_issues" },
        { "function": "mcp::linear::get_issue", "action": "allow", "rule_id": "mcp-client:mcp::linear::get_issue" }
    ]);
    let specs = parse_rules_from_config(&rules);
    assert_eq!(specs.len(), 3, "the parser must keep every rule");
    let permissions = Permissions::compile(&specs).expect("page-written rules should compile");
    for mode in [PermissionMode::Manual, PermissionMode::Auto] {
        for id in ["mcp::linear::list_issues", "mcp::linear::get_issue"] {
            assert_eq!(
                permissions.check(id, &json!({}), mode),
                Decision::Allow {
                    rule_id: format!("mcp-client:{id}")
                },
                "{id} should be allowed in {mode:?}"
            );
        }
        assert_eq!(
            permissions.check("mcp::linear::save_issue", &json!({}), mode),
            Decision::NeedsApproval,
            "a write should wait for a person in {mode:?}"
        );
    }
}
