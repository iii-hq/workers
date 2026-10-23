mod support;

use tmux::functions::{catalog, FunctionSpec};

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
            "tmux::status",
            "tmux::sessions::list",
            "tmux::sessions::create",
            "tmux::sessions::rename",
            "tmux::sessions::kill",
            "tmux::windows::create",
            "tmux::windows::select",
            "tmux::windows::rename",
            "tmux::windows::kill",
            "tmux::panes::select",
            "tmux::panes::capture",
            "tmux::subagents::transcript",
            "tmux::ports::list",
        ]
    );
}

#[test]
fn catalog_ids_are_unique_and_kebab_case() {
    let mut seen = std::collections::HashSet::new();
    for spec in catalog() {
        let id = spec.function_id;
        assert!(seen.insert(id), "duplicate function id {id}");
        assert!(id.starts_with("tmux::"), "{id} is not namespaced");
        assert!(!id.contains('_'), "{id} uses snake_case");
    }
}

#[test]
fn wire_schema_snapshots_match_goldens() {
    let mut failures = Vec::new();
    for spec in catalog() {
        let rel = golden_file_name(spec.function_id);
        if let Err(msg) = support::check_golden(&rel, &spec_to_pretty_json(&spec)) {
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

#[test]
fn terminal_functions_stay_internal() {
    let public: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
    for (id, _) in tmux::terminal::FUNCTIONS {
        assert!(id.starts_with("tmux::terminal::"));
        assert!(
            !public.contains(&id),
            "{id} must not be in the public catalog"
        );
    }
}
