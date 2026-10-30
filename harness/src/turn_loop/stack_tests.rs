//! Turn-loop steps run against a fake engine on the SDK's default-size thread stack.
use super::*;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use iii_sdk::{register_worker, InitOptions, RegisterFunction};
use tokio::sync::{mpsc, oneshot, RwLock};

#[test]
fn completion_future_keeps_finalization_out_of_its_inline_state() {
    fn future_size<F: std::future::Future>(
        _: impl FnOnce(&'static Deps, &'static SessionClient, &'static mut TurnRecord, Value) -> F,
    ) -> usize {
        std::mem::size_of::<F>()
    }
    let bytes = future_size(complete_validated);
    assert!(
        bytes <= 8 * 1024,
        "completion future is {bytes} bytes; keep finalization futures boxed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generated_child_completes_on_a_default_thread_stack() {
    complete_child(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generated_child_survives_a_parent_read_failure_on_the_sdk_stack() {
    complete_child(false).await;
}

async fn complete_child(parent_prompt_present: bool) {
    // The cache is process-wide; parallel cases must not share a prompt.
    let parent_prompt = if parent_prompt_present {
        "parent prompt"
    } else {
        "missing parent prompt"
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observed = calls.clone();
    let (ready_tx, ready_rx) = oneshot::channel();
    let (push, mut pushed) = mpsc::unbounded_channel::<Value>();
    let (done_tx, done_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
        ready_tx.send(()).unwrap();
        let mut done_tx = Some(done_tx);
        loop {
            let frame = tokio::select! {
                Some(message) = pushed.recv() => {
                    socket.send(tokio_tungstenite::tungstenite::Message::Text(message.to_string().into())).await.unwrap();
                    continue;
                }
                frame = socket.next() => {
                    let Some(Ok(frame)) = frame else { break };
                    frame
                }
            };
            let Ok(text) = frame.to_text() else { continue };
            let Ok(message) = serde_json::from_str::<Value>(text) else {
                continue;
            };
            if message["type"] == "invocationresult" {
                done_tx.take().unwrap().send(message).unwrap();
                continue;
            }
            if message["type"] != "invokefunction" {
                continue;
            }
            let function = message["function_id"].as_str().unwrap();
            let data = &message["data"];
            observed
                .lock()
                .unwrap()
                .push((function.to_string(), data.clone()));
            let result = match function {
                "engine::workers::register" => json!({"success": true}),
                "state::get" | "harness::state::get"
                    if data["scope"] == "harness_turn" && data["key"] == "parent" =>
                {
                    json!({
                        "session_id": "parent", "turn_id": "t_parent", "status": "completed",
                        "step": 0, "turn_count": 1, "depth": 0,
                        "options": {"model": "fake", "max_turns": 16,
                            "system_prompt": {"$ref": crate::state::prompt_digest(parent_prompt)},
                            "output": {"type": "text"},
                            "functions": {"allow": ["*"], "deny": [], "expose": "agent_trigger"}
                        }, "created_at": 1, "updated_at": 1,
                        "context_snapshot": {
                            "session_id": "parent", "turn_id": "t_parent", "step": 0,
                            "model": "fake", "usable": 1000, "effective_max_output_tokens": 100,
                            "total": 10, "free": 990, "compacted": false, "timestamp": 1,
                            "categories": {"system_prompt": 0, "tools": 0, "hook_guidance": 0, "overhead": 0,
                                "messages": {"user": 10, "assistant": 0, "function_result": 0, "custom": 0}}
                        }
                    })
                }
                "state::get" | "harness::state::get"
                    if data["scope"] == "harness_prompt" && parent_prompt_present =>
                {
                    json!({"body": parent_prompt, "created_at": 1})
                }
                "state::get" | "harness::state::get" => Value::Null,
                "state::list" | "harness::state::list" => json!([]),
                "state::list_keys" => json!({"keys": []}),
                "state::set" | "harness::state::set" => json!({}),
                "session::get" => json!({"meta": {
                    "session_id": data["session_id"], "metadata": {}
                }}),
                "session::messages" => json!({"messages": []}),
                "session::set-status" => json!({"ok": true}),
                "session::append" => json!({"entry_id": data["entry_id"]}),
                _ => panic!("unexpected RPC: {function}"),
            };
            if function.ends_with("state::get")
                && data["scope"] == "harness_turn"
                && data["key"] == "parent"
            {
                let mut hydrated = result.clone();
                hydrated["options"]["system_prompt"] = json!(parent_prompt);
                serde_json::from_value::<TurnRecord>(hydrated).unwrap();
            }
            if !message["invocation_id"].is_null() {
                let reply = json!({
                    "type": "invocationresult", "invocation_id": message["invocation_id"],
                    "function_id": function, "result": result
                });
                if socket
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        reply.to_string().into(),
                    ))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    });
    let iii = Arc::new(register_worker(&url, InitOptions::default()));
    ready_rx.await.unwrap();
    let cfg = Arc::new(crate::config::WorkerConfig {
        session_timeout_ms: 2_000,
        ..Default::default()
    });
    let deps = Deps::new(
        iii.clone(),
        Arc::new(RwLock::new(cfg.clone())),
        crate::discovery::new_cell(),
        crate::skills::new_cell(),
        crate::events::TurnEvents::register(&iii),
        crate::hooks::HookRegistry::register(&iii),
    );
    let record: TurnRecord = serde_json::from_value(json!({
        "session_id": "child", "turn_id": "t_child", "status": "running",
        "step": 0, "turn_count": 0, "depth": 1,
        "parent": {"session_id": "parent", "turn_id": "t_parent", "function_call_id": "spawn"},
        "options": {"model": "fake", "max_turns": 16}, "created_at": 1, "updated_at": 1
    }))
    .unwrap();
    let payload = serde_json::from_value(json!({
        "session_id": "child", "turn_id": "t_child", "step": 0, "depth": 1
    }))
    .unwrap();
    let mut message = empty_assistant("fake", "fake");
    message.content = vec![ContentBlock::text("done")];
    let generated = Box::new(GeneratedStep {
        payload,
        cfg,
        session: deps.session().await,
        record,
        strategy: crate::contract::OutputStrategy::Text,
        outcome: crate::clients::router::ChatOutcome {
            message,
            ok: true,
            stop_reason: None,
            error: None,
        },
        durable_abort: false,
        guard: deps.locks.guard("child").await,
        replayed: false,
    });
    let deps = Arc::new(deps);
    let generated = Arc::new(Mutex::new(Some(generated)));
    iii.register_function(
        "test::finish",
        RegisterFunction::new_async(move |_: Value| {
            let deps = deps.clone();
            let generated = generated.lock().unwrap().take().unwrap();
            async move {
                Box::pin(finish_step(&deps, generated))
                    .await
                    .map_err(iii_sdk::Error::from)
            }
        }),
    );
    push.send(json!({
        "type": "invokefunction", "function_id": "test::finish", "data": {},
        "invocation_id": uuid::Uuid::new_v4().to_string()
    }))
    .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), done_rx).await;
    iii.shutdown();
    server.abort();
    let result = result.unwrap().unwrap();
    assert_eq!(result["result"]["status"], "completed", "{result}");
    let calls = calls.lock().unwrap();
    assert!(calls.iter().any(|(function, data)| {
        function.ends_with("state::set")
            && data["scope"] == "harness_turn"
            && data["value"]["status"] == "completed"
    }));
    let parent_reads = calls
        .iter()
        .filter(|(function, data)| {
            function.ends_with("state::get")
                && data["scope"] == "harness_turn"
                && data["key"] == "parent"
        })
        .count();
    assert_eq!(
        parent_reads,
        if parent_prompt_present { 1 } else { 2 },
        "a failed pre-check must reach the parent resolver's error handling"
    );
}

/// A redelivered step whose reply already dispatched a call (checkpoint still
/// `Triggered`: the worker died before the result) must not regenerate: the
/// call gets `interrupted_result` and its handler is not invoked again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redelivered_step_with_a_triggered_call_closes_it_instead_of_regenerating() {
    let (result, calls, turn) = redeliver(
        json!({"call_hold": {"state": "triggered", "function_id": "test::hold"}}),
        json!([hold_reply()]),
    )
    .await;
    let names: Vec<&str> = calls.iter().map(|(f, _)| f.as_str()).collect();
    assert!(
        !names
            .iter()
            .any(|f| *f == "router::chat" || f.starts_with("context::")),
        "the step regenerated: {names:?}"
    );
    assert!(!names.contains(&"test::hold"), "the handler ran again");
    let result = result.expect("the step finishes with the stored reply");
    assert_eq!(result.next_step, Some(2));
    assert!(
        calls.iter().any(|(f, data)| f == "session::append"
            && data.to_string().contains("call_hold")
            && data.to_string().contains("\"interrupted\"")),
        "the call closes with interrupted_result: {calls:?}"
    );
    assert_eq!(turn["calls"]["call_hold"]["state"], "done");
    assert_eq!(turn["turn_count"], 2, "no new generation is counted");
}

/// With no dispatched call (none checkpointed yet, or all done) the
/// redelivered step regenerates as before.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redelivered_step_without_a_triggered_call_regenerates() {
    for calls in [
        json!({}),
        json!({"call_hold": {"state": "done", "function_id": "test::hold"}}),
    ] {
        let (_, observed, _) = redeliver(calls, json!([hold_reply()])).await;
        assert!(
            observed.iter().any(|(f, _)| f == "context::assemble"),
            "{observed:?}"
        );
    }
}

