mod support;

use sentinel::functions::catalog;

fn golden_file_name(function_id: &str) -> String {
    format!("schemas/{}.json", function_id.replace("::", "."))
}

#[test]
fn catalog_matches_the_registered_surface() {
    let ids: Vec<_> = catalog().iter().map(|spec| spec.function_id).collect();
    assert_eq!(
        ids,
        [
            "sentinel::status",
            "sentinel::groups::list",
            "sentinel::groups::get",
            "sentinel::groups::resolve",
            "sentinel::groups::ignore",
            "sentinel::groups::unignore",
            "sentinel::groups::reopen",
            "sentinel::occurrences::list",
            "sentinel::evidence::get",
            "sentinel::groups::history",
            "sentinel::diagnoses::list",
            "sentinel::investigate",
            "sentinel::investigate_many",
            "sentinel::source::read",
            "sentinel::investigations::get",
            "sentinel::investigations::list",
            "sentinel::investigations::cancel",
            "sentinel::diagnosis::record",
            "sentinel::trace::get",
            "sentinel::logs::list",
            "sentinel::on-trace-activity",
            "sentinel::on-log",
            "sentinel::ingest",
            "sentinel::on-turn-completed",
            "sentinel::on-schedule",
            "sentinel::on-config-change",
        ]
    );
}

#[test]
fn schemas_are_typed_and_match_goldens() {
    let mut failures = Vec::new();
    for spec in catalog() {
        support::assert_typed_schema(
            &format!("{} request", spec.function_id),
            &spec.request_schema,
        );
        support::assert_typed_schema(
            &format!("{} response", spec.function_id),
            &spec.response_schema,
        );
        let value = serde_json::json!({
            "function_id": spec.function_id,
            "description": spec.description,
            "request_schema": spec.request_schema,
            "response_schema": spec.response_schema,
        });
        let mut actual = serde_json::to_string_pretty(&value).expect("schema serializes");
        actual.push('\n');
        if let Err(error) = support::check_golden(&golden_file_name(spec.function_id), &actual) {
            failures.push(error);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn create_only_missing_joint_goldens_when_explicitly_requested() {
    if std::env::var("SENTINEL_CREATE_NEW_GOLDENS").as_deref() != Ok("1") {
        return;
    }
    use std::io::Write;
    for spec in catalog().into_iter().filter(|s| {
        matches!(
            s.function_id,
            "sentinel::investigate_many" | "sentinel::source::read"
        )
    }) {
        let value = serde_json::json!({"function_id":spec.function_id,"description":spec.description,"request_schema":spec.request_schema,"response_schema":spec.response_schema});
        let actual = format!("{}\n", serde_json::to_string_pretty(&value).unwrap());
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(golden_file_name(spec.function_id));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => file.write_all(actual.as_bytes()).unwrap(),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => assert_eq!(
                std::fs::read_to_string(path).unwrap(),
                actual,
                "existing golden must never be modified"
            ),
            Err(error) => panic!("create new golden: {error}"),
        }
    }
}
