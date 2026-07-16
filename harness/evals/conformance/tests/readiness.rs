//! Readiness failure vectors (spec § Verification: hidden internal
//! functions, missing context manager, wrong queue topic, schema/seed
//! mismatch) exercised over fake catalogs — the pure checks are the same
//! code the live probe runs.

use harness_conformance::readiness::{
    config_failure, missing_functions, missing_trigger_types, topic_failures, ReadinessSpec,
};
use serde_json::json;

fn spec() -> ReadinessSpec {
    ReadinessSpec::harness_surface(vec![(
        "harness".to_string(),
        json!({ "default_filesystem_root": "off" }),
    )])
}

#[test]
fn internal_functions_hidden_from_a_filtered_catalog_are_reported() {
    // A default (filtered) catalog omits internal ids like harness::send;
    // the probe must name the missing surface rather than pass vacuously.
    let filtered = json!({ "functions": [ { "function_id": "harness::status" } ] });
    let missing = missing_functions(&spec(), &filtered);
    assert_eq!(missing, vec!["function harness::send"]);
}

#[test]
fn missing_context_manager_names_both_functions() {
    let spec = ReadinessSpec::pre_harness(vec![]);
    let listed = json!({ "functions": [
        { "function_id": "session::messages" },
        { "function_id": "router::chat" },
        { "function_id": "router::abort" },
        { "function_id": "router::models::list" },
        { "function_id": "router::models::get" },
        { "function_id": "router::models::supports" },
        { "function_id": "router::system_prompt::get" },
        { "function_id": "conformance-recorder::configure" },
        { "function_id": "conformance-recorder::reset" },
        { "function_id": "conformance-recorder::snapshot" },
        { "function_id": "conformance-recorder::await" },
        { "function_id": "conformance-recorder::lifecycle" },
        { "function_id": "engine::queue::list_topics" }
    ]});
    let missing = missing_functions(&spec, &listed);
    assert_eq!(
        missing,
        vec![
            "function context::assemble",
            "function context::count-tokens"
        ]
    );
}

#[test]
fn wrong_or_absent_queue_topic_is_reported_with_broker_detail() {
    let s = spec();
    let empty = topic_failures(&s, &json!([]));
    assert_eq!(empty, vec!["queue topic harness-turn"]);

    let wrong_broker = topic_failures(
        &s,
        &json!([{ "name": "harness-turn", "broker_type": "rabbitmq", "subscriber_count": 1 }]),
    );
    assert_eq!(
        wrong_broker,
        vec!["queue topic harness-turn broker type: expected builtin, got rabbitmq"]
    );

    let ok = topic_failures(
        &s,
        &json!([{ "name": "harness-turn", "broker_type": "builtin", "subscriber_count": 1 }]),
    );
    assert!(ok.is_empty());
}

#[test]
fn missing_trigger_type_is_reported() {
    let missing = missing_trigger_types(&spec(), &json!([{ "id": "harness::turn-started" }]));
    assert_eq!(missing, vec!["trigger type harness::turn-completed"]);
}

#[test]
fn seed_mismatch_is_reported_but_resolved_defaults_are_tolerated() {
    let expected = json!({ "default_filesystem_root": "off" });
    // Worker stored the seed merged with its defaults: seeded key wins → ok.
    let resolved = json!({ "value": { "default_filesystem_root": "off", "max_turns": 500 } });
    assert!(config_failure("harness", &expected, &resolved).is_none());

    // Stored value contradicts the seed → named failure.
    let overridden = json!({ "value": { "default_filesystem_root": "/somewhere" } });
    let failure = config_failure("harness", &expected, &overridden).expect("must fail");
    assert!(failure.contains("configuration harness"), "{failure}");

    // No stored value at all.
    assert!(config_failure("harness", &expected, &json!({})).is_some());
}
