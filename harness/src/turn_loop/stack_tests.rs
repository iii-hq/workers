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
                            "functions": {"allow": ["*"], "deny": [], "expose": "agent_call"}
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
/// `Dispatched`: the worker died before the result) must not regenerate: the
/// call gets `interrupted_result` and its handler is not invoked again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redelivered_step_with_a_dispatched_call_closes_it_and_does_not_regenerate() {
    let (result, calls, turn) = redeliver(
        json!({"call_hold": {"state": "dispatched", "function_id": "test::hold"}}),
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
async fn a_redelivered_step_without_a_dispatched_call_regenerates() {
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

/// A third delivery whose second generation billed exactly what the first
/// did is still kept: the kept entry is named by the reply's revision, which
/// every generation's writes move, and the reply's usage is cleared once kept
/// (a delivery that dies before the clear keeps it again under the same id).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_generation_billing_what_the_one_before_did_is_still_kept() {
    let first = json!({"usage": {"input": 10, "output": 2, "cost_usd": 0.25},
        "model": "fake", "provider": "fake", "timestamp": 2});
    let (_, calls, _) = redeliver(
        json!({}),
        json!([
            {"entry_id": "e_t1_1_superseded_3", "custom": {
                "custom_type": crate::turn_loop::SUPERSEDED_GENERATION, "data": first}},
            text_reply(0.25)
        ]),
    )
    .await;
    let writes: Vec<(&str, &Value)> = calls
        .iter()
        .filter(|(f, _)| f == "session::append" || f == "session::update-message")
        .map(|(f, data)| (f.as_str(), data))
        .collect();
    let kept = writes
        .iter()
        .position(|(_, data)| data["entry_id"] == "e_t1_1_superseded_7")
        .unwrap_or_else(|| panic!("not kept under the reply's revision: {writes:?}"));
    assert_eq!(writes[kept].1["custom"]["data"], first);
    let cleared = writes
        .iter()
        .position(|(f, data)| {
            *f == "session::update-message"
                && data["entry_id"] == "e_t1_1_assistant"
                && data["usage"] == json!({})
        })
        .unwrap_or_else(|| panic!("the kept usage stays on the reply: {writes:?}"));
    assert!(kept < cleared, "cleared before it was kept");
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

/// `waiting for <model>` used to stay up with no model request in flight:
/// through the tools, the enqueue retries and the wait in the queue.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_step_running_its_calls_stops_claiming_to_wait_for_the_model() {
    let (result, calls, _) = generate(hold_reply()["message"].clone()).await;
    assert_eq!(result.expect("the step advances").next_step, Some(2));
    assert_eq!(
        phases(&calls),
        [
            "preparing context",
            "waiting for fake",
            "running tools",
            "test::hold",
            "queueing next step",
            "harness::turn"
        ]
    );
}

/// A live phase written after the terminal projection would leave a finished
/// session `working`: nothing writes its status again until the next turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_step_ending_the_turn_writes_no_live_phase_after_its_projection() {
    let (result, calls, _) = generate(text_reply(0.25)["message"].clone()).await;
    assert_eq!(result.unwrap().status, TurnStatus::Completed);
    assert_eq!(
        phases(&calls),
        ["preparing context", "waiting for fake", "done"]
    );

    let mut reply = hold_reply()["message"].clone();
    reply["content"].as_array_mut().unwrap().push(json!({
        "type": "function_call", "id": "call_submit",
        "function_id": "submit_result", "arguments": {"ok": true}}));
    let (result, calls, _) = generate(reply).await;
    assert_eq!(result.unwrap().status, TurnStatus::Completed);
    assert_eq!(
        phases(&calls),
        [
            "preparing context",
            "waiting for fake",
            "running tools",
            "test::hold",
            "done"
        ]
    );
}

/// A stop mid-generation still runs the post_generate hooks: writing the next
/// phase over its `stopping` ack would show the stop as not taken.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stopped_generation_keeps_its_stopping_ack() {
    let mut reply = text_reply(0.25)["message"].clone();
    reply["stop_reason"] = json!("aborted");
    let (result, calls, _) = generate(reply).await;
    assert_eq!(result.unwrap().status, TurnStatus::Cancelled);
    assert_eq!(
        phases(&calls),
        ["preparing context", "waiting for fake", "stopped"]
    );
}

