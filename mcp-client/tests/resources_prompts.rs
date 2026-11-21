#![cfg(feature = "test-fixtures")]
//! Resources and prompts of the fake stdio MCP server as functions: what a
//! capability registers, the call paths, the list-changed triggers and a
//! failed listing.

mod support;

use std::sync::Arc;
use std::time::Duration;

use mcp_client::host::{EventKind, ServerEvent};
use mcp_client::server::{Manager, State};
use serde_json::{Value, json};
use support::stdio_fixture::{cfg, opts};
use support::{FakeHost, RegOp, connect_opts, hi, server, stdio_timings, until};

const TOOLS: [&str; 8] = [
    "big", "cwd", "echo", "env", "exit", "fail", "mutate", "stderr",
];

const SOON: Duration = Duration::from_secs(5);
const RESTART: Duration = Duration::from_secs(10);

async fn ready(host: &Arc<FakeHost>, env: Value) -> Arc<Manager> {
    let m = connect_opts(host, &cfg(env), opts(stdio_timings())).await;
    let ok = m.wait_for("fake", State::Ready, RESTART).await;
    assert!(ok, "fake never reached ready: {}", server(&m, "fake").await);
    m
}

fn has(host: &FakeHost, suffix: &str) -> bool {
    host.registered_ids()
        .contains(&format!("mcp::fake::{suffix}"))
}

fn has_prompts(host: &FakeHost) -> bool {
    host.registered_ids()
        .iter()
        .any(|id| id.starts_with("mcp::fake::prompts::"))
}

fn unregisters_a_prompt(ops: &[RegOp]) -> bool {
    ops.iter()
        .any(|op| matches!(op, RegOp::Unregister(id) if id.contains("::prompts::")))
}

/// A knob file for `FAKE_MCP_STATE_FILE`, read by the fixture at each start.
fn state_file(lines: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("knobs");
    std::fs::write(&path, lines).unwrap();
    (dir, path)
}

async fn mutate(host: &FakeHost, what: &str) {
    host.call("mcp::fake::mutate", json!({"what": what}))
        .await
        .unwrap();
}

#[tokio::test]
async fn no_capability_registers_nothing_extra() {
    let host = FakeHost::new();
    let m = ready(&host, json!({})).await;
    assert_eq!(
        host.registered_ids(),
        TOOLS.map(|t| format!("mcp::fake::{t}"))
    );
    let v = server(&m, "fake").await;
    assert_eq!(
        v["capabilities"],
        json!({"resources": false, "prompts": false})
    );
    assert_eq!(
        (&v["resource_functions"], &v["prompts"]),
        (&json!([]), &json!([]))
    );
    m.shutdown().await;
}

#[tokio::test]
async fn resources_register_list_and_read_and_templates_only_when_one_exists() {
    let host = FakeHost::new();
    let m = ready(&host, json!({"FAKE_MCP_RESOURCES": "1"})).await;
    assert!(has(&host, "resources::list") && has(&host, "resources::read"));
    assert!(!has(&host, "resources::templates"));
    let v = server(&m, "fake").await;
    assert_eq!(v["capabilities"]["resources"], true, "{v}");
    // Method-not-found is no template, not a failure.
    assert_eq!(v["list_errors"]["templates"], Value::Null, "{v}");
    assert_eq!(
        v["resource_functions"],
        json!([
            {"name": "list", "function_id": "mcp::fake::resources::list", "registered": true},
            {"name": "read", "function_id": "mcp::fake::resources::read", "registered": true}
        ])
    );
    m.shutdown().await;

    let host = FakeHost::new();
    let env = json!({"FAKE_MCP_RESOURCES": "1", "FAKE_MCP_TEMPLATES": "1"});
    let m = ready(&host, env).await;
    let templates = host
        .call("mcp::fake::resources::templates", json!({}))
        .await;
    assert_eq!(
        templates,
        Ok(json!({"templates": [{"uri_template": "file:///{name}.txt", "name": "text file"}]}))
    );
    m.shutdown().await;
}

