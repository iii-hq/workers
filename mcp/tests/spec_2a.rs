// Phase 2a — MCP 2025-06-18 spec method tests.
//
// In-process tests cover pure helpers (pagination, completion candidates,
// tool annotations, templated URI shape). Tests requiring a live engine
// session — log_message → notifications/message round-trip, subscribe →
// resources/updated, tools/call cancellation — are gated `#[ignore]` so
// `cargo test` stays green without an engine running. Run them explicitly
// with `cargo test -- --ignored` against a local engine.

use iii_mcp::prompts;
use iii_mcp::spec;
use serde_json::{Value, json};

// 120 dummy items, page=50 → page0, page1, page2 (50/50/20). Matches the
// task's pagination spec exactly. Cursor is round-tripped (server-opaque
// to clients but we decode internally).
#[test]
fn pagination_round_trip_120_items() {
    let items: Vec<i32> = (0..120).collect();
    let (page0, c0) = spec::paginate(&items, None, spec::PAGE_SIZE);
    assert_eq!(page0.len(), 50);
    assert_eq!(*page0[0], 0);
    assert_eq!(*page0[49], 49);
    let c0 = c0.expect("cursor after page 0");

    let (page1, c1) = spec::paginate(&items, Some(&c0), spec::PAGE_SIZE);
    assert_eq!(page1.len(), 50);
    assert_eq!(*page1[0], 50);
    assert_eq!(*page1[49], 99);
    let c1 = c1.expect("cursor after page 1");

    let (page2, c2) = spec::paginate(&items, Some(&c1), spec::PAGE_SIZE);
    assert_eq!(page2.len(), 20);
    assert_eq!(*page2[0], 100);
    assert_eq!(*page2[19], 119);
    assert!(c2.is_none(), "no cursor after final page");
}

#[test]
fn pagination_garbage_cursor_decodes_to_zero() {
    let items: Vec<i32> = (0..10).collect();
    let (page, _) = spec::paginate(&items, Some("not-a-real-cursor"), 50);
    assert_eq!(page.len(), 10);
}

#[test]
fn prompt_candidates_for_register_function_language() {
    let v = prompts::list_prompt_candidates("register-function", "language");
    assert_eq!(v, vec!["node".to_string(), "python".to_string()]);
}

#[test]
fn prompt_candidates_unknown_returns_empty() {
    let v = prompts::list_prompt_candidates("unknown", "language");
    assert!(v.is_empty());
}

// Mimics what the completion handler does for `ref/prompt` of
// `register-function.language` with value "p" (should match only "python").
// We exercise the filter directly because the engine path requires a session.
#[test]
fn completion_language_prefix_p_matches_python_only() {
    let candidates = prompts::list_prompt_candidates("register-function", "language");
    let filtered: Vec<&String> = candidates.iter().filter(|c| c.starts_with("p")).collect();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0], "python");
}

#[test]
fn completion_language_no_prefix_returns_both() {
    let candidates = prompts::list_prompt_candidates("register-function", "language");
    let filtered: Vec<&String> = candidates.iter().filter(|c| c.starts_with("")).collect();
    assert_eq!(filtered.len(), 2);
}

#[test]
fn resource_templates_list_returns_three() {
    let templates = spec::make_resource_templates();
    assert_eq!(templates.len(), 3);
    let uris: Vec<&str> = templates
        .iter()
        .map(|t| t.get("uriTemplate").and_then(|u| u.as_str()).unwrap())
        .collect();
    assert!(uris.contains(&"iii://function/{id}"));
    assert!(uris.contains(&"iii://worker/{id}"));
    assert!(uris.contains(&"iii://trigger/{id}"));
}

#[test]
fn make_tool_annotations_reads_all_hints() {
    let meta = json!({
        "mcp": {
            "title": "Pretty Title",
            "read_only_hint": true,
            "destructive_hint": false,
            "idempotent_hint": true,
            "open_world_hint": false
        }
    });
    let ann = spec::make_tool_annotations(&meta).expect("annotations present");
    assert_eq!(ann.title.as_deref(), Some("Pretty Title"));
    assert_eq!(ann.read_only_hint, Some(true));
    assert_eq!(ann.destructive_hint, Some(false));
    assert_eq!(ann.idempotent_hint, Some(true));
    assert_eq!(ann.open_world_hint, Some(false));
}

#[test]
fn make_tool_annotations_returns_none_when_no_keys() {
    let meta = json!({ "mcp": { "expose": true } });
    assert!(spec::make_tool_annotations(&meta).is_none());
}

#[test]
fn make_tool_annotations_returns_none_when_no_mcp_block() {
    let meta = json!({ "other": { "stuff": 1 } });
    assert!(spec::make_tool_annotations(&meta).is_none());
}