/// A held call runs nothing until someone decides, possibly hours later:
/// `running tools` pulsed a sub-agent chip that was waiting on a person.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_step_parked_on_a_held_call_stops_claiming_to_run_tools() {
    let (result, calls, turn) = generate_observed(
        hold_reply()["message"].clone(),
        json!({"harness::hook::on-pre-call": {"decision": "hold", "held_by": "test::gate"}}),
    )
    .await;
    assert_eq!(result.unwrap().status, TurnStatus::AwaitingFunctions);
    assert_eq!(turn["calls"]["call_hold"]["state"], "pending");
    assert_eq!(
        phases(&calls),
        [
            "preparing context",
            "waiting for fake",
            "running tools",
            "harness::hook::on-pre-call",
            "waiting for pending calls"
        ]
    );
}

/// A resolve resumes a parked turn through the enqueue retries and the queue
/// wait: the parked reason would stay up with nothing pending any more.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_resolve_resuming_the_turn_says_it_is_queueing_the_next_step() {
    let held = json!({"call_hold": {
        "state": "pending", "function_id": "test::hold", "held_by": "test::gate"}});
    let (result, calls, turn) = run_on_fake_engine(
        json!({}),
        held,
        json!([]),
        None,
        json!({}),
        |deps| async move {
            let req = crate::functions::function_resolve::FunctionResolveRequest {
                session_id: "s".into(),
                turn_id: "t1".into(),
                function_call_id: "call_hold".into(),
                content: Some(vec![ContentBlock::text("denied")]),
                is_error: Some(true),
                ..Default::default()
            };
            crate::deferred::resolve(&deps, req).await
        },
    )
    .await;
    assert!(result.unwrap().turn_resumed);
    assert_eq!(turn["step"], 2);
    assert_eq!(phases(&calls), ["queueing next step", "harness::turn"]);
}

/// An approved call really runs: the parked reason read as still waiting on a
/// person, with nothing left to wait on but the running tool.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_released_call_stops_claiming_to_wait_for_pending_calls() {
    let (result, calls, _) = release(json!({"call_hold": held_call()}), json!({})).await;
    assert!(result.unwrap().turn_resumed);
    assert_eq!(
        phases(&calls),
        [
            "running tools",
            "test::hold",
            "queueing next step",
            "harness::turn"
        ]
    );
}

/// A released call post_call holds again runs nothing until someone decides:
/// `running tools` would stay up for as long as that takes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_released_call_held_again_stops_claiming_to_run_tools() {
    let (result, calls, turn) = release(
        json!({"call_hold": held_call()}),
        json!({"harness::hook::on-post-call": {"decision": "hold", "held_by": "test::gate"}}),
    )
    .await;
    assert!(!result.unwrap().turn_resumed);
    assert_eq!(turn["calls"]["call_hold"]["state"], "pending");
    assert_eq!(
        phases(&calls),
        [
            "running tools",
            "test::hold",
            "harness::hook::on-post-call",
            "waiting for pending calls"
        ]
    );
}

/// The released call finished and a sibling still waits on a person, for
/// hours maybe: `running tools` would stay up with nothing running.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_released_call_with_a_held_sibling_stops_claiming_to_run_tools() {
    let (result, calls, _) = release(
        json!({"call_hold": held_call(), "call_sibling": held_call()}),
        json!({}),
    )
    .await;
    assert!(!result.unwrap().turn_resumed);
    assert_eq!(
        phases(&calls),
        ["running tools", "test::hold", "waiting for pending calls"]
    );
}

/// External work cannot be confirmed stopped, so a stop leaves the turn parked
/// on its `stopping` ack: a live phase over it would show a stopped turn as
/// running until the next delivery observes the abort.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resolves_after_a_stop_keep_its_stopping_ack() {
    let parked = json!({
        "call_hold": held_call(),
        "call_external": {"state": "pending", "function_id": "test::external"},
    });
    let (result, calls, _) = run_on_fake_engine(
        json!({}),
        parked,
        json!([]),
        None,
        json!({}),
        |deps| async move {
            let cfg = deps.cfg().await;
            let mut record =
                crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
                    .await
                    .unwrap()
                    .unwrap();
            record.status = TurnStatus::AwaitingFunctions;
            crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms)
                .await
                .unwrap();
            stop(&deps).await;
            for req in [
                resolve_request("call_hold", "execute"),
                resolve_request("call_external", "deliver"),
            ] {
                crate::deferred::resolve(&deps, req).await.unwrap();
            }
            let payload =
                serde_json::from_value(json!({"session_id": "s", "turn_id": "t1", "step": 2}))
                    .unwrap();
            run_step(&deps, payload).await
        },
    )
    .await;
    assert_eq!(result.unwrap().status, TurnStatus::Cancelled);
    assert_eq!(
        phases(&calls),
        ["stopping", "test::hold", "harness::turn", "stopped"]
    );
}

