//! This file owns the `mod e2e;` declaration so Cargo compiles the
//! per-provider env-gated e2e tests under `tests/e2e/` as part of one binary.

mod e2e;

#[test]
fn binary_name_matches_manifest() {
    assert_eq!(storage::worker_name(), "storage");
}

#[test]
fn multipart_wire_schemas_are_closed_and_typed() {
    use storage::handlers::{multipart, schema_of};
    let schema = schema_of::<multipart::PresignReq>();
    assert_eq!(schema["additionalProperties"], false);
    for field in [
        "bucket",
        "key",
        "upload_id",
        "part_number",
        "size",
        "checksum_sha256",
        "expires_in_seconds",
    ] {
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == field));
    }
    assert!(serde_json::from_value::<multipart::Identity>(
        serde_json::json!({"bucket":"b","key":"k","upload_id":"id","extra":true})
    )
    .is_err());
}