#[tokio::test]
async fn resources_list_is_one_server_page_per_call() {
    let host = FakeHost::new();
    let env = json!({"FAKE_MCP_RESOURCES": "1", "FAKE_MCP_RESOURCE_PAGE": "2"});
    let m = ready(&host, env).await;
    let list = async |args: Value| host.call("mcp::fake::resources::list", args).await;
    let uris = |page: &Value| -> Vec<String> {
        let rows = page["resources"].as_array().unwrap();
        rows.iter()
            .map(|r| r["uri"].as_str().unwrap().to_string())
            .collect()
    };
    let p = list(json!({"_caller_worker_id": "w"})).await.unwrap();
    assert_eq!(uris(&p), ["file:///a.txt", "file:///b.txt"]);
    assert_eq!(p["next_cursor"], "2");
    assert_eq!(
        p["resources"][0],
        json!({"uri": "file:///a.txt", "name": "a.txt", "mime_type": "text/plain"})
    );
    let p = list(json!({"cursor": "2"})).await.unwrap();
    assert_eq!(uris(&p), ["file:///img.png", "file:///bin.dat"]);
    assert_eq!(p["next_cursor"], "4");
    let p = list(json!({"cursor": "4"})).await.unwrap();
    assert_eq!(uris(&p), ["file:///pair"]);
    assert!(p.get("next_cursor").is_none(), "{p}");
    assert_eq!(
        list(json!({"cursor": 2})).await,
        Err("invalid_argument: cursor must be a string".to_string())
    );
    m.shutdown().await;
}

