//! Event -> AgentEvent translation tests.

use grok::grok::events_types::{
    CommandExecutionItem, ItemStatus, McpError, McpToolCallItem, ThreadEvent, ThreadItemDetails,
    Usage, WebSearchItem,
};
use grok::map;
use serde_json::{json, Value};

#[test]
fn function_ids_for_builtin_items() {
    assert_eq!(
        map::function_id(&ThreadItemDetails::CommandExecution(cmd(
            "ls",
            0,
            ItemStatus::Completed
        ))),
        "grok::shell"
    );
    assert_eq!(
        map::function_id(&ThreadItemDetails::WebSearch(WebSearchItem {
            query: "q".into()
        })),
        "grok::web_search"
    );
    assert_eq!(
        map::function_id(&ThreadItemDetails::McpToolCall(mcp(
            "github",
            "create_issue",
            false
        ))),
        "github::create_issue"
    );
}

#[test]
fn exec_classification() {
    assert!(map::is_exec_item(&ThreadItemDetails::CommandExecution(
        cmd("x", 0, ItemStatus::Completed)
    )));
    assert!(!map::is_exec_item(&ThreadItemDetails::AgentMessage {
        text: "hi".into()
    }));
}

#[test]
fn error_classification() {
    assert!(map::is_error_item(&ThreadItemDetails::CommandExecution(
        cmd("x", 1, ItemStatus::Failed)
    )));
    assert!(map::is_error_item(&ThreadItemDetails::CommandExecution(
        cmd("x", 2, ItemStatus::Completed)
    )));
    assert!(!map::is_error_item(&ThreadItemDetails::CommandExecution(
        cmd("x", 0, ItemStatus::Completed)
    )));
    assert!(map::is_error_item(&ThreadItemDetails::McpToolCall(mcp(
        "s", "t", true
    ))));
}

#[test]
fn command_args_and_result() {
    let d = ThreadItemDetails::CommandExecution(CommandExecutionItem {
        command: "ls -la".into(),
        aggregated_output: "total 0".into(),
        exit_code: Some(0),
        status: ItemStatus::Completed,
    });
    assert_eq!(map::args_for(&d), json!({ "command": "ls -la" }));
    assert_eq!(
        map::result_content(&d),
        json!([{ "type": "text", "text": "total 0" }])
    );
}

#[test]
fn usage_maps_and_defaults_zero() {
    let u = Usage {
        input_tokens: 10,
        cached_input_tokens: 100,
        output_tokens: 5,
        reasoning_output_tokens: 7,
    };
    let w = map::map_usage(&u);
    assert_eq!(w.input_tokens, 10);
    assert_eq!(w.cache_read_tokens, 100);
    assert_eq!(w.reasoning_tokens, 7);
    // absent fields default to 0 via serde default
    let z: Usage = serde_json::from_value(json!({})).unwrap();
    let wz = map::map_usage(&z);
    assert_eq!(wz.input_tokens, 0);
    assert_eq!(wz.cache_read_tokens, 0);
}

#[test]
fn parses_a_full_turn_jsonl_sequence() {
    let lines = [
        json!({ "type": "thread.started", "thread_id": "th-1" }),
        json!({ "type": "turn.started" }),
        json!({ "type": "item.started", "item": { "id": "i1", "type": "command_execution", "command": "ls", "aggregated_output": "", "status": "in_progress" } }),
        json!({ "type": "item.completed", "item": { "id": "i1", "type": "command_execution", "command": "ls", "aggregated_output": "files", "exit_code": 0, "status": "completed" } }),
        json!({ "type": "item.completed", "item": { "id": "i2", "type": "agent_message", "text": "done" } }),
        json!({ "type": "turn.completed", "usage": { "input_tokens": 5, "cached_input_tokens": 0, "output_tokens": 2, "reasoning_output_tokens": 0 } }),
    ];
    for line in lines {
        let ev: ThreadEvent =
            serde_json::from_value(line.clone()).unwrap_or_else(|e| panic!("parse {line}: {e}"));
        assert!(
            !matches!(ev, ThreadEvent::Unknown),
            "unexpected Unknown for {line}"
        );
    }
}

#[test]
fn unknown_event_and_item_types_fall_through() {
    let ev: ThreadEvent = serde_json::from_value(json!({ "type": "future.event" })).unwrap();
    assert!(matches!(ev, ThreadEvent::Unknown));
    let item: ThreadEvent = serde_json::from_value(
        json!({ "type": "item.completed", "item": { "id": "x", "type": "future_item" } }),
    )
    .unwrap();
    match item {
        ThreadEvent::ItemCompleted(e) => {
            assert!(matches!(e.item.details, ThreadItemDetails::Unknown))
        }
        _ => panic!("expected ItemCompleted"),
    }
}

fn cmd(command: &str, exit: i32, status: ItemStatus) -> CommandExecutionItem {
    CommandExecutionItem {
        command: command.into(),
        aggregated_output: String::new(),
        exit_code: Some(exit),
        status,
    }
}

fn mcp(server: &str, tool: &str, err: bool) -> McpToolCallItem {
    McpToolCallItem {
        server: server.into(),
        tool: tool.into(),
        arguments: Value::Null,
        result: Value::Null,
        error: if err {
            Some(McpError {
                message: "boom".into(),
            })
        } else {
            None
        },
        status: if err {
            ItemStatus::Failed
        } else {
            ItemStatus::Completed
        },
    }
}
