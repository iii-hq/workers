//! Sugar steps that build common fixtures through the production
//! handlers (never by poking the store directly), so every fixture is
//! itself a contract exercise.

use cucumber::gherkin::Step;
use cucumber::{given, when};
use serde_json::json;

use super::common_steps::docstring_payload;
use crate::common::world::SessionWorld;

async fn must_succeed(world: &mut SessionWorld, what: &str) {
    if let Some(err) = &world.last_error {
        panic!("fixture step `{what}` failed: {err}");
    }
}

#[given("a bare session")]
async fn bare_session(world: &mut SessionWorld) {
    world.call_pure("session::create", json!({})).await;
    must_succeed(world, "a bare session").await;
}

#[given(regex = r#"^a session created with title "([^"]*)"$"#)]
async fn session_with_title(world: &mut SessionWorld, title: String) {
    world
        .call_pure("session::create", json!({ "title": title }))
        .await;
    must_succeed(world, "a session created with title").await;
}

#[given("a session created with:")]
async fn session_created_with(world: &mut SessionWorld, step: &Step) {
    let payload = docstring_payload(world, step);
    world.call_pure("session::create", payload).await;
    must_succeed(world, "a session created with payload").await;
}

#[given(regex = r#"^a user message "([^"]*)" appended to "([^"]+)"$"#)]
#[when(regex = r#"^I append a user message "([^"]*)" to "([^"]+)"$"#)]
async fn append_user_message(world: &mut SessionWorld, text: String, session_id: String) {
    let session_id = world.substitute(&session_id);
    let now = 0; // message timestamps are caller-supplied; 0 keeps fixtures terse
    world
        .call_pure(
            "session::append",
            json!({
                "session_id": session_id,
                "message": {
                    "role": "user",
                    "content": [{ "type": "text", "text": text }],
                    "timestamp": now
                }
            }),
        )
        .await;
}

#[given(regex = r#"^an empty assistant message appended to "([^"]+)"$"#)]
#[when(regex = r#"^I append an empty assistant message to "([^"]+)"$"#)]
async fn append_assistant_message(world: &mut SessionWorld, session_id: String) {
    let session_id = world.substitute(&session_id);
    world
        .call_pure(
            "session::append",
            json!({
                "session_id": session_id,
                "message": {
                    "role": "assistant",
                    "content": [],
                    "stop_reason": "end",
                    "model": "test-model",
                    "provider": "test-provider",
                    "timestamp": 0
                }
            }),
        )
        .await;
}

#[given(regex = r#"^a custom entry of type "([^"]+)" appended to "([^"]+)"$"#)]
#[when(regex = r#"^I append a custom entry of type "([^"]+)" to "([^"]+)"$"#)]
async fn append_custom_entry(world: &mut SessionWorld, custom_type: String, session_id: String) {
    let session_id = world.substitute(&session_id);
    world
        .call_pure(
            "session::append",
            json!({
                "session_id": session_id,
                "custom": { "custom_type": custom_type, "data": { "marker": custom_type } }
            }),
        )
        .await;
}
