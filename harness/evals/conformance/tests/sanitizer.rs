//! Cassette sanitizer fixtures (spec § Verification: credentials, cookies,
//! PII-ish keys, unstable ids must be rejected by the denylist scan) plus
//! the digest rule.

use harness_conformance::types::cassette::{denylist_scan, RouterCassetteV1};
use serde_json::json;

/// A structurally complete cassette — including the mandatory
/// `max_output_tokens` model field the script schema requires.
fn full_cassette_json() -> serde_json::Value {
    json!({
        "schema_version": "1",
        "captured_at": "2026-07-15T00:00:00Z",
        "engine_revision": "085e0fde6b424092a8b7e3ab31ac5e0cd36fa2e0",
        "harness_revision": "abc",
        "router_revision": "def",
        "provider": "scripted",
        "model": "fixture-model",
        "script": {
            "schema_version": "1",
            "scenario_id": "C-E2E-001",
            "model": {
                "id": "fixture-model", "provider": "scripted",
                "context_window": 32768, "max_output_tokens": 4096
            },
            "generations": []
        },
        "sanitized_sha256": ""
    })
}

/// A complete, valid cassette must pass the scan: in particular the
/// mandatory `max_output_tokens` fields must not read as credentials.
#[test]
fn a_complete_valid_cassette_passes_the_scan() {
    let cassette = full_cassette_json();
    let _typed: RouterCassetteV1 = serde_json::from_value(cassette.clone()).unwrap();
    assert!(
        denylist_scan(&cassette).is_empty(),
        "{:?}",
        denylist_scan(&cassette)
    );
}

#[test]
fn credentials_cookies_and_secret_keys_are_rejected() {
    for (name, value) in [
        (
            "authorization header",
            json!({ "headers": { "Authorization": "x" } }),
        ),
        ("api key", json!({ "api_key": "value" })),
        ("cookie", json!({ "Set-Cookie": "sid=1" })),
        ("password", json!({ "password": "hunter2" })),
        ("credential", json!({ "aws_credentials": {} })),
        ("bare token key", json!({ "token": "value" })),
        ("access token", json!({ "access_token": "value" })),
        ("hyphenated token key", json!({ "Refresh-Token": "value" })),
        ("bearer value", json!({ "note": "Bearer abc123" })),
        ("sk- value", json!({ "text": "sk-ant-abc123" })),
        ("github token", json!({ "text": "ghp_abc123" })),
        (
            "private metadata",
            json!({ "provider_private": { "x": 1 } }),
        ),
    ] {
        assert!(
            !denylist_scan(&value).is_empty(),
            "{name} must be rejected: {value}"
        );
    }
}

#[test]
fn digest_covers_canonical_json_without_the_digest_field() {
    let mut cassette: RouterCassetteV1 = serde_json::from_value(full_cassette_json()).unwrap();
    cassette.sanitized_sha256 = cassette.digest().unwrap();
    cassette.verify_digest().unwrap();
    cassette.provider = "tampered".into();
    assert!(cassette.verify_digest().is_err());
}
