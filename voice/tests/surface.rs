//! The wire surface, pinned: function ids, their order, and the manifest the
//! publish pipeline reads. A change here is a change callers see.

use voice::config::WorkerConfig;
use voice::functions;
use voice::manifest;

#[test]
fn the_catalog_lists_every_public_function_in_order() {
    let ids: Vec<&str> = functions::catalog().iter().map(|s| s.function_id).collect();
    assert_eq!(
        ids,
        [
            "voice::dictation::start",
            "voice::dictation::push",
            "voice::dictation::stop",
            "voice::dictation::list",
            "voice::transcribe",
            "voice::speak",
            "voice::speak::stop",
            "voice::models::list",
            "voice::models::download",
            "voice::models::remove",
            "voice::doctor",
        ]
    );
}

#[test]
fn request_schemas_name_their_required_fields() {
    let by_id: std::collections::HashMap<&str, _> = functions::catalog()
        .into_iter()
        .map(|s| (s.function_id, s))
        .collect();
    let push = &by_id["voice::dictation::push"];
    let required = push
        .request_schema
        .schema
        .object
        .as_ref()
        .unwrap()
        .required
        .clone();
    assert!(required.contains("session_id"));
    assert!(required.contains("seq"));
    assert!(required.contains("pcm16_base64"));
    let start = &by_id["voice::dictation::start"];
    let required = start
        .request_schema
        .schema
        .object
        .as_ref()
        .unwrap()
        .required
        .clone();
    assert!(required.contains("output_function_id"));
}

#[test]
fn the_manifest_default_config_round_trips() {
    let manifest = manifest::build_manifest();
    let parsed = WorkerConfig::from_json(&manifest.default_config).expect("default config parses");
    assert_eq!(parsed, WorkerConfig::default());
}
