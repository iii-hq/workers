//! The wire surface, pinned: function ids, their order, and the manifest the
//! publish pipeline reads. A change here is a change callers see.

use voice::config::WorkerConfig;
use voice::events::{refuse_invocation, EventKind};
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
            "voice::speech::prepare",
            "voice::speak::stop",
            "voice::models::list",
            "voice::models::download",
            "voice::models::remove",
            "voice::doctor",
        ]
    );
}

#[test]
fn every_trigger_type_has_an_observable_function_outside_the_call_catalog() {
    let callable: Vec<&str> = functions::catalog().iter().map(|s| s.function_id).collect();
    let observable: Vec<&str> = EventKind::all()
        .iter()
        .map(|kind| kind.observable_function())
        .collect();
    assert_eq!(
        observable,
        [
            "voice::on-transcript",
            "voice::on-session-started",
            "voice::on-session-stopped",
            "voice::on-model-progress",
            "voice::on-speech-ended",
        ]
    );
    let readme =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).unwrap();
    for kind in EventKind::all() {
        let id = kind.observable_function();
        assert!(!callable.contains(&id), "{id} is observed, never called");
        assert!(readme.contains(id), "{id} missing from README.md");
        match refuse_invocation(id) {
            iii_sdk::errors::Error::Remote { code, .. } => assert_eq!(code, "FORBIDDEN"),
            other => panic!("{id}: expected FORBIDDEN, got {other:?}"),
        }
    }
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
fn speech_text_format_defaults_to_markdown_but_supports_literal_text() {
    use voice::speech_text::{prepare, TextFormat};
    let req: functions::speak::Request =
        serde_json::from_value(serde_json::json!({"text": "**Olá**"})).unwrap();
    assert_eq!(prepare(&req.text, req.text_format, 100).unwrap(), "Olá");
    let req: functions::speak::Request =
        serde_json::from_value(serde_json::json!({"text": "**literal**", "text_format": "plain"}))
            .unwrap();
    assert!(matches!(req.text_format, TextFormat::Plain));
    assert_eq!(
        prepare(&req.text, req.text_format, 100).unwrap(),
        "**literal**"
    );
    assert!(serde_json::from_value::<functions::speak::Request>(
        serde_json::json!({"text": "Hello", "text_format": "html"})
    )
    .is_err());
}

#[test]
fn the_manifest_default_config_round_trips() {
    let manifest = manifest::build_manifest();
    let parsed = WorkerConfig::from_json(&manifest.default_config).expect("default config parses");
    assert_eq!(parsed, WorkerConfig::default());
}