fn held_call() -> Value {
    json!({"state": "pending", "function_id": "test::hold",
        "held_by": "test::gate", "held_arguments": {}})
}

fn resolve_request(
    call: &str,
    action: &str,
) -> crate::functions::function_resolve::FunctionResolveRequest {
    crate::functions::function_resolve::FunctionResolveRequest {
        session_id: "s".into(),
        turn_id: "t1".into(),
        function_call_id: call.into(),
        action: Some(action.into()),
        ..Default::default()
    }
}

async fn stop(deps: &Deps) {
    let req = crate::functions::stop::StopRequest {
        session_id: "s".into(),
        turn_id: None,
    };
    crate::functions::stop::stop_one(deps, req).await.unwrap();
}

/// Approve `call_hold` on a fake engine.
async fn release(
    checkpoints: Value,
    hooks: Value,
) -> (
    Result<crate::functions::function_resolve::FunctionResolveResponse, HarnessError>,
    Vec<(String, Value)>,
    Value,
) {
    run_on_fake_engine(
        json!({}),
        checkpoints,
        json!([]),
        None,
        hooks,
        |deps| async move {
            crate::deferred::resolve(&deps, resolve_request("call_hold", "execute")).await
        },
    )
    .await
}

/// Every status write (its reason, or the status without one) in order with
/// the hook chains, the tool call and the enqueue it must precede.
fn phases(calls: &[(String, Value)]) -> Vec<&str> {
    calls
        .iter()
        .filter_map(|(function, data)| match function.as_str() {
            "session::set-status" => data["reason"].as_str().or(data["status"].as_str()),
            "test::hold" | "harness::turn" => Some(function.as_str()),
            hook if hook.starts_with("harness::hook::") => Some(hook),
            _ => None,
        })
        .collect()
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
    run_on_fake_engine(json!({}), checkpoints, stored, None, json!({}), step_one).await
}

/// Run step 1 with nothing stored, the provider streaming `reply`.
async fn generate(
    reply: Value,
) -> (
    Result<TurnStepResult, HarnessError>,
    Vec<(String, Value)>,
    Value,
) {
    generate_observed(reply, json!({})).await
}

/// [`generate`] with `hooks` (hook function -> chain outcome) observed.
async fn generate_observed(
    reply: Value,
    hooks: Value,
) -> (
    Result<TurnStepResult, HarnessError>,
    Vec<(String, Value)>,
    Value,
) {
    run_on_fake_engine(
        json!({}),
        json!({}),
        json!([]),
        Some(reply),
        hooks,
        step_one,
    )
    .await
}

async fn step_one(deps: Deps) -> Result<TurnStepResult, HarnessError> {
    let payload =
        serde_json::from_value(json!({"session_id": "s", "turn_id": "t1", "step": 1})).unwrap();
    run_step(&deps, payload).await
}

