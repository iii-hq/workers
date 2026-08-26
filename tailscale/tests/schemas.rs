mod support;

use tailscale::functions::{catalog, FunctionSpec};

fn golden_file_name(function_id: &str) -> String {
    format!("schemas/{}.json", function_id.replace("::", "."))
}

fn spec_to_pretty_json(spec: &FunctionSpec) -> String {
    let value = serde_json::json!({
        "function_id": spec.function_id,
        "description": spec.description,
        "request_schema": spec.request_schema,
        "response_schema": spec.response_schema,
    });
    let mut pretty = serde_json::to_string_pretty(&value).expect("spec serializes");
    pretty.push('\n');
    pretty
}

#[test]
fn catalog_lists_all_functions_in_registration_order() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(
        ids,
        vec![
            "tailscale::status",
            "tailscale::configuration",
            "tailscale::connect",
            "tailscale::disconnect",
            "tailscale::login",
            "tailscale::logout",
            "tailscale::version",
            "tailscale::ip",
            "tailscale::netcheck",
            "tailscale::ping",
            "tailscale::whois",
            "tailscale::dns::status",
            "tailscale::dns::query",
            "tailscale::peers::list",
            "tailscale::exit-node::list",
            "tailscale::exit-node::suggest",
            "tailscale::exit-node::set",
            "tailscale::prefs::get",
            "tailscale::prefs::set",
            "tailscale::share",
            "tailscale::share::stop",
            "tailscale::serve::list",
            "tailscale::serve::add",
            "tailscale::serve::remove",
            "tailscale::serve::reset",
            "tailscale::file::targets",
            "tailscale::file::send",
            "tailscale::file::receive",
            "tailscale::cert",
            "tailscale::drive::list",
            "tailscale::drive::share",
            "tailscale::drive::unshare",
            "tailscale::lock::status",
            "tailscale::accounts::list",
            "tailscale::accounts::switch",
            "tailscale::update",
            "tailscale::bugreport",
            "tailscale::metrics",
        ]
    );
}

#[test]
fn catalog_ids_are_unique_and_kebab_case() {
    let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    let mut seen = std::collections::HashSet::new();
    for id in &ids {
        assert!(seen.insert(*id), "duplicate function id {id}");
        assert!(id.starts_with("tailscale::"), "{id} is not namespaced");
        assert!(
            !id.contains('_'),
            "{id} uses snake_case; multi-word segments are kebab-case"
        );
    }
}

#[test]
fn wire_schema_snapshots_match_goldens() {
    let mut failures = Vec::new();
    for spec in catalog() {
        let rel = golden_file_name(spec.function_id);
        let actual = spec_to_pretty_json(&spec);
        if let Err(msg) = support::check_golden(&rel, &actual) {
            failures.push(msg);
        }
    }
    assert!(
        failures.is_empty(),
        "{} wire-schema golden(s) drifted:\n\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_function_has_typed_request_and_response_schemas() {
    for spec in catalog() {
        support::assert_typed_schema(
            &format!("{} request_schema", spec.function_id),
            &spec.request_schema,
        );
        support::assert_typed_schema(
            &format!("{} response_schema", spec.function_id),
            &spec.response_schema,
        );
    }
}

#[test]
fn response_schemas_carry_field_descriptions() {
    for spec in catalog() {
        let rendered = serde_json::to_string(&spec.response_schema).expect("schema serializes");
        assert!(
            rendered.contains("description"),
            "{}: response schema lost its field descriptions",
            spec.function_id
        );
    }
}
