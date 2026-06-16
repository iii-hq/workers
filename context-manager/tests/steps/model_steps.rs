//! Model catalog (fake router), inline-limit models, and config knobs.

use cucumber::{given, then};
use std::sync::atomic::Ordering;

use context_manager::types::{Model, ThinkingLevel};
use serde_json::json;

use crate::common::world::ContextWorld;

fn parse_level(raw: &str) -> ThinkingLevel {
    serde_json::from_value(json!(raw)).unwrap_or_else(|_| panic!("unknown thinking level {raw}"))
}

#[given(
    regex = r#"^the router knows model "([^"]+)" with context window (\d+) and max output (\d+)$"#
)]
async fn router_model(world: &mut ContextWorld, id: String, context_window: u64, max_output: u64) {
    world.resolver.insert(Model {
        id: id.clone(),
        provider: "test".into(),
        context_window,
        max_output_tokens: max_output,
        input_limit: None,
        thinking_budgets: None,
    });
    world.model_inputs.insert(id.clone(), json!({ "id": id }));
}

#[given(
    regex = r#"^the router knows model "([^"]+)" with input limit (\d+), context window (\d+) and max output (\d+)$"#
)]
async fn router_model_input_limit(
    world: &mut ContextWorld,
    id: String,
    input_limit: u64,
    context_window: u64,
    max_output: u64,
) {
    world.resolver.insert(Model {
        id: id.clone(),
        provider: "test".into(),
        context_window,
        max_output_tokens: max_output,
        input_limit: Some(input_limit),
        thinking_budgets: None,
    });
    world.model_inputs.insert(id.clone(), json!({ "id": id }));
}

#[given(
    regex = r#"^the router model "([^"]+)" declares a thinking budget of (\d+) for "([^"]+)"$"#
)]
async fn router_model_thinking(world: &mut ContextWorld, id: String, budget: u64, level: String) {
    let mut models = world
        .resolver
        .models
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let model = models
        .get_mut(&id)
        .unwrap_or_else(|| panic!("declare the router model {id} first"));
    model
        .thinking_budgets
        .get_or_insert_with(Default::default)
        .insert(parse_level(&level), budget);
}

#[given("the router is unavailable")]
async fn router_unavailable(world: &mut ContextWorld) {
    world.resolver.unavailable.store(true, Ordering::SeqCst);
}

#[given(regex = r#"^inline model "([^"]+)" with context window (\d+) and max output (\d+)$"#)]
async fn inline_model(world: &mut ContextWorld, id: String, context_window: u64, max_output: u64) {
    world.model_inputs.insert(
        id.clone(),
        json!({
            "id": id,
            "limits": { "context_window": context_window, "max_output_tokens": max_output }
        }),
    );
}

#[given(
    regex = r#"^inline model "([^"]+)" with input limit (\d+), context window (\d+) and max output (\d+)$"#
)]
async fn inline_model_input_limit(
    world: &mut ContextWorld,
    id: String,
    input_limit: u64,
    context_window: u64,
    max_output: u64,
) {
    world.model_inputs.insert(
        id.clone(),
        json!({
            "id": id,
            "limits": {
                "context_window": context_window,
                "max_output_tokens": max_output,
                "input_limit": input_limit
            }
        }),
    );
}

#[given(regex = r#"^config "([^"]+)" is (\d+)$"#)]
async fn config_numeric(world: &mut ContextWorld, field: String, value: u64) {
    match field.as_str() {
        "reserved_tokens_cap" => world.config.reserved_tokens_cap = value,
        "reserved_pct" => world.config.reserved_pct = value,
        "tail_turns" => world.config.tail_turns = value as usize,
        "protect_recent_tokens" => world.config.protect_recent_tokens = value,
        "min_free_tokens" => world.config.min_free_tokens = value,
        "max_output_chars" => world.config.max_output_chars = value as usize,
        "lease_ttl_secs" => world.config.lease_ttl_secs = value,
        "summarizer_timeout_ms" => world.config.summarizer_timeout_ms = value,
        other => panic!("unknown numeric config field {other}"),
    }
}

#[given(regex = r#"^config allow_fallback_limits is (true|false)$"#)]
async fn config_fallback(world: &mut ContextWorld, value: String) {
    world.config.allow_fallback_limits = value == "true";
}

#[then(regex = r#"^the router was asked for model "([^"]+)"$"#)]
async fn router_was_asked(world: &mut ContextWorld, id: String) {
    let calls = world
        .resolver
        .calls
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    assert!(
        calls.iter().any(|(_, called_id)| called_id == &id),
        "router was never asked for {id}; calls: {calls:?}"
    );
}

#[then("the router was never queried")]
async fn router_never_queried(world: &mut ContextWorld) {
    let calls = world
        .resolver
        .calls
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    assert!(
        calls.is_empty(),
        "expected no router lookups, got {calls:?}"
    );
}
