//! Readiness failure vectors (hidden internal functions, missing context
//! manager, wrong queue topic, schema/seed mismatch) exercised over fake catalogs — the pure checks are the same
//! code the live probe runs.

use harness_integration::readiness::{
    config_failure, has_function, has_registered_trigger, missing_functions, missing_trigger_types,
    registered_trigger_failures, topic_failures, ExpectedTriggerBinding, ReadinessSpec,
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
fn function_catalog_requires_every_structured_id() {
    let required = ["recorder::target", "recorder::secondary"];
    let complete = json!({ "functions": [
        { "function_id": "recorder::target" },
        { "id": "recorder::secondary" }
    ]});
    assert!(required.iter().all(|id| has_function(&complete, id)));

    let incidental_text = json!({ "functions": [
        {
            "function_id": "recorder::target",
            "description": "calls recorder::secondary"
        }
    ]});
    assert!(!required.iter().all(|id| has_function(&incidental_text, id)));
}

#[test]
fn registered_trigger_catalog_requires_every_bound_function_id() {
    let required = ["integration-recorder::lifecycle", "recorder::binding"];
    let complete = json!([
        { "function_id": "integration-recorder::lifecycle" },
        { "function_id": "recorder::binding" }
    ]);
    assert!(required
        .iter()
        .all(|id| has_registered_trigger(&complete, id)));

    let incomplete = json!([
        { "function_id": "integration-recorder::lifecycle" }
    ]);
    assert!(!required
        .iter()
        .all(|id| has_registered_trigger(&incomplete, id)));
}

#[test]
fn registered_trigger_contract_requires_exact_type_config_and_cardinality() {
    let expected = vec![ExpectedTriggerBinding {
        trigger_type: "harness::turn-completed".into(),
        function_id: "integration-recorder::lifecycle".into(),
        config: json!({ "session_id": "s_1" }),
    }];
    let exact = json!({ "registered_triggers": [{
        "id": "trigger-1",
        "trigger_type": "harness::turn-completed",
        "function_id": "integration-recorder::lifecycle",
        "worker_name": "integration-recorder",
        "config": { "session_id": "s_1" },
        "config_summary": "{}"
    }]});
    assert!(registered_trigger_failures(&expected, &exact).is_empty());

    let wrong_session = json!({ "registered_triggers": [{
        "id": "trigger-1",
        "trigger_type": "harness::turn-completed",
        "function_id": "integration-recorder::lifecycle",
        "worker_name": "integration-recorder",
        "config": { "session_id": "s_other" },
        "config_summary": "{}"
    }]});
    assert!(!registered_trigger_failures(&expected, &wrong_session).is_empty());

    let duplicate = json!({ "registered_triggers": [
        exact["registered_triggers"][0].clone(),
        {
            "id": "trigger-2",
            "trigger_type": "harness::turn-completed",
            "function_id": "integration-recorder::lifecycle",
            "worker_name": "integration-recorder",
            "config": { "session_id": "s_1" },
            "config_summary": "{}"
        }
    ]});
    assert!(!registered_trigger_failures(&expected, &duplicate).is_empty());
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
        { "function_id": "integration-recorder::lifecycle" },
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