#[test]
fn log_level_string_round_trip() {
    assert_eq!(spec::level_from_str("debug"), Some(spec::LOG_DEBUG));
    assert_eq!(spec::level_from_str("info"), Some(spec::LOG_INFO));
    assert_eq!(spec::level_from_str("warning"), Some(spec::LOG_WARNING));
    assert_eq!(spec::level_from_str("emergency"), Some(spec::LOG_EMERGENCY));
    assert_eq!(spec::level_from_str("nonsense"), None);
}

#[test]
fn log_message_notification_includes_logger_when_set() {
    let n =
        spec::log_message_notification("warning", &json!({"msg": "x"}), Some("svc"));
    assert_eq!(n["method"], "notifications/message");
    assert_eq!(n["params"]["level"], "warning");
    assert_eq!(n["params"]["logger"], "svc");
}

#[test]
fn log_message_notification_omits_logger_when_unset() {
    let n = spec::log_message_notification("info", &json!({"msg": "x"}), None);
    assert!(n["params"].get("logger").is_none());
}

#[test]
fn progress_notification_shape() {
    let n = spec::progress_notification(&json!("t1"), 50.0, Some(100.0), Some("halfway"));
    assert_eq!(n["method"], "notifications/progress");
    assert_eq!(n["params"]["progressToken"], json!("t1"));
    assert_eq!(n["params"]["progress"], 50.0);
    assert_eq!(n["params"]["total"], 100.0);
    assert_eq!(n["params"]["message"], "halfway");
}

#[test]
fn resource_updated_notification_shape() {
    let n = spec::resource_updated_notification("iii://functions");
    assert_eq!(n["method"], "notifications/resources/updated");
    assert_eq!(n["params"]["uri"], "iii://functions");
}

#[test]
fn logging_set_level_updates_atomic() {
    use std::sync::atomic::{AtomicU8, Ordering};
    let level = AtomicU8::new(spec::LOG_INFO);
    spec::handle_logging_set_level(&level, Some(json!({"level": "warning"})))
        .expect("set warning");
    assert_eq!(level.load(Ordering::SeqCst), spec::LOG_WARNING);

    let err = spec::handle_logging_set_level(&level, Some(json!({"level": "garbage"})));
    assert!(err.is_err());
    assert_eq!(level.load(Ordering::SeqCst), spec::LOG_WARNING);
}

#[test]
fn subscribe_unsubscribe_round_trip() {
    use std::collections::HashSet;
    use std::sync::Mutex;
    let subs: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
    spec::handle_resources_subscribe(&subs, Some(json!({"uri": "iii://functions"})))
        .expect("subscribe");
    assert!(subs.lock().unwrap().contains("iii://functions"));
    spec::handle_resources_unsubscribe(&subs, Some(json!({"uri": "iii://functions"})))
        .expect("unsubscribe");
    assert!(!subs.lock().unwrap().contains("iii://functions"));
}

