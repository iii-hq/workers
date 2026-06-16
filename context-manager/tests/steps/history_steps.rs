//! Given-steps that build the candidate history. Sizes are expressed
//! in heuristic tokens (`~N tokens` -> N*4 chars of text), so prune
//! accounting in assertions is exact.

use cucumber::given;
use serde_json::{json, Value};

use crate::common::world::ContextWorld;

fn text_block(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

fn sized_text(tokens: u64) -> String {
    "x".repeat((tokens * 4) as usize)
}

pub fn push_user(world: &mut ContextWorld, text: &str) {
    let ts = world.tick();
    world
        .messages
        .push(json!({ "role": "user", "content": [text_block(text)], "timestamp": ts }));
}

pub fn push_assistant(world: &mut ContextWorld, content: Vec<Value>, stop_reason: &str) {
    let ts = world.tick();
    world.messages.push(json!({
        "role": "assistant", "content": content, "stop_reason": stop_reason,
        "model": "test-model", "provider": "test", "timestamp": ts
    }));
}

#[given("an empty history")]
async fn empty_history(world: &mut ContextWorld) {
    world.messages.clear();
}

#[given(regex = r#"^a user message "([^"]*)"$"#)]
async fn user_message(world: &mut ContextWorld, text: String) {
    push_user(world, &text);
}

#[given(regex = r#"^a user message of ~(\d+) tokens$"#)]
async fn user_message_sized(world: &mut ContextWorld, tokens: u64) {
    push_user(world, &sized_text(tokens));
}

#[given(regex = r#"^an assistant message "([^"]*)"$"#)]
async fn assistant_message(world: &mut ContextWorld, text: String) {
    push_assistant(world, vec![text_block(&text)], "end");
}

#[given(regex = r#"^an assistant message of ~(\d+) tokens$"#)]
async fn assistant_message_sized(world: &mut ContextWorld, tokens: u64) {
    push_assistant(world, vec![text_block(&sized_text(tokens))], "end");
}

#[given(regex = r#"^an assistant function call "([^"]+)" to "([^"]+)"$"#)]
async fn assistant_function_call(world: &mut ContextWorld, call_id: String, function_id: String) {
    push_assistant(
        world,
        vec![json!({
            "type": "function_call", "id": call_id, "function_id": function_id,
            "arguments": { "arg": "value" }
        })],
        "function_call",
    );
}

#[given(regex = r#"^a function result for call "([^"]+)" from "([^"]+)" of ~(\d+) tokens$"#)]
async fn function_result_sized(
    world: &mut ContextWorld,
    call_id: String,
    function_id: String,
    tokens: u64,
) {
    let ts = world.tick();
    world.messages.push(json!({
        "role": "function_result", "function_call_id": call_id, "function_id": function_id,
        "content": [text_block(&sized_text(tokens))], "details": {}, "is_error": false,
        "timestamp": ts
    }));
}

#[given(regex = r#"^a function result for call "([^"]+)" from "([^"]+)" saying "([^"]*)"$"#)]
async fn function_result_text(
    world: &mut ContextWorld,
    call_id: String,
    function_id: String,
    text: String,
) {
    let ts = world.tick();
    world.messages.push(json!({
        "role": "function_result", "function_call_id": call_id, "function_id": function_id,
        "content": [text_block(&text)], "details": {}, "is_error": false, "timestamp": ts
    }));
}

#[given(regex = r#"^a custom message of type "([^"]+)"$"#)]
async fn custom_message(world: &mut ContextWorld, custom_type: String) {
    let ts = world.tick();
    world.messages.push(json!({
        "role": "custom", "custom_type": custom_type,
        "content": [text_block("app-facing bookkeeping")], "timestamp": ts
    }));
}

#[given(regex = r#"^a custom message of type "([^"]+)" of ~(\d+) tokens$"#)]
async fn custom_message_sized(world: &mut ContextWorld, custom_type: String, tokens: u64) {
    let ts = world.tick();
    world.messages.push(json!({
        "role": "custom", "custom_type": custom_type,
        "content": [text_block(&sized_text(tokens))], "timestamp": ts
    }));
}

#[given(regex = r#"^a user message carrying an inline function result block for call "([^"]+)"$"#)]
async fn user_with_inline_result(world: &mut ContextWorld, call_id: String) {
    let ts = world.tick();
    world.messages.push(json!({
        "role": "user",
        "content": [
            { "type": "function_result", "function_call_id": call_id,
              "content": [{ "type": "text", "text": "inline result" }] }
        ],
        "timestamp": ts
    }));
}

#[given(regex = r#"^a user message containing an image of (\d+) chars$"#)]
async fn user_with_image(world: &mut ContextWorld, chars: usize) {
    let ts = world.tick();
    world.messages.push(json!({
        "role": "user",
        "content": [{ "type": "image", "mime": "image/png", "data": "A".repeat(chars) }],
        "timestamp": ts
    }));
}

/// `N` plain user/assistant turn pairs of `~tokens` each (the user
/// message carries the bulk; the assistant reply is tiny).
#[given(regex = r#"^(\d+) conversation turns of ~(\d+) tokens each$"#)]
async fn conversation_turns(world: &mut ContextWorld, turn_count: usize, tokens: u64) {
    for i in 0..turn_count {
        push_user(world, &sized_text(tokens));
        push_assistant(world, vec![text_block(&format!("reply {i}"))], "end");
    }
}
