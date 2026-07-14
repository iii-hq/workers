//! High-level When-steps that call the four functions over the built
//! history, with optional options/system-prompt docstrings.

use cucumber::gherkin::Step;
use cucumber::when;
use serde_json::{json, Value};

use crate::common::world::ContextWorld;
use crate::steps::common_steps::docstring_payload;

fn history(world: &ContextWorld) -> Value {
    Value::Array(world.messages.clone())
}

#[when(regex = r#"^I count tokens with model "([^"]+)"$"#)]
async fn count_tokens(world: &mut ContextWorld, model: String) {
    let payload = json!({ "messages": history(world), "model": world.model_input(&model) });
    world.call_pure("context::count-tokens", payload).await;
}

#[when(regex = r#"^I count tokens with model "([^"]+)" and system prompt "([^"]*)"$"#)]
async fn count_tokens_system(world: &mut ContextWorld, model: String, system_prompt: String) {
    let payload = json!({
        "messages": history(world),
        "model": world.model_input(&model),
        "system_prompt": system_prompt
    });
    world.call_pure("context::count-tokens", payload).await;
}

#[when(regex = r#"^I count tokens with model "([^"]+)" and tools:$"#)]
async fn count_tokens_tools(world: &mut ContextWorld, model: String, step: &Step) {
    let tools = docstring_payload(world, step);
    let payload = json!({
        "messages": history(world),
        "model": world.model_input(&model),
        "tools": tools
    });
    world.call_pure("context::count-tokens", payload).await;
}

#[when("I prune the history")]
async fn prune(world: &mut ContextWorld) {
    let payload = json!({ "messages": history(world) });
    world.call_pure("context::prune", payload).await;
}

#[when("I prune the history with options:")]
async fn prune_options(world: &mut ContextWorld, step: &Step) {
    let options = docstring_payload(world, step);
    let payload = json!({ "messages": history(world), "options": options });
    world.call_pure("context::prune", payload).await;
}

#[when("I prune the response messages again with the same options:")]
async fn prune_again(world: &mut ContextWorld, step: &Step) {
    let options = docstring_payload(world, step);
    let messages = world
        .last_response
        .as_ref()
        .and_then(|r| r.get("messages"))
        .cloned()
        .expect("a prior prune response with messages");
    let payload = json!({ "messages": messages, "options": options });
    world.call_pure("context::prune", payload).await;
}

#[when(regex = r#"^I compact the history with model "([^"]+)"$"#)]
async fn compact(world: &mut ContextWorld, model: String) {
    let payload = json!({ "messages": history(world), "model": world.model_input(&model) });
    world.call_pure("context::compact", payload).await;
}

#[when(regex = r#"^I compact the history with model "([^"]+)" and options:$"#)]
async fn compact_options(world: &mut ContextWorld, model: String, step: &Step) {
    let options = docstring_payload(world, step);
    let payload = json!({
        "messages": history(world),
        "model": world.model_input(&model),
        "options": options
    });
    world.call_pure("context::compact", payload).await;
}

#[when(regex = r#"^I assemble the history with model "([^"]+)"$"#)]
async fn assemble(world: &mut ContextWorld, model: String) {
    let payload = json!({ "messages": history(world), "model": world.model_input(&model) });
    world.call_pure("context::assemble", payload).await;
}

#[when(regex = r#"^I assemble the history with model "([^"]+)" and options:$"#)]
async fn assemble_options(world: &mut ContextWorld, model: String, step: &Step) {
    let options = docstring_payload(world, step);
    let payload = json!({
        "messages": history(world),
        "model": world.model_input(&model),
        "options": options
    });
    world.call_pure("context::assemble", payload).await;
}

#[when(regex = r#"^I assemble the history with model "([^"]+)" and system prompt "([^"]*)"$"#)]
async fn assemble_system(world: &mut ContextWorld, model: String, system_prompt: String) {
    let payload = json!({
        "messages": history(world),
        "model": world.model_input(&model),
        "system_prompt": system_prompt
    });
    world.call_pure("context::assemble", payload).await;
}

#[when(
    regex = r#"^I assemble the history with model "([^"]+)", system prompt "([^"]*)" and options:$"#
)]
async fn assemble_system_options(
    world: &mut ContextWorld,
    model: String,
    system_prompt: String,
    step: &Step,
) {
    let options = docstring_payload(world, step);
    let payload = json!({
        "messages": history(world),
        "model": world.model_input(&model),
        "system_prompt": system_prompt,
        "options": options
    });
    world.call_pure("context::assemble", payload).await;
}

#[when(regex = r#"^I assemble the history with model "([^"]+)" and request fields:$"#)]
async fn assemble_request_fields(world: &mut ContextWorld, model: String, step: &Step) {
    let extras = docstring_payload(world, step);
    let mut payload = json!({
        "messages": history(world),
        "model": world.model_input(&model),
    });
    let payload_object = payload
        .as_object_mut()
        .expect("assemble payload is an object");
    let extras_object = extras
        .as_object()
        .expect("assemble request fields must be a JSON object");
    for (key, value) in extras_object {
        payload_object.insert(key.clone(), value.clone());
    }
    world.call_pure("context::assemble", payload).await;
}