// ---------------------------------------------------------------------------
// Integration tests below this line require a live iii-engine. They are
// gated `#[ignore]` so plain `cargo test` stays green; run with
// `cargo test -- --ignored` after `iii-engine` is up on ws://localhost:49134.
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires live iii-engine on ws://localhost:49134"]
async fn live_logging_set_level_then_log_message() {
    use iii_mcp::handler::{ExposureConfig, McpHandler};
    use iii_sdk::{InitOptions, TriggerRequest, register_worker};

    let url = "ws://localhost:49134";
    let iii = register_worker(url, InitOptions::default());
    let handler = std::sync::Arc::new(McpHandler::new(
        iii.clone(),
        url.to_string(),
        ExposureConfig::new(false, false, None),
    ));

    // Bring handler to initialized state.
    handler
        .handle(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
        .await;
    handler
        .handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await;

    // setLevel info, then a `warning` log_message — should fire.
    handler
        .handle(json!({"jsonrpc":"2.0","id":2,"method":"logging/setLevel","params":{"level":"info"}}))
        .await;
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "mcp::log_message".into(),
            payload: json!({"level": "warning", "data": {"hello": "world"}}),
            action: None,
            timeout_ms: None,
        })
        .await
        .expect("log_message trigger");

    // Drain the notification — should be a notifications/message at warning.
    let mut got_warning = false;
    for _ in 0..20 {
        if let Some(n) = handler.take_notification().await {
            let v: Value = serde_json::from_str(&n).unwrap();
            if v["method"] == "notifications/message" && v["params"]["level"] == "warning" {
                got_warning = true;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(got_warning, "expected notifications/message at warning level");

    // Bump level to error, fire warning → should be filtered.
    handler
        .handle(json!({"jsonrpc":"2.0","id":3,"method":"logging/setLevel","params":{"level":"error"}}))
        .await;
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "mcp::log_message".into(),
            payload: json!({"level": "debug", "data": {}}),
            action: None,
            timeout_ms: None,
        })
        .await;

    let mut filtered = true;
    for _ in 0..6 {
        if let Some(n) = handler.take_notification().await {
            let v: Value = serde_json::from_str(&n).unwrap();
            if v["method"] == "notifications/message" {
                filtered = false;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(filtered, "debug message should not fire when level=error");
}

#[tokio::test]
#[ignore = "requires live iii-engine on ws://localhost:49134"]
async fn live_progress_token_round_trip() {
    use iii_mcp::handler::{ExposureConfig, McpHandler};
    use iii_sdk::{InitOptions, TriggerRequest, register_worker};

    let url = "ws://localhost:49134";
    let iii = register_worker(url, InitOptions::default());
    let handler = std::sync::Arc::new(McpHandler::new(
        iii.clone(),
        url.to_string(),
        ExposureConfig::new(false, false, None),
    ));
    handler
        .handle(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
        .await;
    handler
        .handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await;

    let _ = iii
        .trigger(TriggerRequest {
            function_id: "mcp::progress".into(),
            payload: json!({"token": "t1", "progress": 50.0}),
            action: None,
            timeout_ms: None,
        })
        .await
        .expect("progress trigger");

    let mut got = false;
    for _ in 0..20 {
        if let Some(n) = handler.take_notification().await {
            let v: Value = serde_json::from_str(&n).unwrap();
            if v["method"] == "notifications/progress"
                && v["params"]["progressToken"] == "t1"
                && v["params"]["progress"] == 50.0
            {
                got = true;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(got, "expected notifications/progress for token t1");
}

#[tokio::test]
#[ignore = "requires live iii-engine on ws://localhost:49134"]
async fn live_subscribe_function_added_emits_updated() {
    use iii_mcp::handler::{ExposureConfig, McpHandler};
    use iii_sdk::{InitOptions, RegisterFunctionMessage, register_worker};

    let url = "ws://localhost:49134";
    let iii = register_worker(url, InitOptions::default());
    let handler = std::sync::Arc::new(McpHandler::new(
        iii.clone(),
        url.to_string(),
        ExposureConfig::new(false, false, None),
    ));
    handler
        .handle(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
        .await;
    handler
        .handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await;
    handler
        .handle(json!({
            "jsonrpc":"2.0","id":2,"method":"resources/subscribe",
            "params":{"uri":"iii://functions"}
        }))
        .await;

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "demo::ping".into(),
            description: Some("test".into()),
            request_format: None,
            response_format: None,
            metadata: None,
            invocation: None,
        },
        |_input: Value| async move { Ok(json!({"ok": true})) },
    );

    let mut got = false;
    for _ in 0..40 {
        if let Some(n) = handler.take_notification().await {
            let v: Value = serde_json::from_str(&n).unwrap();
            if v["method"] == "notifications/resources/updated"
                && v["params"]["uri"] == "iii://functions"
            {
                got = true;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(got, "expected notifications/resources/updated for iii://functions");
}

#[tokio::test]
#[ignore = "requires live iii-engine on ws://localhost:49134"]
async fn live_tools_call_cancelled() {
    use iii_mcp::handler::{ExposureConfig, McpHandler};
    use iii_sdk::{InitOptions, RegisterFunctionMessage, register_worker};

    let url = "ws://localhost:49134";
    let iii = register_worker(url, InitOptions::default());

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "demo::slow".into(),
            description: Some("slow handler".into()),
            request_format: None,
            response_format: None,
            metadata: Some(json!({"mcp": {"expose": true}})),
            invocation: None,
        },
        |_input: Value| async move {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            Ok(json!({"ok": true}))
        },
    );

    let handler = std::sync::Arc::new(McpHandler::new(
        iii.clone(),
        url.to_string(),
        ExposureConfig::new(false, true, None),
    ));
    handler
        .handle(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
        .await;
    handler
        .handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await;

    let h2 = handler.clone();
    let call = tokio::spawn(async move {
        h2.handle(json!({
            "jsonrpc":"2.0","id":42,"method":"tools/call",
            "params":{"name":"demo__slow","arguments":{}}
        }))
        .await
    });

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    handler
        .handle(json!({
            "jsonrpc":"2.0","method":"notifications/cancelled",
            "params":{"requestId": 42}
        }))
        .await;

    let resp = call.await.expect("join").expect("response");
    let v: Value = resp;
    let result = &v["result"];
    assert_eq!(result["isError"], true);
    let text = result["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        text.to_lowercase().contains("cancel"),
        "expected cancel message, got {}",
        text
    );
}
