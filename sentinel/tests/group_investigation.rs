//! Contract-first tests for joint investigations. These must fail before the
//! new route exists; the ordinary investigation contract stays untouched.
use sentinel::functions::{INVESTIGATION_ALLOW, INVESTIGATION_DENY, catalog};
use serde_json::{Value, json};

fn joint_schema(request: bool) -> Value {
    let spec = catalog()
        .into_iter()
        .find(|spec| spec.function_id == "sentinel::investigate_many")
        .expect("joint investigations must expose a typed backend route");
    serde_json::to_value(if request {
        spec.request_schema
    } else {
        spec.response_schema
    })
    .expect("published schema serializes")
}

#[test]
fn joint_request_requires_selection_profile_and_idempotency_without_routing_overrides() {
    let schema = joint_schema(true);
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["required"],
        json!(["group_ids", "profile_id", "request_id"])
    );
    for field in [
        "model",
        "provider",
        "options",
        "session_id",
        "functions",
        "filesystem_root",
    ] {
        assert!(
            schema["properties"].get(field).is_none(),
            "caller must not control {field}"
        );
    }
    assert_eq!(schema["properties"]["hypothesis"]["maxLength"], 2000);
}

#[test]
fn joint_request_publishes_conservative_selection_limits() {
    let schema = joint_schema(true);
    assert_eq!(schema["properties"]["group_ids"]["minItems"], 2);
    assert_eq!(schema["properties"]["group_ids"]["maxItems"], 5);
    assert_eq!(schema["properties"]["request_id"]["minLength"], 1);
    assert_eq!(schema["properties"]["request_id"]["maxLength"], 128);
}

#[test]
fn joint_response_identifies_the_single_session_and_all_authorized_members() {
    let schema = joint_schema(false);
    let required = schema["required"].as_array().expect("typed response");
    for field in [
        "existing",
        "group_ids",
        "investigation_id",
        "profile_id",
        "session_id",
    ] {
        assert!(
            required.contains(&json!(field)),
            "missing response field {field}"
        );
    }
    assert!(schema["properties"].get("first_pass_turn_id").is_some());
}

#[test]
fn an_investigation_cannot_start_a_joint_investigation() {
    let id = "sentinel::investigate_many";
    assert!(!INVESTIGATION_ALLOW.contains(&id));
    assert!(
        INVESTIGATION_DENY.iter().any(|pattern| {
            pattern
                .strip_suffix('*')
                .map_or(*pattern == id, |prefix| id.starts_with(prefix))
        }),
        "the new entry point must be explicitly denied to investigation sessions"
    );
}
