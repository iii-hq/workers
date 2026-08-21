mod support;

use provider_command_code::surface::{catalog, FunctionSpec};

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
    let ids: Vec<&str> = catalog().iter().map(|spec| spec.function_id).collect();
    assert_eq!(
        ids,
        vec![
            "provider::command-code::stream",
            "provider::command-code::abort",
            "provider::command-code::refresh_models",
            "provider::command-code::on_router_ready",
        ]
    );
}

#[test]
fn wire_schema_snapshots_match_goldens() {
    let mut failures = Vec::new();
    for spec in catalog() {
        let relative_path = golden_file_name(spec.function_id);
        let actual = spec_to_pretty_json(&spec);
        if let Err(message) = support::check_golden(&relative_path, &actual) {
            failures.push(message);
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
fn no_orphan_schema_goldens() {
    let directory = support::golden_root().join("schemas");
    let expected: Vec<String> = catalog()
        .iter()
        .map(|spec| {
            std::path::Path::new(&golden_file_name(spec.function_id))
                .file_name()
                .expect("golden file name")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let entries = std::fs::read_dir(&directory).expect("read schema golden directory");
    for entry in entries {
        let entry = entry.expect("read schema golden entry");
        if !entry
            .file_type()
            .expect("read schema golden type")
            .is_file()
            || entry.path().extension() != Some(std::ffi::OsStr::new("json"))
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(
            expected.iter().any(|expected| expected == &name),
            "orphan golden tests/golden/schemas/{name}: no catalog entry produces it"
        );
    }
}