/// A redelivered text-only step regenerates over its billed reply, and keeps
/// that generation's usage first: metrics then count both generations, as
/// the running session cost does.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redelivered_text_only_step_keeps_the_billed_generation_it_regenerates_over() {
    let (_, calls, _) = redeliver(json!({}), json!([text_reply(0.25)])).await;
    let appends: Vec<&Value> = calls
        .iter()
        .filter(|(f, _)| f == "session::append")
        .map(|(_, data)| data)
        .collect();
    let kept = appends
        .iter()
        .position(|data| data["custom"]["custom_type"] == crate::turn_loop::SUPERSEDED_GENERATION)
        .unwrap_or_else(|| panic!("the billed generation is not kept: {appends:?}"));
    let placeholder = appends
        .iter()
        .position(|data| data["entry_id"] == "e_t1_1_assistant")
        .expect("the step regenerates");
    assert!(kept < placeholder, "kept after the reply was overwritten");
    let data = &appends[kept]["custom"]["data"];
    assert_eq!(data["usage"]["cost_usd"], 0.25);

    // The regenerated reply overwrites the stored one; metrics read both.
    let entries: Vec<crate::clients::LoadedEntry> = serde_json::from_value(json!([
        {"entry_id": appends[kept]["entry_id"], "custom": {
            "custom_type": crate::turn_loop::SUPERSEDED_GENERATION, "data": data}},
        text_reply(0.5)
    ]))
    .unwrap();
    let totals = crate::functions::metrics::fold_totals(entries);
    assert_eq!(totals.cost_usd, Some(0.75), "the running cost billed both");
    assert_eq!(totals.turns, 1);
}