#[tokio::test]
async fn resources_read_translates_each_kind() {
    let host = FakeHost::new();
    let m = ready(&host, json!({"FAKE_MCP_RESOURCES": "1"})).await;
    let read = async |args: Value| host.call("mcp::fake::resources::read", args).await;
    assert_eq!(
        read(json!({"uri": "file:///a.txt"})).await,
        Ok(json!({"content": [{"type": "text", "text": "alpha"}]}))
    );
    assert_eq!(
        read(json!({"uri": "file:///img.png"})).await,
        Ok(json!({"content": [{"type": "image", "mime": "image/png", "data": "iVBORw0KGgo="}]}))
    );
    // The note counts the base64 characters.
    assert_eq!(
        read(json!({"uri": "file:///bin.dat"})).await,
        Ok(
            json!({"content": [{"type": "text", "text": "[binary resource file:///bin.dat, application/octet-stream, 8 bytes, not shown]"}]})
        )
    );
    let pair = read(json!({"uri": "file:///pair"})).await.unwrap();
    let texts: Vec<&str> = pair["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts.len(), 2, "{pair}");
    assert!(texts[0].starts_with("[file:///pair/1]"), "{pair}");
    assert!(texts[1].ends_with("two"), "{pair}");
    assert_eq!(
        read(json!({"uri": "file:///nope"})).await,
        Err("MCP error -32002: resource not found: file:///nope".to_string())
    );
    assert_eq!(
        read(json!({})).await,
        Err("invalid_argument: uri must be a string".to_string())
    );
    m.shutdown().await;
}

#[tokio::test]
async fn one_function_per_prompt_with_the_argument_schema() {
    let host = FakeHost::new();
    let m = ready(&host, json!({"FAKE_MCP_PROMPTS": "1"})).await;
    assert!(has(&host, "prompts::code_review") && has(&host, "prompts::summary"));
    assert!(!has(&host, "prompts::bad.name"));
    let def = host.def("mcp::fake::prompts::code_review").unwrap();
    assert_eq!(def.description, "Review code");
    assert_eq!(def.request_format["required"], json!(["language"]));
    assert_eq!(
        def.request_format["properties"]["focus"],
        json!({"type": "string"})
    );
    // `properties` is an object and objects are re-sorted: the order shows in metadata only.
    assert_eq!(
        def.metadata["mcp"]["arguments"],
        json!(["language", "focus"])
    );
    let summary = host.def("mcp::fake::prompts::summary").unwrap();
    assert_eq!(summary.description, "Summarize");
    let v = server(&m, "fake").await;
    assert_eq!(v["capabilities"]["prompts"], true, "{v}");
    let review = json!({"name": "code_review", "function_id": "mcp::fake::prompts::code_review",
        "registered": true, "title": null, "arguments": 2, "required": 1});
    assert!(v["prompts"].as_array().unwrap().contains(&review), "{v}");
    let skipped = v["skipped"].as_array().unwrap();
    let bad = skipped.iter().any(|s| {
        s["kind"] == "prompt"
            && s["name"] == "bad.name"
            && s["reason"].as_str().is_some_and(|r| !r.is_empty())
    });
    assert!(bad, "{v}");
    m.shutdown().await;
}

#[tokio::test]
async fn prompt_calls_return_messages_with_roles() {
    let host = FakeHost::new();
    let m = ready(&host, json!({"FAKE_MCP_PROMPTS": "1"})).await;
    let prompt = async |name: &str, args: Value| {
        let id = format!("mcp::fake::prompts::{name}");
        host.call(&id, args).await
    };
    assert_eq!(
        prompt(
            "code_review",
            json!({"language": "rust", "_caller_worker_id": "w"})
        )
        .await,
        Ok(json!({"description": "Review code", "messages": [
            {"role": "user", "content": [{"type": "text", "text": "Review rust code, focus: all"}]}]}))
    );
    let coerced = prompt("code_review", json!({"language": 3, "focus": true}))
        .await
        .unwrap();
    assert_eq!(
        coerced["messages"][0]["content"][0]["text"],
        "Review 3 code, focus: true"
    );
    assert_eq!(
        prompt("code_review", json!({"language": {}})).await,
        Err("invalid_argument: language must be a string".to_string())
    );
    let summary = prompt("summary", json!({})).await.unwrap();
    assert_eq!(
        summary["messages"][0],
        json!({"role": "user", "content": [{"type": "image", "mime": "image/png", "data": "iVBORw0KGgo="}]})
    );
    assert_eq!(summary["messages"][1]["role"], "assistant");
    let text = summary["messages"][1]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(
        text.starts_with("[file:///a.txt]") && text.ends_with("alpha"),
        "{summary}"
    );
    m.shutdown().await;
}

#[tokio::test]
async fn prompts_list_changed_adds_changes_and_removes_without_touching_the_tools() {
    let host = FakeHost::new();
    let m = ready(&host, json!({"FAKE_MCP_PROMPTS": "1"})).await;
    let mark = host.ops().len();
    mutate(&host, "add_prompt").await;
    let added = until(SOON, || async { has(&host, "prompts::extra_prompt") }).await;
    assert!(added, "{:?}", host.registered_ids());
    mutate(&host, "change_prompt").await;
    let arguments = || {
        let def = host.def("mcp::fake::prompts::code_review");
        def.map(|d| d.metadata["mcp"]["arguments"].clone())
    };
    let changed = until(SOON, || async {
        arguments() == Some(json!(["language", "focus", "style"]))
    })
    .await;
    assert!(changed, "{:?}", arguments());
    mutate(&host, "remove_prompt").await;
    let removed = until(SOON, || async { !has(&host, "prompts::summary") }).await;
    assert!(removed, "{:?}", host.registered_ids());

    let ops = host.ops()[mark..].to_vec();
    for op in &ops {
        let (RegOp::Register(id) | RegOp::Unregister(id)) = op;
        assert!(id.starts_with("mcp::fake::prompts::"), "{ops:?}");
    }
    let review = "mcp::fake::prompts::code_review".to_string();
    let at = |op: RegOp| ops.iter().position(|o| *o == op);
    let unregister = at(RegOp::Unregister(review.clone())).expect("unregistered");
    let register = at(RegOp::Register(review)).expect("registered again");
    assert!(unregister < register, "{ops:?}");
    let tools = ServerEvent {
        server: "fake".into(),
        kind: EventKind::Tools,
    };
    assert!(host.events().contains(&tools));

    // The tool trigger works over stdio as well, and leaves the prompts alone.
    let mark = host.ops().len();
    mutate(&host, "add_tool").await;
    let added = until(SOON, || async { has(&host, "extra") }).await;
    assert!(added, "{:?}", host.registered_ids());
    assert!(has(&host, "prompts::code_review"));
    let ops = host.ops()[mark..].to_vec();
    assert!(!unregisters_a_prompt(&ops), "{ops:?}");
    m.shutdown().await;
}

#[tokio::test]
async fn resources_list_changed_adds_the_templates_function() {
    let host = FakeHost::new();
    let m = ready(&host, json!({"FAKE_MCP_RESOURCES": "1"})).await;
    assert!(!has(&host, "resources::templates"));
    let tools_events = || {
        let events = host.events();
        events.iter().filter(|e| e.kind == EventKind::Tools).count()
    };
    let before = tools_events();
    mutate(&host, "add_template").await;
    let added = until(SOON, || async { has(&host, "resources::templates") }).await;
    assert!(added, "{:?}", host.registered_ids());
    let emitted = until(SOON, || async { tools_events() > before }).await;
    assert!(emitted, "{:?}", host.events());
    m.shutdown().await;
}

#[tokio::test]
async fn a_failed_prompt_list_leaves_the_server_ready() {
    let host = FakeHost::new();
    let env = json!({"FAKE_MCP_PROMPTS": "1", "FAKE_MCP_PROMPTS_FAIL": "1"});
    let m = ready(&host, env).await;
    let v = server(&m, "fake").await;
    assert_eq!((&v["state"], &v["error"]), (&json!("ready"), &Value::Null));
    let error = v["list_errors"]["prompts"].as_str().unwrap_or_default();
    assert!(error.contains("prompts unavailable"), "{v}");
    assert!(!has_prompts(&host), "{:?}", host.registered_ids());
    let echo = host.call("mcp::fake::echo", json!({"text": "hi"})).await;
    assert_eq!(echo, Ok(hi()));
    m.shutdown().await;
}

#[tokio::test]
async fn a_failed_prompt_list_on_reconnect_keeps_the_prompts() {
    let host = FakeHost::new();
    let (_dir, state) = state_file("");
    let env = json!({"FAKE_MCP_PROMPTS": "1", "FAKE_MCP_STATE_FILE": state});
    let m = ready(&host, env).await;
    assert!(has(&host, "prompts::code_review"));
    let mark = host.ops().len();

    std::fs::write(&state, "FAKE_MCP_PROMPTS_FAIL=1\n").unwrap();
    m.reconnect("fake").await.unwrap();
    let failed = until(RESTART, || async {
        let v = server(&m, "fake").await;
        v["list_errors"]["prompts"].is_string() && v["state"] == "ready"
    })
    .await;
    let v = server(&m, "fake").await;
    assert!(failed, "{v}");
    assert_eq!(v["state"], "ready");
    assert!(has(&host, "prompts::code_review") && has(&host, "prompts::summary"));
    let ops = host.ops()[mark..].to_vec();
    assert!(!unregisters_a_prompt(&ops), "{ops:?}");

    std::fs::write(&state, "").unwrap();
    m.reconnect("fake").await.unwrap();
    let cleared = until(RESTART, || async {
        let v = server(&m, "fake").await;
        v["state"] == "ready" && v["list_errors"]["prompts"].is_null()
    })
    .await;
    assert!(cleared, "{}", server(&m, "fake").await);
    m.shutdown().await;
}

#[tokio::test]
async fn a_capability_that_disappears_unregisters_its_kind_and_a_disconnect_never_does() {
    let host = FakeHost::new();
    let (_dir, state) = state_file("");
    let env =
        json!({"FAKE_MCP_PROMPTS": "1", "FAKE_MCP_RESOURCES": "1", "FAKE_MCP_STATE_FILE": state});
    let m = ready(&host, env).await;
    let pid = async || {
        let out = host
            .call("mcp::fake::env", json!({"name": "FAKE_MCP_PID"}))
            .await;
        out.map(|v| v["content"][0]["text"].clone())
    };

    // A disconnect: the process comes back with the same lists.
    let first = pid().await.unwrap();
    let exit = host.call("mcp::fake::exit", json!({"code": 1})).await;
    assert!(exit.is_err(), "{exit:?}");
    let back = until(RESTART, || async { pid().await.is_ok_and(|p| p != first) }).await;
    assert!(back, "{}", server(&m, "fake").await);
    let ops = host.ops();
    let unregistered = ops.iter().any(|op| matches!(op, RegOp::Unregister(_)));
    assert!(!unregistered, "{ops:?}");

    // The next connect no longer advertises prompts.
    std::fs::write(&state, "FAKE_MCP_PROMPTS=0\n").unwrap();
    m.reconnect("fake").await.unwrap();
    let gone = until(RESTART, || async { !has_prompts(&host) }).await;
    assert!(gone, "{:?}", host.registered_ids());
    assert!(has(&host, "echo") && has(&host, "resources::read"));
    let v = server(&m, "fake").await;
    assert_eq!(
        (&v["capabilities"]["prompts"], &v["prompts"]),
        (&json!(false), &json!([])),
        "{v}"
    );
    m.shutdown().await;
}
