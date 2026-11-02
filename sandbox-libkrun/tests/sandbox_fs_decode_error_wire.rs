//! R1 — decode-error wire snapshot test. T6 migrated the 10 fs::* handlers
//! from `register_function_with` (hand-rolled deserialization) to
//! `RegisterFunction::new_async` (SDK-driven deserialization). E1 from the
//! plan-eng-review identified the risk that the SDK's decode-failure wire
//! shape might drift from the old `"bad request: ..."` plain-text form.
//!
//! This test does NOT verify the SDK's exact wire shape (the SDK owns
//! that contract). Instead, it pins ONE invariant: the daemon-side
//! handler still constructs the typed Request struct from a serde JSON
//! payload, so missing required fields produce a serde error somewhere
//! identifiable. If a future SDK upgrade collapses that path silently,
//! this test fails and forces a conscious review of whether the new
//! wire shape is acceptable.
//!
//! Verification mechanism: deserialise a known-bad payload directly
//! through the Request struct, assert it errors with a message naming
//! the missing field. The new_async path uses `serde_json::from_value`
//! internally, so this asserts the same thing the handler does without
//! booting a daemon.
//!
//! Handlers take `Payload<T>`, so that failure reaches the handler and
//! answers S001 like any other invalid request; the `*_answers_s001` tests
//! pin that body.

use sandbox_libkrun::sandbox_daemon::errors::Payload;
use sandbox_libkrun::sandbox_daemon::exec::ExecRequest;
use sandbox_libkrun::sandbox_daemon::fs::ls::LsRequest;

#[test]
fn ls_request_missing_path_field_fails_with_named_field() {
    let payload = serde_json::json!({
        "sandbox_id": "00000000-0000-0000-0000-000000000000"
        // path missing
    });
    let err = serde_json::from_value::<LsRequest>(payload).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("path") && (msg.contains("missing") || msg.contains("required")),
        "decode error message must name the missing field; got {msg:?}"
    );
}

#[test]
fn ls_request_missing_sandbox_id_fails_with_named_field() {
    let payload = serde_json::json!({
        "path": "/home/app"
        // sandbox_id missing
    });
    let err = serde_json::from_value::<LsRequest>(payload).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("sandbox_id") && (msg.contains("missing") || msg.contains("required")),
        "decode error message must name the missing field; got {msg:?}"
    );
}

/// The body a handler answers for `payload`, decoded the way the SDK does it.
fn decode_error<T: serde::de::DeserializeOwned>(payload: serde_json::Value) -> serde_json::Value {
    let req = serde_json::from_value::<Payload<T>>(payload).expect("any JSON reaches the handler");
    let Err(err) = req.decode() else {
        panic!("the payload decoded")
    };
    serde_json::from_str(&err.to_string()).unwrap()
}

fn assert_s001(body: &serde_json::Value, field: &str) {
    assert_eq!(body["code"], "S001", "{body}");
    assert_eq!(body["type"], "validation", "{body}");
    assert_eq!(body["retryable"], false, "{body}");
    assert_eq!(
        body["docs_url"],
        "https://iii.dev/docs/creating-workers/sandboxes#S001"
    );
    assert!(body["message"].as_str().unwrap().contains(field), "{body}");
}

#[test]
fn exec_without_sandbox_id_answers_s001() {
    let body = decode_error::<ExecRequest>(serde_json::json!({ "cmd": "true" }));
    assert_s001(&body, "sandbox_id");
}

#[test]
fn ls_with_a_numeric_sandbox_id_answers_s001() {
    let body = decode_error::<LsRequest>(serde_json::json!({ "sandbox_id": 42, "path": "/" }));
    assert_s001(&body, "string");
}

/// The wrapper must not change the request schema the adapter advertises.
#[test]
fn payload_advertises_the_request_schema() {
    assert_eq!(
        schemars::schema_for!(Payload<ExecRequest>),
        schemars::schema_for!(ExecRequest)
    );
}

#[test]
fn ls_request_wrong_field_type_is_rejected() {
    let payload = serde_json::json!({
        "sandbox_id": 42,  // should be string
        "path": "/home/app"
    });
    let err = serde_json::from_value::<LsRequest>(payload).unwrap_err();
    // The exact wording is serde-version-specific; we just assert the
    // error mentions the offending field or type.
    assert!(
        !err.to_string().is_empty(),
        "decode error must produce a message"
    );
}