/// Without a `reply` the provider's stream channel cannot open: the step ends
/// at that boundary. `script` answers chosen calls first: a function id, or
/// a function id and the state scope it acts on (`"state::list
/// harness_queue"`), maps to the replies its next calls get in order, each
/// `{"result": ..}` or `{"error": {"code", "message"}}`.
async fn run_on_fake_engine<T, F: std::future::Future<Output = T>>(
    mut script: Value,
    checkpoints: Value,
    stored: Value,
    reply: Option<Value>,
    hooks: Value,
    drive: impl FnOnce(Deps) -> F,
) -> (T, Vec<(String, Value)>, Value) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let calls = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
    let observed = calls.clone();
    let (ready_tx, ready_rx) = oneshot::channel();
    let turn = Arc::new(Mutex::new(json!({
        "session_id": "s", "turn_id": "t1", "status": "running",
        "step": 1, "turn_count": 2, "depth": 0,
        "options": {"model": "fake", "max_turns": 16,
            "functions": {"allow": ["*"], "deny": [], "expose": "agent_call"}},
        "calls": checkpoints,
        "created_at": 1, "updated_at": 1
    })));
    let stored_turn = turn.clone();
    let observed_hooks: Vec<String> = hooks.as_object().unwrap().keys().cloned().collect();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
        // The hook functions only register on an engine advertising observe.v2.
        let hello = json!({"type": "workerregistered", "worker_id": "w",
            "features": ["observe.v2"]});
        let frame = tokio_tungstenite::tungstenite::Message::Text(hello.to_string().into());
        socket.send(frame).await.unwrap();
        ready_tx.send(()).unwrap();
        let channel = reply.is_some();
        tokio::spawn(async move {
            let Some(reply) = reply else { return };
            let done = json!({"type": "done", "message": reply}).to_string();
            // The stream channel is not the only later dial: answer them all.
            while let Ok((tcp, _)) = listener.accept().await {
                let Ok(mut stream) = tokio_tungstenite::accept_async(tcp).await else {
                    continue;
                };
                let frame = tokio_tungstenite::tungstenite::Message::Text(done.clone().into());
                tokio::spawn(async move {
                    let _ = stream.send(frame).await;
                    let _ = stream.close(None).await;
                });
            }
        });
        while let Some(Ok(frame)) = socket.next().await {
            let Ok(text) = frame.to_text() else { continue };
            let Ok(message) = serde_json::from_str::<Value>(text) else {
                continue;
            };
            if message["type"] == "registerfunction" {
                let id = message["id"].as_str().unwrap();
                if !hooks[id].is_null() {
                    let registered = json!({"type": "functionregistered", "id": id, "revision": 1,
                        "observers": [{"id": format!("o_{id}"), "target": "test::gate"}]});
                    let frame = tokio_tungstenite::tungstenite::Message::Text(
                        registered.to_string().into(),
                    );
                    socket.send(frame).await.unwrap();
                }
                continue;
            }
            if message["type"] != "invokefunction" {
                continue;
            }
            let function = message["function_id"].as_str().unwrap().to_string();
            // The harness runs a hook chain itself and calls each observer's
            // target through its observer: record and answer that call as
            // the hook point the observer observes.
            let function = message["activation"]["observer_id"]
                .as_str()
                .and_then(|id| id.strip_prefix("o_"))
                .map_or(function, str::to_string);
            let data = message["data"].clone();
            observed
                .lock()
                .unwrap()
                .push((function.clone(), data.clone()));
            let scoped = format!("{function} {}", data["scope"].as_str().unwrap_or_default());
            let scripted = [scoped.as_str(), function.as_str()]
                .into_iter()
                .find_map(|call| {
                    let replies = script.get_mut(call)?.as_array_mut()?;
                    (!replies.is_empty()).then(|| replies.remove(0))
                });
            let result = match function.as_str() {
                _ if scripted.is_some() => scripted.as_ref().unwrap()["result"].clone(),
                hook if !hooks[hook].is_null() => hooks[hook].clone(),
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
                "session::messages" if !data["after_entry_id"].is_null() => {
                    json!({"messages": []})
                }
                "session::messages" => {
                    let mut messages = vec![json!({"entry_id": "e_user", "message": {
                        "role": "user", "timestamp": 1,
                        "content": [{"type": "text", "text": "hold"}]}})];
                    messages.extend(stored.as_array().unwrap().iter().cloned());
                    json!({ "messages": messages })
                }
                "session::get-message" => json!({"entry": {
                    "kind": "message", "id": data["entry_id"], "parent_id": null,
                    "timestamp": 2, "revision": 7,
                    "message": text_reply(0.25)["message"]}}),
                "session::update-message" => json!({"updated": true, "revision": 8}),
                "context::assemble" => json!({
                    "messages": [{"role": "user", "timestamp": 1,
                        "content": [{"type": "text", "text": "hold"}]}],
                    "token_count": 10, "usable": 1000, "effective_max_output_tokens": 100
                }),
                "session::set-status" => json!({"ok": true}),
                "session::append" => json!({"entry_id": data["entry_id"]}),
                "state::set" => json!({}),
                "harness::state::compare-and-set" => json!({"swapped": true}),
                "engine::channels::create" if channel => json!({
                    "writer": {"channel_id": "c", "access_key": "k", "direction": "write"},
                    "reader": {"channel_id": "c", "access_key": "k", "direction": "read"},
                }),
                _ => Value::Null,
            };
            if !message["invocation_id"].is_null() {
                let mut reply = json!({
                    "type": "invocationresult", "invocation_id": message["invocation_id"],
                    "function_id": function, "result": result
                });
                if let Some(error) = scripted.as_ref().and_then(|reply| reply.get("error")) {
                    reply["error"] = error.clone();
                    reply.as_object_mut().unwrap().remove("result");
                }
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
    // A hook chain is only called once its observer is active.
    tokio::time::timeout(Duration::from_secs(5), async {
        while observed_hooks.iter().any(|id| !iii.is_observed(id)) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the hooks' observers activate");
    // The catalog wait of an overflowing step alone takes about 7 s.
    let result = tokio::time::timeout(Duration::from_secs(20), drive(deps)).await;
    iii.shutdown();
    server.abort();
    let calls = calls.lock().unwrap().clone();
    let turn = turn.lock().unwrap().clone();
    (result.unwrap(), calls, turn)
}

/// An assembly that overflows only because the provider has not registered
/// the model yet (a harness waking sessions at boot, ahead of its providers)
/// waits for the catalog and assembles again. It used to fail the turn as an
/// unknown model, a failure nothing retries.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_overflow_on_a_model_listed_late_assembles_again() {
    let overflow = json!({"error": {"code": "context/overflow", "message":
        "context/overflow: assembled context requires 9000 tokens but usable budget is 8000"}});
    let (result, calls, _) = run_on_fake_engine(
        json!({
            "context::assemble": [overflow],
            "router::models::get": [
                {"result": {"model": null}},
                {"result": {"model": {"id": "fake"}}}
            ],
            "router::models::list": [{"result": {"models": []}}],
        }),
        json!({}),
        json!([]),
        Some(text_reply(0.25)["message"].clone()),
        json!({}),
        step_one,
    )
    .await;
    let assemblies = calls
        .iter()
        .filter(|(f, _)| f == "context::assemble")
        .count();
    assert_eq!(
        result.expect("the step finishes").status,
        TurnStatus::Completed,
        "{calls:?}"
    );
    assert_eq!(assemblies, 2, "{calls:?}");
}

/// A completion whose finalize drain delivers a message parked during the
/// last step is followed by a reseeded turn, so it is not terminal: a
/// consumer that finalizes the exchange on `terminal: true` closed it while
/// the session went on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_completion_followed_by_a_reseeded_turn_is_not_terminal() {
    let parked = json!({"id": "q1", "session_id": "s", "entry_id": "e_wake", "queued_at": 3,
        "message": {"role": "user", "timestamp": 3,
            "content": [{"type": "text", "text": "[notification] the job finished"}]}});
    // Parked before the first drain, or between it and the terminal write.
    for drains in [json!([[parked]]), json!([[], [parked]])] {
        let replies: Vec<Value> = drains
            .as_array()
            .unwrap()
            .iter()
            .map(|rows| json!({ "result": rows }))
            .collect();
        complete_with_parked_message(json!({ "state::list harness_queue": replies })).await;
    }
}

async fn complete_with_parked_message(script: Value) {
    let (result, calls, _) = run_on_fake_engine(
        script,
        json!({}),
        json!([]),
        None,
        json!({ crate::events::ON_TURN_COMPLETED_FN: {} }),
        |deps| async move {
            let session = deps.session().await;
            let mut record = crate::state::get_turn(&deps.iii, "s", 2_000)
                .await?
                .expect("the running turn");
            finalize_completed(&deps, &session, &mut record, Some(json!("done"))).await
        },
    )
    .await;
    assert_eq!(result.unwrap().status, TurnStatus::Completed);
    assert!(
        calls
            .iter()
            .any(|(f, data)| f == "session::append" && data["entry_id"] == "e_wake"),
        "the parked message reaches the transcript: {calls:?}"
    );
    let completed: Vec<&Value> = calls
        .iter()
        .filter(|(f, _)| f == crate::events::ON_TURN_COMPLETED_FN)
        .map(|(_, data)| data)
        .collect();
    assert_eq!(completed.len(), 1, "{calls:?}");
    assert_eq!(completed[0]["terminal"], json!(false), "{completed:?}");
}
