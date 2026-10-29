//! Catalog-level checks: the function ids the permissions files reference
//! exist, and the configuration schema accepts its own defaults.

use iii_stories::config::{StoriesConfig, normalize, schema};
use iii_stories::events::{
    BUILD_TRIGGER, BuildEvent, CHANGED_TRIGGER, ChangedEvent, ON_BUILD_FN, ON_CHANGED_FN,
    refuse_build, refuse_changed,
};
use iii_stories::functions::FUNCTION_IDS;
use iii_stories::model::{ChangeSummary, LineInfo, LineKind};

#[test]
fn permissions_file_names_every_public_function() {
    let permissions =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/iii-permissions.yaml"))
            .unwrap();
    for id in FUNCTION_IDS {
        assert!(
            permissions.contains(id),
            "{id} missing from iii-permissions.yaml"
        );
    }
    for internal in [
        "!stories::ui-content",
        "!stories::ui-file",
        "!stories::on-config-change",
        "!stories::on-changed",
        "!stories::on-build",
    ] {
        assert!(
            permissions.contains(internal),
            "{internal} missing from iii-permissions.yaml"
        );
    }
}

#[test]
fn observable_functions_sit_next_to_their_trigger_types_and_are_not_invocable() {
    assert_eq!(CHANGED_TRIGGER, "stories:changed");
    assert_eq!(BUILD_TRIGGER, "stories:build");
    assert_eq!(ON_CHANGED_FN, "stories::on-changed");
    assert_eq!(ON_BUILD_FN, "stories::on-build");
    for id in [ON_CHANGED_FN, ON_BUILD_FN] {
        assert!(
            !FUNCTION_IDS.contains(&id),
            "{id} is observed, never called: it is not a public function"
        );
    }
    let readme =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).unwrap();
    let manifest =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/iii.worker.yaml")).unwrap();
    for id in [ON_CHANGED_FN, ON_BUILD_FN] {
        assert!(readme.contains(id), "{id} missing from README.md");
        assert!(manifest.contains(id), "{id} missing from iii.worker.yaml");
    }
}

fn changed_event() -> ChangedEvent {
    let line = LineInfo {
        key: "worktree".into(),
        kind: LineKind::Worktree,
        label: "working tree".into(),
        sha: None,
        dirty: None,
        built_at: "t".into(),
    };
    ChangedEvent {
        workspace: "ws".into(),
        line: line.clone(),
        base: line,
        summary: ChangeSummary::default(),
        changes: vec![],
        at: "t".into(),
    }
}

fn build_event() -> BuildEvent {
    BuildEvent {
        workspace: "ws".into(),
        build_id: "b_1".into(),
        status: "done".into(),
        line: "worktree".into(),
        message: None,
        at: "t".into(),
    }
}

#[test]
fn each_on_function_refuses_direct_invocation() {
    for (id, refusal) in [
        (ON_CHANGED_FN, refuse_changed(changed_event())),
        (ON_BUILD_FN, refuse_build(build_event())),
    ] {
        match refusal {
            Err(iii_sdk::errors::Error::Remote { code, message, .. }) => {
                assert_eq!(code, iii_sdk::protocol::observe_codes::FORBIDDEN);
                assert!(message.contains(id), "{message}");
            }
            other => panic!("{id}: expected FORBIDDEN, got {other:?}"),
        }
    }
}

/// The schemas the SDK publishes for a typed sync handler `fn(T) -> Result<R, Error>`
/// (same generator settings as `iii_sdk::RegisterFunction::new`).
fn handler_schemas<T: schemars::JsonSchema, R: schemars::JsonSchema>(
    _handler: fn(T) -> Result<R, iii_sdk::errors::Error>,
) -> (serde_json::Value, serde_json::Value) {
    let schema_for = |root| serde_json::to_value(root).unwrap();
    let generator = || schemars::r#gen::SchemaSettings::draft07().into_generator();
    (
        schema_for(generator().into_root_schema_for::<T>()),
        schema_for(generator().into_root_schema_for::<R>()),
    )
}

/// Mirrors `.github/scripts/collect_worker_interface.py`: an AnyValue or
/// empty schema carries none of these keywords.
fn assert_typed_schema(function_id: &str, which: &str, schema: &serde_json::Value) {
    const KEYS: [&str; 7] = [
        "type",
        "properties",
        "$ref",
        "allOf",
        "anyOf",
        "oneOf",
        "enum",
    ];
    assert!(
        schema
            .as_object()
            .is_some_and(|o| KEYS.iter().any(|k| o.contains_key(*k))),
        "{function_id} {which} schema is untyped (AnyValue/empty): {schema}"
    );
}

#[test]
fn observable_functions_have_typed_request_and_response_schemas() {
    for (function_id, (request, response), required) in [
        (
            ON_CHANGED_FN,
            handler_schemas(refuse_changed),
            &["workspace", "line", "base", "summary", "changes", "at"][..],
        ),
        (
            ON_BUILD_FN,
            handler_schemas(refuse_build),
            &["workspace", "build_id", "status", "line", "at"][..],
        ),
    ] {
        assert_typed_schema(function_id, "request", &request);
        assert_typed_schema(function_id, "response", &response);
        assert_eq!(request["type"], "object", "{function_id}: {request}");
        for field in required {
            assert!(
                request["required"]
                    .as_array()
                    .is_some_and(|r| r.contains(&(*field).into())),
                "{function_id} request must document the delivered event ({field}): {request}"
            );
        }
    }
}

#[test]
fn defaults_satisfy_the_schema_shape() {
    let defaults = StoriesConfig::default();
    let schema = schema();
    let properties = schema["properties"].as_object().unwrap();
    let json = defaults.to_json();
    for key in json.as_object().unwrap().keys() {
        assert!(
            properties.contains_key(key),
            "default field {key} is not in the schema"
        );
    }
    assert_eq!(normalize(&json), defaults);
}

#[test]
fn worker_manifest_declares_the_binary_and_dependencies() {
    let manifest =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/iii.worker.yaml")).unwrap();
    assert!(manifest.contains("name: stories"));
    assert!(manifest.contains("bin: stories"));
    assert!(manifest.contains("browser:"));
    assert!(manifest.contains("configuration:"));
}
