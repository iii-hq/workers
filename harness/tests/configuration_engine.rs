//! Stored harness configurations upgraded through the real configuration authority.

#[path = "support/engine.rs"]
mod engine;

use std::time::Duration;

use engine::{isolated_engine, worker_options};
use harness::config::WorkerConfig;
use harness::configuration;
use harness::types::turn::ExposeMode;
use iii_sdk::{CallRequest, IIIClient, register_worker};
use serde_json::{Value, json};

async fn call(iii: &IIIClient, function: &str, payload: Value) -> Value {
    iii.call(CallRequest {
        function_id: function.into(),
        payload,
        action: None,
        timeout_ms: Some(5_000),
    })
    .await
    .unwrap_or_else(|error| panic!("{function} failed: {error}"))
}

async fn upgrade_stored_policy(current_schema: bool) {
    let Some(engine) = isolated_engine().await else {
        eprintln!("skipping: iii engine is not available");
        return;
    };
    let iii = register_worker(&engine.url, worker_options("configuration-upgrade"));
    iii.wait_until_registered(Duration::from_secs(5))
        .await
        .expect("test worker registers");
    let id = configuration::config_id();
    let stored = json!({
        "default_functions": {
            "allow": ["state::get", "session::list"],
            "deny": ["state::set"],
            "expose": "agent_trigger"
        },
        "default_max_turns": "${HARNESS_UPGRADE_TEST_MAX_TURNS:37}"
    });
    let mut previous_schema = WorkerConfig::json_schema();
    previous_schema["definitions"]["ExposeMode"]["enum"] = json!(["agent_trigger", "native"]);
    call(
        &iii,
        "configuration::ensure",
        json!({
            "id": id,
            "name": "Harness",
            "description": "Harness configuration from the previous release.",
            "schema": previous_schema,
            "initial_value": stored
        }),
    )
    .await;
    if current_schema {
        // A failed upgrade may already have published the new schema while
        // keeping the stored value. Even the initial legacy probe must be raw.
        call(
            &iii,
            "configuration::ensure",
            json!({
                "id": id,
                "name": "Harness",
                "description": "Harness configuration after a failed upgrade.",
                "schema": WorkerConfig::json_schema()
            }),
        )
        .await;
    }

    configuration::register_config(&iii, None)
        .await
        .expect("the stored policy is migrated before schema-validated reads");
    let loaded = configuration::fetch_config(&iii)
        .await
        .expect("the upgraded configuration loads through the authority");
    assert_eq!(
        loaded.default_functions.expect("stored policy").expose,
        ExposeMode::AgentCall
    );
    assert_eq!(loaded.default_max_turns, 37);
    let mut expected = stored;
    expected["default_functions"]["expose"] = json!("agent_call");
    let raw = call(&iii, "configuration::get", json!({"id": id, "raw": true})).await;
    assert_eq!(raw["value"], expected, "only the exposure name changes");

    configuration::register_config(&iii, None)
        .await
        .expect("a second boot is idempotent");
    assert_eq!(
        call(&iii, "configuration::get", json!({"id": id, "raw": true})).await["value"],
        expected
    );
    let schema = call(&iii, "configuration::schema", json!({"id": id})).await;
    assert_eq!(schema["schema"], WorkerConfig::json_schema());
    iii.shutdown_async().await;
}

#[tokio::test]
async fn legacy_exposure_loads_through_the_configuration_authority() {
    upgrade_stored_policy(false).await;
}

#[tokio::test]
async fn legacy_exposure_recovers_after_the_current_schema_was_registered() {
    upgrade_stored_policy(true).await;
}
