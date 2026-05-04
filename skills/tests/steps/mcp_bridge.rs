//! Step defs for tests/features/mcp_bridge.feature.
//!
//! End-to-end shape checks that prove the internal RPC the `mcp` worker
//! will call returns exactly the JSON envelope MCP expects.

use cucumber::{given, then, when};
use iii_sdk::{IIIError, RegisterFunction, TriggerRequest};
use serde_json::{json, Value};

use crate::common::world::IiiSkillsWorld;

const LAST: &str = "mcp_bridge_last";

async fn call(world: &mut IiiSkillsWorld, function_id: &str, payload: Value) {
    world.stash.remove(LAST);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    if let Ok(v) = iii
        .trigger(TriggerRequest {
            function_id: function_id.into(),
            payload,
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
    {
        world.stash.insert(LAST.into(), v);
    }
}

#[given("a skill and a prompt are registered for the bridge test")]
async fn seed_bridge(world: &mut IiiSkillsWorld) {
    let Some(iii) = world.iii.clone() else {
        return;
    };
    let skill_id = world.scoped_id("bridge");
    world
        .stash
        .insert("bridge_skill_id".into(), Value::String(skill_id.clone()));
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "skills::register".to_string(),
            payload: json!({
                "id": skill_id,
                "skill": "# bridge\n\nbody content\n",
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;

    // Handler backing the prompt.
    let handler_id = format!("bdd::bridge-handler-{}", world.unique_id);
    world.stash.insert(
        "bridge_handler_id".into(),
        Value::String(handler_id.clone()),
    );
    iii.register_function(
        RegisterFunction::new_async(handler_id.clone(), |_input: Value| async move {
            Ok::<_, IIIError>(json!({ "content": "bridge body" }))
        })
        .description("bdd bridge prompt handler"),
    );
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;

    let prompt_name = world.scoped_id("bridge-prompt");
    world.stash.insert(
        "bridge_prompt_name".into(),
        Value::String(prompt_name.clone()),
    );
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "prompts::register".to_string(),
            payload: json!({
                "name": prompt_name,
                "description": "bdd bridge",
                "function_id": handler_id,
                "arguments": [
                    { "name": "to", "description": "Recipient", "required": true }
                ]
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
}

// ── resources-list ──────────────────────────────────────────────────────

#[when("I call skills::resources-list through the bridge")]
async fn bridge_resources_list(world: &mut IiiSkillsWorld) {
    call(world, "skills::resources-list", json!({})).await;
}

#[then("the bridged resources envelope has the iii://skills index")]
fn bridge_has_index(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    let arr = v["resources"].as_array().expect("missing .resources");
    assert!(
        arr.iter()
            .any(|r| r["uri"].as_str() == Some("iii://skills")),
        "missing iii://skills: {arr:?}"
    );
}

#[then("the bridged resources envelope includes the seeded skill uri")]
fn bridge_has_seeded(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let id = world
        .stash
        .get("bridge_skill_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let want = format!("iii://{id}");
    let v = world.stash.get(LAST).expect("no response recorded");
    let arr = v["resources"].as_array().expect("missing .resources");
    assert!(
        arr.iter().any(|r| r["uri"].as_str() == Some(want.as_str())),
        "missing {want:?}: {arr:?}"
    );
}

// ── resources-read ──────────────────────────────────────────────────────

#[when("I call skills::resources-read on the seeded skill uri")]
async fn bridge_resources_read(world: &mut IiiSkillsWorld) {
    let id = world
        .stash
        .get("bridge_skill_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    call(
        world,
        "skills::resources-read",
        json!({ "uri": format!("iii://{id}") }),
    )
    .await;
}

#[then("the bridged read contents mime is text/markdown")]
fn bridge_read_mime_md(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    let mime = v["contents"][0]["mimeType"].as_str().unwrap_or("");
    assert_eq!(mime, "text/markdown", "{v}");
}

#[then("the bridged read contents text includes the skill body")]
fn bridge_read_text_has_body(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    let text = v["contents"][0]["text"].as_str().unwrap_or("");
    assert!(text.contains("body content"), "{text}");
}

// ── prompts mcp-list ────────────────────────────────────────────────────

#[when("I call prompts::mcp-list through the bridge")]
async fn bridge_mcp_list(world: &mut IiiSkillsWorld) {
    call(world, "prompts::mcp-list", json!({})).await;
}

#[then("the bridged prompts listing includes the seeded prompt with a required argument")]
fn bridge_prompts_listing(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let name = world
        .stash
        .get("bridge_prompt_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let v = world.stash.get(LAST).expect("no response recorded");
    let arr = v["prompts"].as_array().expect("missing .prompts");
    let entry = arr
        .iter()
        .find(|p| p["name"].as_str() == Some(name.as_str()))
        .unwrap_or_else(|| panic!("missing prompt {name:?}: {arr:?}"));
    let args = entry["arguments"].as_array().expect("missing arguments");
    assert_eq!(args.len(), 1, "args len: {entry}");
    assert_eq!(args[0]["name"].as_str(), Some("to"));
    assert_eq!(args[0]["required"].as_bool(), Some(true));
}

// ── prompts mcp-get ─────────────────────────────────────────────────────

#[when("I call prompts::mcp-get through the bridge with arguments to=alice@example.com")]
async fn bridge_mcp_get(world: &mut IiiSkillsWorld) {
    let name = world
        .stash
        .get("bridge_prompt_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    call(
        world,
        "prompts::mcp-get",
        json!({
            "name": name,
            "arguments": { "to": "alice@example.com" }
        }),
    )
    .await;
}

#[then("the bridged prompts::mcp-get returns a single user message")]
fn bridge_mcp_get_shape(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    assert!(
        v["description"].as_str().is_some(),
        "missing description: {v}"
    );
    let messages = v["messages"].as_array().expect("missing messages");
    assert_eq!(messages.len(), 1, "{v}");
    assert_eq!(messages[0]["role"].as_str(), Some("user"));
}
