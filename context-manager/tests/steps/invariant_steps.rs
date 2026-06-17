//! Structural-invariant assertions over the response messages
//! (context-manager.md § Structural invariants). These are the checks
//! that prove pruned/compacted output is still accepted by providers.

use std::collections::HashSet;

use cucumber::then;
use serde_json::Value;

use crate::common::world::ContextWorld;
use crate::steps::common_steps::response_or_panic;

fn response_messages(world: &ContextWorld) -> Vec<Value> {
    response_or_panic(world)
        .get("messages")
        .and_then(Value::as_array)
        .expect("response has a messages array")
        .clone()
}

/// Every `function_result` (message-level or inline block) in the
/// returned context must have its `function_call` present earlier in
/// the same context — providers reject orphaned results.
#[then("call/result pairing is intact in the response messages")]
async fn pairing_intact(world: &mut ContextWorld) {
    if world.soft_skipped {
        return;
    }
    let messages = response_messages(world);
    let mut seen_calls: HashSet<String> = HashSet::new();
    for (idx, message) in messages.iter().enumerate() {
        for block in message
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match block.get("type").and_then(Value::as_str) {
                Some("function_call") => {
                    if let Some(id) = block.get("id").and_then(Value::as_str) {
                        seen_calls.insert(id.to_string());
                    }
                }
                Some("function_result") => {
                    let call_id = block
                        .get("function_call_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    assert!(
                        seen_calls.contains(call_id),
                        "message {idx}: inline function_result for `{call_id}` has no \
                         preceding function_call in the returned context"
                    );
                }
                _ => {}
            }
        }
        if message.get("role").and_then(Value::as_str) == Some("function_result") {
            let call_id = message
                .get("function_call_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            assert!(
                seen_calls.contains(call_id),
                "message {idx}: function_result for `{call_id}` has no preceding \
                 function_call in the returned context"
            );
        }
    }
}

#[then("the response messages contain no custom roles")]
async fn no_custom_roles(world: &mut ContextWorld) {
    if world.soft_skipped {
        return;
    }
    for (idx, message) in response_messages(world).iter().enumerate() {
        assert_ne!(
            message.get("role").and_then(Value::as_str),
            Some("custom"),
            "message {idx} is a custom message — it must never reach the model"
        );
    }
}

#[then("the response messages equal the request history")]
async fn equals_request(world: &mut ContextWorld) {
    if world.soft_skipped {
        return;
    }
    let messages = response_messages(world);
    assert_eq!(
        Value::Array(messages),
        Value::Array(world.messages.clone()),
        "response messages must be the request history, untouched"
    );
}

#[then("the response messages have as many messages as the request")]
async fn same_count(world: &mut ContextWorld) {
    if world.soft_skipped {
        return;
    }
    let messages = response_messages(world);
    assert_eq!(messages.len(), world.messages.len());
}

/// context::assemble must never strip the model-facing context down to
/// nothing: the provider rejects an empty messages array ("messages: at
/// least one message is required").
#[then("the response messages are not empty")]
async fn messages_not_empty(world: &mut ContextWorld) {
    if world.soft_skipped {
        return;
    }
    let messages = response_messages(world);
    assert!(
        !messages.is_empty(),
        "context::assemble returned an empty messages array — the provider \
         rejects it (\"messages: at least one message is required\")"
    );
}

#[then(regex = r#"^response message (\d+) text is "([^"]*)"$"#)]
async fn message_text_is(world: &mut ContextWorld, idx: usize, expected: String) {
    if world.soft_skipped {
        return;
    }
    let messages = response_messages(world);
    let message = messages
        .get(idx)
        .unwrap_or_else(|| panic!("no response message at index {idx}"));
    let text = message
        .get("content")
        .and_then(Value::as_array)
        .and_then(|blocks| blocks.first())
        .and_then(|block| block.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert_eq!(text, expected, "message {idx} text mismatch");
}

#[then(regex = r#"^response message (\d+) text has (\d+) chars$"#)]
async fn message_text_len(world: &mut ContextWorld, idx: usize, expected: usize) {
    if world.soft_skipped {
        return;
    }
    let messages = response_messages(world);
    let message = messages
        .get(idx)
        .unwrap_or_else(|| panic!("no response message at index {idx}"));
    let text = message
        .get("content")
        .and_then(Value::as_array)
        .and_then(|blocks| blocks.first())
        .and_then(|block| block.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert_eq!(text.len(), expected, "message {idx} text length mismatch");
}

#[then(regex = r#"^every response message keeps its function_call_id$"#)]
async fn keeps_call_ids(world: &mut ContextWorld) {
    if world.soft_skipped {
        return;
    }
    for (idx, message) in response_messages(world).iter().enumerate() {
        if message.get("role").and_then(Value::as_str) == Some("function_result") {
            let call_id = message.get("function_call_id").and_then(Value::as_str);
            assert!(
                call_id.is_some_and(|id| !id.is_empty()),
                "message {idx} lost its function_call_id"
            );
        }
    }
}

#[then(regex = r#"^the response messages start at request message (\d+)$"#)]
async fn starts_at_request_index(world: &mut ContextWorld, idx: usize) {
    if world.soft_skipped {
        return;
    }
    let messages = response_messages(world);
    let expected_tail: Vec<Value> = world.messages[idx..].to_vec();
    assert_eq!(
        Value::Array(messages),
        Value::Array(expected_tail),
        "response messages must be exactly the request messages from index {idx} on"
    );
}
