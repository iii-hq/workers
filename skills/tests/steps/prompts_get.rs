//! Step defs for tests/features/prompts_get.feature.
//!
//! Drives `prompts::mcp-get` (the internal RPC mcp uses to answer MCP
//! `prompts/get`). Validates the three normalization shapes, not-found
//! errors, and the hard-floor guard against infra-namespace handlers.

use cucumber::{given, then, when};
use iii_sdk::{IIIError, RegisterFunction, TriggerRequest};
use serde_json::{json, Value};

use crate::common::world::IiiSkillsWorld;

const LAST_OK: &str = "prompts_get_last_ok";
const LAST_ERR: &str = "prompts_get_last_err";
const REGISTERED_NAME: &str = "prompts_get_registered_name";

async fn register_handler_returning<S: AsRef<str>>(world: &mut IiiSkillsWorld, kind: S) -> String {
    let Some(iii) = world.iii.clone() else {
        return String::new();
    };
    let kind = kind.as_ref().to_string();
    let fn_id = format!("bdd::prompt-handler-{kind}-{}", world.unique_id);
    iii.register_function(
        RegisterFunction::new_async(fn_id.clone(), move |_input: Value| {
            let k = kind.clone();
            async move {
                match k.as_str() {
                    "string" => Ok::<_, IIIError>(Value::String("str body".to_string())),
                    "content" => Ok::<_, IIIError>(json!({ "content": "obj body" })),
                    "messages" => Ok::<_, IIIError>(json!({
                        "messages": [
                            { "role": "user", "content": { "type": "text", "text": "m1" } },
                            { "role": "assistant", "content": { "type": "text", "text": "m2" } }
                        ]
                    })),
                    "bogus" => Ok::<_, IIIError>(json!({ "x": 1 })),
                    _ => Err(IIIError::Handler(format!("unknown kind {k}"))),
                }
            }
        })
        .description("bdd: prompt handler variant"),
    );
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    fn_id
}

async fn register_prompt_pointing_at(world: &mut IiiSkillsWorld, function_id: &str) -> String {
    let Some(iii) = world.iii.clone() else {
        return String::new();
    };
    let name = world.scoped_id("pget");
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "prompts::register".to_string(),
            payload: json!({
                "name": name,
                "description": "bdd prompt",
                "function_id": function_id,
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
    world
        .stash
        .insert(REGISTERED_NAME.into(), Value::String(name.clone()));
    name
}

async fn call_mcp_get(world: &mut IiiSkillsWorld, payload: Value) {
    world.stash.remove(LAST_OK);
    world.stash.remove(LAST_ERR);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    match iii
        .trigger(TriggerRequest {
            function_id: "prompts::mcp-get".to_string(),
            payload,
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
    {
        Ok(v) => {
            world.stash.insert(LAST_OK.into(), v);
        }
        Err(e) => {
            world
                .stash
                .insert(LAST_ERR.into(), Value::String(e.to_string()));
        }
    }
}

// ── seed handlers ───────────────────────────────────────────────────────

#[given("a prompt handler that returns a string")]
async fn seed_string(world: &mut IiiSkillsWorld) {
    let fn_id = register_handler_returning(world, "string").await;
    let _ = register_prompt_pointing_at(world, &fn_id).await;
}

#[given("a prompt handler that returns a {content} object")]
async fn seed_content(world: &mut IiiSkillsWorld) {
    let fn_id = register_handler_returning(world, "content").await;
    let _ = register_prompt_pointing_at(world, &fn_id).await;
}

#[given("a prompt handler that returns a {messages: [...]} object")]
async fn seed_messages(world: &mut IiiSkillsWorld) {
    let fn_id = register_handler_returning(world, "messages").await;
    let _ = register_prompt_pointing_at(world, &fn_id).await;
}

#[given("a prompt handler that returns an unsupported shape")]
async fn seed_bogus(world: &mut IiiSkillsWorld) {
    let fn_id = register_handler_returning(world, "bogus").await;
    let _ = register_prompt_pointing_at(world, &fn_id).await;
}

#[given(regex = r#"^a prompt pointing at the infra function "([^"]+)"$"#)]
async fn seed_prompt_infra(world: &mut IiiSkillsWorld, function_id: String) {
    let _ = register_prompt_pointing_at(world, &function_id).await;
}

// ── actions ─────────────────────────────────────────────────────────────

#[when("I call prompts::mcp-get on the scoped prompt")]
async fn get_scoped(world: &mut IiiSkillsWorld) {
    let Some(name_value) = world.stash.get(REGISTERED_NAME).cloned() else {
        return;
    };
    let name = name_value.as_str().unwrap_or("").to_string();
    call_mcp_get(world, json!({ "name": name, "arguments": {} })).await;
}

#[when(regex = r#"^I call prompts::mcp-get with an unknown name "([^"]*)"$"#)]
async fn get_unknown(world: &mut IiiSkillsWorld, name: String) {
    call_mcp_get(world, json!({ "name": name })).await;
}

// ── assertions ──────────────────────────────────────────────────────────

#[then("the mcp-get call succeeds")]
fn get_ok(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    assert!(
        world.stash.contains_key(LAST_OK),
        "expected success; got error: {:?}",
        world.stash.get(LAST_ERR)
    );
}

#[then("the mcp-get call fails")]
fn get_fails(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    assert!(
        world.stash.contains_key(LAST_ERR),
        "expected failure; got success: {:?}",
        world.stash.get(LAST_OK)
    );
}

#[then(regex = r#"^the mcp-get error mentions "([^"]+)"$"#)]
fn get_error_mentions(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let err = world
        .stash
        .get(LAST_ERR)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    assert!(err.contains(&needle), "error missing {needle:?}: {err:?}");
}

#[then(regex = r#"^the messages array has length (\d+)$"#)]
fn messages_len(world: &mut IiiSkillsWorld, want: usize) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_OK).expect("no success recorded");
    let arr = v["messages"].as_array().expect("missing .messages");
    assert_eq!(arr.len(), want, "messages len mismatch: {v}");
}

#[then(regex = r#"^message (\d+) has role "([^"]+)" and text "([^"]+)"$"#)]
fn message_shape(world: &mut IiiSkillsWorld, idx: usize, role: String, text: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_OK).expect("no success recorded");
    let arr = v["messages"].as_array().expect("missing .messages");
    let msg = arr
        .get(idx)
        .unwrap_or_else(|| panic!("no message {idx}: {arr:?}"));
    assert_eq!(
        msg["role"].as_str().unwrap_or(""),
        role,
        "role at {idx}: {msg}"
    );
    assert_eq!(
        msg["content"]["type"].as_str().unwrap_or(""),
        "text",
        "content.type at {idx}: {msg}"
    );
    assert_eq!(
        msg["content"]["text"].as_str().unwrap_or(""),
        text,
        "content.text at {idx}: {msg}"
    );
}

#[then(regex = r#"^the result description is "([^"]+)"$"#)]
fn result_description(world: &mut IiiSkillsWorld, want: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_OK).expect("no success recorded");
    assert_eq!(v["description"].as_str().unwrap_or(""), want, "{v}");
}
