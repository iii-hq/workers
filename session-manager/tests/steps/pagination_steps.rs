//! Steps for the lazy transcript readers: fixtures that build tool runs the
//! way the harness writes them (an assistant message calling a function,
//! then the result answering it), and a walk that pages `messages-tail`
//! back to the root to compare it with `session::messages`.

use cucumber::{given, then, when};
use serde_json::{json, Value};

use crate::common::world::SessionWorld;

async fn must_succeed(world: &mut SessionWorld, what: &str) {
    if let Some(err) = &world.last_error {
        panic!("fixture step `{what}` failed: {err}");
    }
}

/// The harness shape of a generate step that calls a function: a thinking
/// block, optional prose, one `function_call`, `stop_reason: function_call`.
fn call_message(call_id: &str, function_id: &str, text: Option<&str>) -> Value {
    let mut content = vec![json!({ "type": "thinking", "text": "let me look" })];
    if let Some(text) = text {
        content.push(json!({ "type": "text", "text": text }));
    }
    content.push(json!({
        "type": "function_call", "id": call_id, "function_id": function_id,
        "arguments": { "cmd": "ls" }
    }));
    json!({
        "role": "assistant", "content": content, "stop_reason": "function_call",
        "model": "test-model", "provider": "test-provider", "timestamp": 0
    })
}

fn result_message(call_id: &str, function_id: &str) -> Value {
    json!({
        "role": "function_result", "function_call_id": call_id, "function_id": function_id,
        "content": [{ "type": "text", "text": "README.md" }],
        "details": { "code": 0 }, "is_error": false, "timestamp": 0
    })
}

async fn append(world: &mut SessionWorld, session_id: &str, message: Value, what: &str) {
    world
        .call_pure(
            "session::append",
            json!({ "session_id": session_id, "message": message }),
        )
        .await;
    must_succeed(world, what).await;
}

#[given(regex = r#"^an assistant call "([^"]+)" to "([^"]+)" appended to "([^"]+)"$"#)]
async fn assistant_call(
    world: &mut SessionWorld,
    call_id: String,
    function_id: String,
    sid: String,
) {
    let sid = world.substitute(&sid);
    let message = call_message(&call_id, &function_id, None);
    append(world, &sid, message, "an assistant call").await;
}

#[given(
    regex = r#"^an assistant call "([^"]+)" to "([^"]+)" saying "([^"]*)" appended to "([^"]+)"$"#
)]
async fn assistant_call_saying(
    world: &mut SessionWorld,
    call_id: String,
    function_id: String,
    text: String,
    sid: String,
) {
    let sid = world.substitute(&sid);
    let message = call_message(&call_id, &function_id, Some(&text));
    append(world, &sid, message, "an assistant call saying").await;
}

#[given(regex = r#"^a function result for "([^"]+)" appended to "([^"]+)"$"#)]
async fn function_result(world: &mut SessionWorld, call_id: String, sid: String) {
    let sid = world.substitute(&sid);
    let message = result_message(&call_id, "shell::run");
    append(world, &sid, message, "a function result").await;
}

#[given(regex = r#"^a final assistant message "([^"]*)" appended to "([^"]+)"$"#)]
async fn final_assistant(world: &mut SessionWorld, text: String, sid: String) {
    let sid = world.substitute(&sid);
    let message = json!({
        "role": "assistant", "content": [{ "type": "text", "text": text }], "stop_reason": "end",
        "model": "test-model", "provider": "test-provider", "timestamp": 0
    });
    append(world, &sid, message, "a final assistant message").await;
}

/// `n` call/result pairs with ids `<prefix>_1..n`, the way a long agent
/// phase lands in the transcript.
#[given(regex = r#"^a tool run of (\d+) calls named "([^"]+)" appended to "([^"]+)"$"#)]
async fn tool_run(world: &mut SessionWorld, n: usize, prefix: String, sid: String) {
    let sid = world.substitute(&sid);
    for i in 1..=n {
        let call_id = format!("{prefix}_{i}");
        let call = call_message(&call_id, "shell::run", None);
        append(world, &sid, call, "a tool run call").await;
        let result = result_message(&call_id, "shell::run");
        append(world, &sid, result, "a tool run result").await;
    }
}

/// Page `messages-tail` from the newest page back to the root with the
/// given limit, collecting entry ids oldest first, and compare with one
/// exhaustive `session::messages` read: no gaps, no duplicates, same order.
/// The walk also asserts every page is non-empty while `has_more` holds, so
/// a cursor that stops making progress fails here instead of hanging a UI.
#[when(regex = r#"^I walk "([^"]+)" back through "session::messages-tail" with limit (\d+)$"#)]
#[then(
    regex = r#"^walking "([^"]+)" back through "session::messages-tail" with limit (\d+) matches "session::messages"$"#
)]
async fn walk_back(world: &mut SessionWorld, sid: String, limit: usize) {
    let sid = world.substitute(&sid);
    let mut walked: Vec<String> = Vec::new();
    let mut before: Option<String> = None;
    loop {
        let mut payload = json!({ "session_id": sid, "limit": limit });
        if let Some(b) = &before {
            payload["before_entry_id"] = json!(b);
        }
        world.call_pure("session::messages-tail", payload).await;
        let resp = world
            .last_response
            .clone()
            .unwrap_or_else(|| panic!("messages-tail failed: {:?}", world.last_error));
        let page: Vec<String> = resp["messages"]
            .as_array()
            .expect("messages array")
            .iter()
            .map(|m| m["entry_id"].as_str().expect("entry_id").to_string())
            .collect();
        let has_more = resp["has_more"].as_bool().expect("has_more");
        assert!(
            !page.is_empty() || !has_more,
            "an empty page claimed more history: {resp}"
        );
        assert_eq!(
            resp["oldest_entry_id"].as_str().map(str::to_string),
            page.first().cloned(),
            "oldest_entry_id must name the page's first entry"
        );
        walked.splice(0..0, page.iter().cloned());
        if !has_more {
            break;
        }
        before = page.first().cloned();
    }

    let mut full: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut payload = json!({ "session_id": sid, "include_custom": true, "limit": 500 });
        if let Some(c) = &cursor {
            payload["cursor"] = json!(c);
        }
        world.call_pure("session::messages", payload).await;
        let resp = world.last_response.clone().expect("messages succeeds");
        for m in resp["messages"].as_array().expect("messages array") {
            full.push(m["entry_id"].as_str().expect("entry_id").to_string());
        }
        match resp["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
    }
    assert_eq!(walked, full, "the paged walk must reproduce the full path");
}
