/// Integration tests for the `ToolDescriptor` struct and the
/// `validate_tool_namespace` helper introduced in Task 0.2.
///
/// `RegisterSkillInput` is module-private; tests verify the public surface:
///   - `ToolDescriptor` serialises / deserialises correctly.
///   - `validate_tool_namespace` accepts properly-namespaced tools.
///   - `validate_tool_namespace` rejects tools under a different namespace.

use iii_skills::{validate_tool_namespace, ToolDescriptor};

#[test]
fn register_with_two_tools_round_trips() {
    // Deserialise two tool descriptors from raw JSON — the same shape a
    // caller passes inside the `tools` array of `skills::register`.
    let raw = serde_json::json!([
        { "name": "foo::do_thing", "description": "x", "input_schema": {"type": "object"} },
        { "name": "foo::other",    "description": "y", "input_schema": {"type": "object"}, "max_bytes": 1024 }
    ]);
    let tools: Vec<ToolDescriptor> = serde_json::from_value(raw).unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].name, "foo::do_thing");
    assert_eq!(tools[1].max_bytes, Some(1024));
    // First tool has no max_bytes — should default to None.
    assert_eq!(tools[0].max_bytes, None);
}

#[test]
fn validate_rejects_unnamespaced_tool() {
    let tools = vec![ToolDescriptor {
        name: "bar::do_thing".into(),
        description: "x".into(),
        input_schema: serde_json::json!({}),
        max_bytes: None,
    }];
    // Worker "foo" cannot register a tool namespaced under "bar".
    assert!(validate_tool_namespace("foo", &tools).is_err());
}

#[test]
fn validate_accepts_correctly_namespaced_tools() {
    let tools = vec![
        ToolDescriptor {
            name: "myworker::send".into(),
            description: "Send something".into(),
            input_schema: serde_json::json!({ "type": "object" }),
            max_bytes: None,
        },
        ToolDescriptor {
            name: "myworker::receive".into(),
            description: "Receive something".into(),
            input_schema: serde_json::json!({ "type": "object" }),
            max_bytes: Some(512),
        },
    ];
    assert!(validate_tool_namespace("myworker", &tools).is_ok());
}

#[test]
fn validate_accepts_empty_tools_list() {
    // Empty slice is always valid — most workers don't export LLM tools.
    assert!(validate_tool_namespace("anything", &[]).is_ok());
}

#[test]
fn tool_descriptor_max_bytes_defaults_to_none_when_absent() {
    let raw = serde_json::json!({
        "name": "w::fn",
        "description": "d",
        "input_schema": {}
    });
    let t: ToolDescriptor = serde_json::from_value(raw).unwrap();
    assert_eq!(t.max_bytes, None);
}