fn hold_reply() -> Value {
    json!({"entry_id": "e_t1_1_assistant", "message": {
        "role": "assistant", "stop_reason": "function_call",
        "model": "fake", "provider": "fake", "timestamp": 2,
        "usage": {"input": 10, "output": 2, "cost_usd": 0.1},
        "content": [{"type": "function_call", "id": "call_hold",
            "function_id": "agent_trigger",
            "arguments": {"function": "test::hold", "payload": {}}}]}})
}

fn text_reply(cost: f64) -> Value {
    json!({"entry_id": "e_t1_1_assistant", "message": {
        "role": "assistant", "stop_reason": "end",
        "model": "fake", "provider": "fake", "timestamp": 2,
        "usage": {"input": 10, "output": 2, "cost_usd": cost},
        "content": [{"type": "text", "text": "done"}]}})
}

/// Run step 1 of turn `t1` again on a fake engine, as the queue redelivers it
/// after the worker died: the record as the first delivery persisted it (the
/// generation counted, `calls` checkpointed) and the entries that step left
/// after the user message (`stored`). Returns the outcome, every RPC, and the
/// last record.
async fn redeliver(
    checkpoints: Value,
    stored: Value,
) -> (
    Result<TurnStepResult, HarnessError>,
    Vec<(String, Value)>,
    Value,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let calls = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
    let observed = calls.clone();
    let (ready_tx, ready_rx) = oneshot::channel();
    let turn = Arc::new(Mutex::new(json!({
        "session_id": "s", "turn_id": "t1", "status": "running",
        "step": 1, "turn_count": 2, "depth": 0,
        "options": {"model": "fake", "max_turns": 16},
        "calls": checkpoints,
        "created_at": 1, "updated_at": 1
    })));
    let stored_turn = turn.clone();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
        ready_tx.send(()).unwrap();
        while let Some(Ok(frame)) = socket.next().await {
            let Ok(text) = frame.to_text() else { continue };
            let Ok(message) = serde_json::from_str::<Value>(text) else {
                continue;
            };
            if message["type"] != "invokefunction" {
                continue;
            }
            let function = message["function_id"].as_str().unwrap().to_string();
            let data = message["data"].clone();
            observed
                .lock()
                .unwrap()
                .push((function.clone(), data.clone()));
            let result = match function.as_str() {
                "engine::workers::register" => json!({"success": true}),
                "state::get" | "harness::state::get" if data["scope"] == "harness_turn" => {
                    stored_turn.lock().unwrap().clone()
                }
                "state::set" if data["scope"] == "harness_turn" => {
                    *stored_turn.lock().unwrap() = data["value"].clone();
                    json!({})
                }
                "state::list" | "harness::state::list" => json!([]),
                "state::list_keys" => json!({"keys": []}),
                "session::get" => json!({"meta": {"session_id": "s", "metadata": {}}}),
                "session::messages" => {
                    let mut messages = vec![json!({"entry_id": "e_user", "message": {
                        "role": "user", "timestamp": 1,
                        "content": [{"type": "text", "text": "hold"}]}})];
                    messages.extend(stored.as_array().unwrap().iter().cloned());
                    json!({ "messages": messages })
                }
                "context::assemble" => json!({
                    "messages": [{"role": "user", "timestamp": 1,
                        "content": [{"type": "text", "text": "hold"}]}],
                    "token_count": 10, "usable": 1000, "effective_max_output_tokens": 100
                }),
                "session::set-status" => json!({"ok": true}),
                "session::append" => json!({"entry_id": data["entry_id"]}),
                "state::set" => json!({}),
                _ => Value::Null,
            };
            if !message["invocation_id"].is_null() {
                let reply = json!({
                    "type": "invocationresult", "invocation_id": message["invocation_id"],
                    "function_id": function, "result": result
                });
                let frame = tokio_tungstenite::tungstenite::Message::Text(reply.to_string().into());
                if socket.send(frame).await.is_err() {
                    break;
                }
            }
        }
    });
    let iii = Arc::new(register_worker(&url, InitOptions::default()));
    ready_rx.await.unwrap();
    let cfg = Arc::new(crate::config::WorkerConfig {
        session_timeout_ms: 2_000,
        ..Default::default()
    });
    let deps = Deps::new(
        iii.clone(),
        Arc::new(RwLock::new(cfg)),
        crate::discovery::new_cell(),
        crate::skills::new_cell(),
        crate::events::TurnEvents::register(&iii),
        crate::hooks::HookRegistry::register(&iii),
    );
    let payload =
        serde_json::from_value(json!({"session_id": "s", "turn_id": "t1", "step": 1})).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(10), run_step(&deps, payload)).await;
    iii.shutdown();
    server.abort();
    let calls = calls.lock().unwrap().clone();
    let turn = turn.lock().unwrap().clone();
    (result.unwrap(), calls, turn)
}
