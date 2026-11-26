//! Turn-loop steps run against a fake engine on the SDK's default-size thread stack.
use super::*;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use iii_sdk::{InitOptions, RegisterFunction, register_worker};
use tokio::sync::{RwLock, mpsc, oneshot};

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
            generation_id: None,
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

/// A completing turn decides once whether it ends the exchange. Its extraction
/// proof and its completion event agree even when the session's wake store
/// answers differently between the two writes (unreadable, then empty). Memory
/// acknowledged the turn and left before it ended: no pre_generate observer is
/// left, and the boundary is still recorded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_completion_gives_its_proof_and_its_event_one_terminal_decision() {
    use crate::events::ON_TURN_COMPLETED_FN;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let calls = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
    let observed = calls.clone();
    let (ready_tx, ready_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
        let send =
            |value: Value| tokio_tungstenite::tungstenite::Message::Text(value.to_string().into());
        let hello = json!({"type": "workerregistered", "worker_id": "w",
            "features": ["observe.v2"]});
        socket.send(send(hello)).await.unwrap();
        ready_tx.send(()).unwrap();
        let mut ledger = json!([{"session_id": "s", "entry_id": "e_user", "channel": "direct",
            "query": null, "continuation": null, "extraction": [{"version": 1,
            "session_id": "s", "turn_id": "t1", "entry_id": "e_user", "text_sha256": "h",
            "policy": "on", "bank": "b"}]}]);
        let mut wake_reads = 0;
        while let Some(Ok(frame)) = socket.next().await {
            let Ok(text) = frame.to_text() else { continue };
            let Ok(message) = serde_json::from_str::<Value>(text) else {
                continue;
            };
            if message["type"] == "registerfunction" && message["id"] == ON_TURN_COMPLETED_FN {
                let registered = json!({"type": "functionregistered", "id": ON_TURN_COMPLETED_FN,
                    "revision": 1, "observers": [{"id": "o_completed", "target": "test::gate"}]});
                socket.send(send(registered)).await.unwrap();
                continue;
            }
            if message["type"] != "invokefunction" {
                continue;
            }
            let function = message["function_id"].as_str().unwrap().to_string();
            let data = message["data"].clone();
            observed
                .lock()
                .unwrap()
                .push((function.clone(), data.clone()));
            let scope = data["scope"].as_str().unwrap_or_default();
            let mut reply = json!({"type": "invocationresult",
                "invocation_id": message["invocation_id"], "function_id": function});
            match (function.as_str(), scope) {
                ("harness::state::list", crate::state::BINDING_SCOPE) => {
                    wake_reads += 1;
                    if wake_reads == 1 {
                        reply["error"] = json!({"code": "UNAVAILABLE", "message": "store busy"});
                    } else {
                        reply["result"] = json!([]);
                    }
                }
                ("harness::state::get", crate::hooks::recall::SCOPE) => {
                    reply["result"] = ledger.clone();
                }
                ("harness::state::compare-and-set", crate::hooks::recall::SCOPE) => {
                    ledger = data["value"].clone();
                    reply["result"] = json!({"swapped": true});
                }
                ("state::list" | "harness::state::list", _) => reply["result"] = json!([]),
                _ => reply["result"] = Value::Null,
            }
            if !message["invocation_id"].is_null() && socket.send(send(reply)).await.is_err() {
                break;
            }
        }
        ledger
    });
    let iii = Arc::new(register_worker(&url, InitOptions::default()));
    ready_rx.await.unwrap();
    let deps = Deps::new(
        iii.clone(),
        Arc::new(RwLock::new(Arc::new(crate::config::WorkerConfig {
            session_timeout_ms: 2_000,
            ..Default::default()
        }))),
        crate::discovery::new_cell(),
        crate::skills::new_cell(),
        crate::events::TurnEvents::register(&iii),
        crate::hooks::HookRegistry::register(&iii),
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while !iii.is_observed(ON_TURN_COMPLETED_FN) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the completion event's observer activates");
    let mut record: TurnRecord = serde_json::from_value(json!({
        "session_id": "s", "turn_id": "t1", "status": "running",
        "step": 1, "turn_count": 2, "depth": 0, "recall_sent": true,
        "options": {"model": "fake", "max_turns": 16}, "created_at": 1, "updated_at": 1
    }))
    .unwrap();
    let session = deps.session().await;
    tokio::time::timeout(
        Duration::from_secs(10),
        finalize_completed(&deps, &session, &mut record, None),
    )
    .await
    .unwrap()
    .unwrap();
    iii.shutdown();
    let ledger = server.await.unwrap();
    let proof = ledger[0]["extraction"][0]["terminal"]["exchange_finished"].clone();
    let calls = calls.lock().unwrap();
    let event = calls
        .iter()
        .find(|(function, _)| function == ON_TURN_COMPLETED_FN)
        .map(|(_, data)| data["terminal"].clone())
        .expect("the completion event");
    assert!(proof.is_boolean(), "the proof records a boundary: {ledger}");
    assert_eq!(proof, event, "one terminal decision for proof and event");
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

/// A send can narrow a running side chat while its model request is in flight.
/// The generated call must see that durable ceiling, and finalization must not
/// overwrite it with the wider policy that generated the reply.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_policy_narrowed_during_generation_applies_before_dispatch() {
    let (result, calls, turn) = generate_observed(
        hold_reply()["message"].clone(),
        json!({"narrow_during_generate": true}),
    )
    .await;

    assert_eq!(result.expect("the denied call advances").next_step, Some(2));
    assert!(!calls.iter().any(|(function, _)| function == "test::hold"));
    assert_eq!(
        turn["options"]["functions"],
        json!({"allow": ["session::get"], "deny": [], "expose": "agent_call"})
    );
    assert_eq!(turn["functions_acknowledged"], json!([999]));
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

/// A `shell::fs::read` result is a stream handle: the turn records it with its
/// `access_key` withheld, in its place a note naming the call that reads text
/// (MOT-5827).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stream_handle_result_is_recorded_with_its_access_key_withheld() {
    let mut reply = hold_reply()["message"].clone();
    reply["content"][0]["arguments"] =
        json!({"function": "shell::fs::read", "payload": {"path": "MARKER.txt"}});
    let read = json!({"content": {"channel_id": "c", "access_key": "secret-key",
        "direction": "read"}, "size": 16});
    let (result, calls, _) = generate_observed(reply, json!({"shell::fs::read": read})).await;
    assert_eq!(result.expect("the step advances").next_step, Some(2));
    assert!(
        calls.iter().any(|(f, _)| f == "shell::fs::read"),
        "dispatched"
    );
    let recorded = calls
        .iter()
        .find(|(f, d)| f == "session::append" && d.to_string().contains("function_result"))
        .map(|(_, d)| d.to_string())
        .expect("the result is appended");
    assert!(!recorded.contains("secret-key"), "{recorded}");
    assert!(recorded.contains("coder::read-file"), "{recorded}");
}

/// Recall provenance is read only when the step calls Memory's pre_generate
/// hook: a stack without Memory reads no ledger, neither per step nor when the
/// turn ends, even with another pre_generate observer called every step.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_a_step_calling_memory_reads_the_recall_ledger() {
    let ledger_reads = |calls: &[(String, Value)]| {
        calls
            .iter()
            .filter(|(f, data)| {
                f.ends_with("state::get") && data["scope"] == crate::hooks::recall::SCOPE
            })
            .count()
    };
    let finished = || text_reply(0.25)["message"].clone();
    let (result, calls, _) = generate(finished()).await;
    assert_eq!(result.unwrap().status, TurnStatus::Completed);
    assert_eq!(ledger_reads(&calls), 0, "{calls:?}");

    let gate = json!({"harness::hook::on-pre-generate": {"decision": "continue"}});
    let (result, calls, turn) = generate_observed(finished(), gate).await;
    assert_eq!(result.unwrap().status, TurnStatus::Completed);
    assert!(phases(&calls).contains(&"harness::hook::on-pre-generate"));
    assert_eq!(ledger_reads(&calls), 0, "{calls:?}");
    assert!(turn.get("recall_sent").is_none(), "{turn}");

    let memory = json!({"harness::hook::on-pre-generate":
        {"decision": "continue", "target": "memory::hook::pre-generate"}});
    let (result, calls, turn) = generate_observed(hold_reply()["message"].clone(), memory).await;
    assert_eq!(result.unwrap().next_step, Some(2));
    assert_eq!(ledger_reads(&calls), 1, "Memory gets the source");
    // Stored: the turn's end records its boundary even once Memory left.
    assert_eq!(turn["recall_sent"], true, "{turn}");
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
    let (result, calls, turn) =
        run_on_fake_engine(held, json!([]), None, json!({}), |deps| async move {
            let req = crate::functions::function_resolve::FunctionResolveRequest {
                session_id: "s".into(),
                turn_id: "t1".into(),
                function_call_id: "call_hold".into(),
                content: Some(vec![ContentBlock::text("denied")]),
                is_error: Some(true),
                ..Default::default()
            };
            crate::deferred::resolve(&deps, req).await
        })
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

/// A person approved the call for one namespace: the release must run it there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_released_call_runs_in_the_namespace_it_was_held_for() {
    let mut held = held_call();
    held["namespace"] = json!("zz-a");
    let hooks =
        json!({"harness::hook::on-post-call": {"decision": "hold", "held_by": "test::gate"}});
    let (_, calls, _) = release(json!({"call_hold": held}), hooks).await;
    let post = calls
        .iter()
        .find(|(function, _)| function == "harness::hook::on-post-call")
        .expect("post_call ran");
    assert_eq!(post.1["call"]["namespace"], "zz-a");
    assert_eq!(sent_namespace(&calls, "test::hold"), "zz-a");
}

/// The hooks after the holder see the wrapper's description on release, as
/// they would have without the hold: it lives only in the transcript.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_released_call_shows_later_hooks_its_description() {
    let stored = json!([{"entry_id": "e_t1_1_assistant", "message": {
        "role": "assistant", "stop_reason": "function_call",
        "model": "fake", "provider": "fake", "timestamp": 2,
        "usage": {"input": 10, "output": 2, "cost_usd": 0.1},
        "content": [{"type": "function_call", "id": "call_hold",
            "function_id": "agent_call",
            "arguments": {"function": "test::hold", "arguments": {},
                "description": "Writing tests"}}]}}]);
    let hooks = json!({"harness::hook::on-pre-call": {
        "decision": "continue", "targets": ["test::gate", "test::stamp"]}});
    let (result, calls, _) = run_on_fake_engine(
        json!({"call_hold": held_call()}),
        stored,
        None,
        hooks,
        |deps| async move {
            crate::deferred::resolve(&deps, resolve_request("call_hold", "execute")).await
        },
    )
    .await;
    assert!(result.unwrap().turn_resumed);
    let resumed: Vec<&Value> = calls
        .iter()
        .filter(|(function, _)| function == "harness::hook::on-pre-call")
        .map(|(_, data)| data)
        .collect();
    assert_eq!(resumed.len(), 1, "only the hook after the holder runs");
    assert_eq!(resumed[0]["call"]["description"], "Writing tests");
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

/// The namespace the invoke frame of `function` named.
fn sent_namespace<'a>(calls: &'a [(String, Value)], function: &str) -> &'a Value {
    let key = format!("namespace of {function}");
    &calls
        .iter()
        .find(|(f, _)| *f == key)
        .unwrap_or_else(|| panic!("{function} was not invoked: {calls:?}"))
        .1
}

fn hold_reply_in(namespace: &str) -> Value {
    let mut reply = hold_reply()["message"].clone();
    reply["content"][0]["arguments"]["namespace"] = json!(namespace);
    reply
}

/// Whether this harness answered `function`, named in namespace `zz-a`, with
/// the refusal its own controls get there, and sent no frame for it.
fn refused_in_zz_a(calls: &[(String, Value)], function: &str) -> bool {
    let refusal = format!("{function} was called in namespace zz-a");
    !calls.iter().any(|(f, _)| f == function)
        && calls
            .iter()
            .any(|(f, data)| f == "session::append" && data.to_string().contains(&refusal))
}

/// The model named a namespace: the call goes out in it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_runs_in_the_namespace_the_model_named() {
    let (_, calls, _) = generate(hold_reply_in("zz-a")).await;
    assert_eq!(sent_namespace(&calls, "test::hold"), "zz-a");
}

fn hold_in(namespace: &str) -> crate::clients::engine::FunctionDescriptor {
    crate::clients::engine::FunctionDescriptor {
        function_id: "test::hold".into(),
        namespace: namespace.into(),
        description: None,
        parameters: None,
    }
}

/// The stored turn switched to native exposure, with `native_names` set.
async fn go_native(deps: &Deps, native_names: Value) -> Result<(), HarnessError> {
    let record = crate::state::get_turn_unhydrated(&deps.iii, "s", 2_000).await?;
    let mut record = serde_json::to_value(record.expect("the turn is stored")).unwrap();
    record["options"]["functions"]["expose"] = json!("native");
    record["native_names"] = native_names;
    let record = serde_json::from_value(record).unwrap();
    crate::state::put_turn(&deps.iii, &record, 2_000).await
}

/// The model's native call `zz-a::test::hold` named the `zz_a` copy, the only
/// one when the request went out. `zz-a` registering meanwhile sorts first
/// and takes that name in a newer table: the call must still run in `zz_a`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_native_call_runs_where_its_generation_named_it() {
    let mut reply = hold_reply()["message"].clone();
    reply["content"][0]["function_id"] = json!("zz-a::test::hold");
    reply["content"][0]["arguments"] = json!({});
    for joins in [false, true] {
        let (_, calls, _) = run_on_fake_engine(
            json!({}),
            json!([]),
            Some(reply.clone()),
            json!({}),
            |deps| async move {
                go_native(&deps, json!({})).await?;
                crate::discovery::apply(&deps.functions, vec![hold_in("zz_a")]).await;
                let payload =
                    serde_json::from_value(json!({"session_id": "s", "turn_id": "t1", "step": 1}))
                        .unwrap();
                let PreparedStep::Generated(generated) =
                    Box::pin(generate_step(&deps, payload)).await?
                else {
                    panic!("the step generates");
                };
                if joins {
                    let both = vec![hold_in("zz-a"), hold_in("zz_a")];
                    crate::discovery::apply(&deps.functions, both).await;
                }
                Box::pin(finish_step(&deps, generated)).await
            },
        )
        .await;
        assert_eq!(
            sent_namespace(&calls, "test::hold"),
            "zz_a",
            "joins: {joins}"
        );
    }
}

/// A redelivered step replays its stored reply with the table its generation
/// stored, whatever registered since.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replayed_native_call_runs_where_its_generation_named_it() {
    let mut stored = hold_reply();
    let mut next = stored["message"]["content"][0].clone();
    next["id"] = json!("call_next");
    stored["message"]["content"]
        .as_array_mut()
        .unwrap()
        .push(next);
    for call in stored["message"]["content"].as_array_mut().unwrap() {
        call["function_id"] = json!("zz-a::test::hold");
        call["arguments"] = json!({});
    }
    let dispatched = json!({"call_hold": {"state": "dispatched", "function_id": "test::hold",
        "namespace": "zz_a"}});
    let (_, calls, _) = run_on_fake_engine(
        dispatched,
        json!([stored]),
        None,
        json!({}),
        |deps| async move {
            go_native(&deps, json!({"zz-a::test::hold": ["zz_a", "test::hold"]})).await?;
            let both = vec![hold_in("zz-a"), hold_in("zz_a")];
            crate::discovery::apply(&deps.functions, both).await;
            step_one(deps).await
        },
    )
    .await;
    assert_eq!(sent_namespace(&calls, "test::hold"), "zz_a");
}

/// A deny qualified with the namespace the model named stops the call there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_denied_in_the_namespace_it_named_is_not_dispatched() {
    let (_, calls, turn) = generate(hold_reply_in("zz-denied")).await;
    assert!(!calls.iter().any(|(f, _)| f == "test::hold"), "{calls:?}");
    assert_eq!(turn["calls"]["call_hold"]["state"], "done", "{turn}");
}

/// A held call is released later, from its checkpoint: the namespace must be
/// on it, or an approval for another namespace runs in the implicit one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_held_call_keeps_the_namespace_the_model_named() {
    let (_, _, turn) = generate_observed(
        hold_reply_in("zz-a"),
        json!({"harness::hook::on-pre-call": {"decision": "hold", "held_by": "test::gate"}}),
    )
    .await;
    assert_eq!(turn["calls"]["call_hold"]["namespace"], "zz-a");
}

/// An old approval gate cannot show a namespace: the call is refused before it runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_in_another_namespace_is_refused_under_an_old_gate() {
    let hooks =
        json!({"harness::hook::on-pre-call": {"decision": "continue", "target": "approval::gate"}});
    let (_, calls, _) = generate_observed(hold_reply_in("zz-a"), hooks).await;
    assert!(!calls.iter().any(|(f, _)| f == "test::hold"), "{calls:?}");
    assert!(
        calls.iter().any(|(f, data)| f == "session::append"
            && data.to_string().contains("approval_gate_too_old")),
        "{calls:?}"
    );
}

/// A call held for another namespace before a gate rollback: the old gate
/// releases it unseen, so the release refuses it before it runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_released_call_in_another_namespace_is_refused_under_an_old_gate() {
    let mut held = held_call();
    held["namespace"] = json!("zz-a");
    held["held_by"] = json!("approval::gate");
    let hooks =
        json!({"harness::hook::on-pre-call": {"decision": "continue", "target": "approval::gate"}});
    let (_, calls, _) = release(json!({"call_hold": held}), hooks).await;
    assert!(!calls.iter().any(|(f, _)| f == "test::hold"), "{calls:?}");
    assert!(
        calls.iter().any(|(f, data)| f == "session::append"
            && data.to_string().contains("approval_gate_too_old")),
        "{calls:?}"
    );
}

/// A control this harness answers itself, named in another namespace, is
/// refused: neither answered here nor dispatched to that namespace's harness.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_control_the_model_named_in_another_namespace_is_refused() {
    for control in [crate::functions::ASK_ID, crate::functions::SPAWN_ID] {
        let mut reply = hold_reply_in("zz-a");
        reply["content"][0]["arguments"]["function"] = json!(control);
        let (_, calls, turn) = generate(reply).await;
        assert!(refused_in_zz_a(&calls, control), "{control}: {calls:?}");
        assert!(
            turn["calls"]["call_hold"]["child_session_id"].is_null(),
            "{turn}"
        );
    }
}

/// A spawn approved for another namespace is refused: the generic release
/// path would dispatch it to that namespace's harness, which seeds a child
/// with none of this turn's guards.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_released_spawn_named_in_another_namespace_is_refused() {
    let mut held = held_call();
    held["function_id"] = json!(crate::functions::SPAWN_ID);
    held["namespace"] = json!("zz-a");
    let (_, calls, turn) = release(json!({"call_hold": held}), json!({})).await;
    assert!(
        refused_in_zz_a(&calls, crate::functions::SPAWN_ID),
        "{calls:?}"
    );
    assert!(
        turn["calls"]["call_hold"]["child_session_id"].is_null(),
        "{turn}"
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
    let (result, calls, _) =
        run_on_fake_engine(parked, json!([]), None, json!({}), |deps| async move {
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
            // On its own task, as the SDK runs the step handler: under this test's frames
            // a debug build's step overflows the default thread stack.
            tokio::spawn(async move { run_step(&deps, payload).await })
                .await
                .expect("step task")
        })
        .await;
    assert_eq!(result.unwrap().status, TurnStatus::Cancelled);
    assert_eq!(
        phases(&calls),
        ["stopping", "test::hold", "harness::turn", "stopped"]
    );
}

/// A wake queued during a parking turn's final step reseeds the session once
/// the turn finishes. The parking turn is not the run's outcome: a consumer
/// that settled on it lost the woken turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_finalize_that_drains_a_wake_reports_the_turn_as_continuing() {
    let hooks = json!({ "harness::on-turn-completed": {} });
    let (result, calls, turn) =
        run_on_fake_engine(json!({}), json!([]), None, hooks, |deps| async move {
            let cfg = deps.cfg().await;
            let wake = crate::state::QueuedMessage {
                id: "q_wake".into(),
                session_id: "s".into(),
                message: AgentMessage::user_text("[notification] woke"),
                entry_id: "e_fire_sub_1".into(),
                origin: None,
                channel: None,
                queued_at: 1,
            };
            crate::state::enqueue_message(&deps.iii, &wake, cfg.session_timeout_ms)
                .await
                .unwrap();
            let mut record =
                crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
                    .await
                    .unwrap()
                    .unwrap();
            finalize_completed(&deps, &deps.session().await, &mut record, None).await
        })
        .await;
    assert_eq!(result.unwrap().status, TurnStatus::Completed);
    let completed: Vec<&Value> = calls
        .iter()
        .filter(|(function, _)| function == "harness::on-turn-completed")
        .map(|(_, data)| data)
        .collect();
    assert_eq!(completed.len(), 1, "{calls:?}");
    assert_eq!(completed[0]["turn_id"], "t1");
    assert_eq!(completed[0]["terminal"], false);
    // `harness::status` reads `continues` on every write of the finished turn,
    // so it reports `expects_wake` until the reseed replaces the record.
    let finished: Vec<&Value> = calls
        .iter()
        .filter(|(function, data)| {
            function == "state::set"
                && data["scope"] == "harness_turn"
                && data["value"]["turn_id"] == "t1"
        })
        .map(|(_, data)| &data["value"])
        .collect();
    assert!(!finished.is_empty());
    assert!(
        finished
            .iter()
            .all(|record| record["status"] == "completed" && record["continues"] == true),
        "{finished:?}"
    );
    assert_eq!(turn["status"], "running");
    assert_ne!(
        turn["turn_id"], "t1",
        "the drained wake reseeds the session"
    );
    assert!(turn.get("continues").is_none(), "{turn}");
}

/// The reseed after a finalize drain fails before any reseeded turn is stored.
/// Nothing replaces the finished turn, so it completes again, terminally.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_reseed_ends_the_exchange() {
    let refuse = |function: &str, data: &Value| function == "state::set" && reseeds(data);
    let (status, calls, turn) = finalize_with_a_queued_wake(refuse, false).await;
    assert_eq!(
        completions(&calls),
        [(json!("t1"), json!(false)), (json!("t1"), json!(true))]
    );
    assert_eq!(turn["turn_id"], "t1");
    assert_eq!(turn["status"], "completed");
    assert!(turn.get("continues").is_none(), "{turn}");
    assert!(!status.expects_wake);
    assert_eq!(
        turn["took_wakes"],
        json!(["sub"]),
        "its finalize drained the wake"
    );
}

/// A step whose drain fails after it delivered a wake still notes the wake:
/// the re-drain on a retry no longer lists that row.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_step_drain_failing_after_a_wake_still_notes_it() {
    let refuse = |function: &str, data: &Value| {
        function == "session::append" && data["entry_id"] == "e_user_2"
    };
    let ((result, queued), _, turn) = run_on_faulty_engine(
        json!({}),
        json!([]),
        None,
        json!({}),
        refuse,
        |deps| async move {
            queue_wake_then_message(&deps).await;
            let result = step_one(deps.clone()).await;
            (result, queued_entries(&deps).await)
        },
    )
    .await;
    assert!(result.is_err());
    assert_eq!(turn["took_wakes"], json!(["sub"]), "{turn}");
    assert_eq!(queued, ["e_user_2"]);
}

/// The finalize drain notes a delivered wake even when a later row fails.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_finalize_drain_failing_after_a_wake_still_notes_it() {
    let refuse = |function: &str, data: &Value| {
        function == "session::append" && data["entry_id"] == "e_user_2"
    };
    let ((took_wakes, reseeds, queued), _, _) = run_on_faulty_engine(
        json!({}),
        json!([]),
        None,
        json!({}),
        refuse,
        |deps| async move {
            queue_wake_then_message(&deps).await;
            let cfg = deps.cfg().await;
            let mut record =
                crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
                    .await
                    .unwrap()
                    .unwrap();
            let session = deps.session().await;
            let reseeds = drain_queued_best_effort(&deps, &session, &mut record).await;
            (record.took_wakes, reseeds, queued_entries(&deps).await)
        },
    )
    .await;
    assert_eq!(took_wakes, ["sub"]);
    assert!(!reseeds, "a failed drain does not reseed");
    assert_eq!(queued, ["e_user_2"]);
}

/// Queue a wake fire, then a user message behind it.
async fn queue_wake_then_message(deps: &Deps) {
    let cfg = deps.cfg().await;
    let rows = [
        (
            "q_1",
            "e_fire_sub_1",
            Some(json!({ "notification": true, "binding": "sub" })),
        ),
        ("q_2", "e_user_2", None),
    ];
    for (queued_at, (id, entry_id, origin)) in (1..).zip(rows) {
        let row = crate::state::QueuedMessage {
            id: id.into(),
            session_id: "s".into(),
            message: AgentMessage::user_text(entry_id),
            entry_id: entry_id.into(),
            origin,
            channel: None,
            queued_at,
        };
        crate::state::enqueue_message(&deps.iii, &row, cfg.session_timeout_ms)
            .await
            .unwrap();
    }
}

/// Entry ids still in the session's queue.
async fn queued_entries(deps: &Deps) -> Vec<String> {
    let cfg = deps.cfg().await;
    crate::state::list_queued(&deps.iii, "s", cfg.session_timeout_ms)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.entry_id)
        .collect()
}

/// A reseeded turn whose first enqueue fails has finalized itself: the turn
/// before it is not settled again over it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reseed_that_cannot_be_scheduled_is_not_settled_over() {
    let refuse = |function: &str, data: &Value| function == "harness::turn" && reseeds(data);
    let (status, calls, turn) = finalize_with_a_queued_wake(refuse, false).await;
    assert_ne!(turn["turn_id"], "t1", "{turn}");
    assert_eq!(turn["status"], "failed", "{turn}");
    assert!(!status.expects_wake);
    assert!(
        !completions(&calls).contains(&(json!("t1"), json!(true))),
        "{calls:?}"
    );
}

/// A failed turn's reseed fails the same way: the second completion carries
/// its failure and ends the exchange.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_turn_whose_reseed_fails_ends_the_exchange() {
    let refuse = |function: &str, data: &Value| function == "state::set" && reseeds(data);
    let (status, calls, turn) = finalize_with_a_queued_wake(refuse, true).await;
    assert_eq!(
        completions(&calls),
        [(json!("t1"), json!(false)), (json!("t1"), json!(true))]
    );
    let settled = &calls
        .iter()
        .rfind(|(function, _)| function == "harness::on-turn-completed")
        .unwrap()
        .1;
    assert_eq!(settled["status"], "failed", "{settled}");
    assert!(settled["result_error"].is_string(), "{settled}");
    assert_eq!(settled["result_error"], turn["result_error"], "{turn}");
    assert_eq!(settled["reason"], turn["result_error"], "{settled}");
    assert_eq!(turn["status"], "failed");
    assert_eq!(turn["exchange_finished"], true, "{turn}");
    assert!(turn.get("continues").is_none(), "{turn}");
    assert!(turn.get("owes_completion").is_none(), "{turn}");
    assert!(!status.expects_wake);
}

/// A store that also refuses the settling write gets no terminal completion
/// yet: the record keeps its promise for the orphan redrive to settle or
/// reseed, so the exchange never continues after a terminal completion.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reseed_failure_whose_settle_write_fails_completes_nothing_yet() {
    let refuse =
        |function: &str, data: &Value| function == "state::set" && (reseeds(data) || settles(data));
    let (_, calls, turn) = finalize_with_a_queued_wake(refuse, false).await;
    assert_eq!(
        turn["continues"], true,
        "the settle write was refused: {turn}"
    );
    assert_eq!(completions(&calls), [(json!("t1"), json!(false))]);
}

/// A settle write that errors may still have landed, with `continues` gone: the
/// stored turn then owes its completion, and the next delivery of its step sends it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_settle_write_that_lands_despite_its_error_still_completes() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static LOST: AtomicBool = AtomicBool::new(false);
    fn refuse(function: &str, data: &Value) -> bool {
        function == "state::set"
            && (reseeds(data) || (settles(data) && !LOST.swap(true, Ordering::SeqCst)))
    }
    let hooks = json!({ "harness::on-turn-completed": {} });
    let ((after, step), calls, turn) = run_on_misleading_engine(
        json!({}),
        json!([]),
        None,
        hooks,
        refuse,
        |function, data| function == "state::set" && settles(data),
        |deps| async move {
            finalize_t1_with_a_queued_wake(&deps, false).await;
            let cfg = deps.cfg().await;
            let after = crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
                .await
                .unwrap()
                .unwrap();
            (after, step_one(deps.clone()).await)
        },
    )
    .await;
    assert!(after.awaits_step(), "the settle landed: {after:?}");
    assert!(step.unwrap().skipped);
    assert_eq!(
        completions(&calls),
        [(json!("t1"), json!(false)), (json!("t1"), json!(true))]
    );
    assert_eq!(turn["turn_id"], "t1", "{turn}");
    assert!(turn.get("continues").is_none(), "{turn}");
    assert!(turn.get("owes_completion").is_none(), "{turn}");
}

/// The process died after the finalize wrote `continues` and before its
/// reseed: the redelivered step seeds the turn that write promised.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redelivered_finished_step_redoes_the_reseed_its_finalize_never_reached() {
    for status in [TurnStatus::Completed, TurnStatus::Failed] {
        let ((result, report), calls, turn) =
            redeliver_finished(status, true, false, |_, _| false).await;
        assert!(result.unwrap().skipped);
        assert_ne!(turn["turn_id"], "t1", "{turn}");
        assert_eq!(turn["status"], "running", "{turn}");
        assert!(turn.get("continues").is_none(), "{turn}");
        assert!(
            calls
                .iter()
                .any(|(function, data)| function == "harness::turn" && reseeds(data)),
            "{calls:?}"
        );
        assert_eq!(report.status, TurnStatus::Running, "{report:?}");
    }
}

/// The redone reseed fails like a live one: the finished turn ends the
/// exchange instead of promising a turn nobody seeds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redelivered_finished_step_whose_reseed_fails_ends_the_exchange() {
    let refuse = |function: &str, data: &Value| function == "state::set" && reseeds(data);
    let ((result, report), calls, turn) =
        redeliver_finished(TurnStatus::Completed, true, false, refuse).await;
    assert!(result.unwrap().skipped);
    assert_eq!(completions(&calls), [(json!("t1"), json!(true))]);
    assert_eq!(turn["turn_id"], "t1");
    assert!(turn.get("continues").is_none(), "{turn}");
    assert!(!report.expects_wake);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redelivered_finished_step_that_does_not_continue_is_only_dropped() {
    let ((result, _), calls, turn) =
        redeliver_finished(TurnStatus::Completed, false, false, |_, _| false).await;
    assert!(result.unwrap().skipped);
    assert_eq!(turn["turn_id"], "t1");
    assert!(completions(&calls).is_empty(), "{calls:?}");
    assert!(
        !calls
            .iter()
            .any(
                |(function, data)| (function == "harness::turn" || function == "state::set")
                    && reseeds(data)
            ),
        "{calls:?}"
    );
}

/// A stop that reaches the session between the crashed finalize and its
/// redelivery takes back the turn that finalize promised: the exchange ends as
/// a stop does, and the redelivery seeds nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_before_the_redelivery_takes_back_the_promised_turn() {
    for (status, ended) in [
        (TurnStatus::Completed, "cancelled"),
        (TurnStatus::Failed, "failed"),
    ] {
        let ((result, report), calls, turn) =
            redeliver_finished(status, true, true, |_, _| false).await;
        assert!(result.unwrap().skipped);
        assert_eq!(turn["turn_id"], "t1", "{turn}");
        assert_eq!(turn["status"], ended, "{turn}");
        assert!(turn.get("continues").is_none(), "{turn}");
        assert_eq!(turn["abort"], true, "{turn}");
        assert_eq!(turn["exchange_finished"], true, "{turn}");
        assert!(turn.get("owes_completion").is_none(), "{turn}");
        let completed: Vec<(Value, Value, Value)> = calls
            .iter()
            .filter(|(function, _)| function == "harness::on-turn-completed")
            .map(|(_, data)| {
                (
                    data["turn_id"].clone(),
                    data["status"].clone(),
                    data["terminal"].clone(),
                )
            })
            .collect();
        assert_eq!(completed, [(json!("t1"), json!(ended), json!(true))]);
        assert!(
            !calls
                .iter()
                .any(
                    |(function, data)| (function == "harness::turn" || function == "state::set")
                        && reseeds(data)
                ),
            "{calls:?}"
        );
        assert!(!report.expects_wake);
    }
}

/// A stop accepted on a running turn that finds it finished under the lock,
/// still promising a turn its finalize could not seed or settle, takes that
/// promise back: the redrive seeds nothing after the stop.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_that_finds_the_promise_under_the_lock_takes_it_back() {
    let hooks = json!({ "harness::on-turn-completed": {} });
    let ((stopped, result), calls, turn) = run_on_faulty_engine(
        json!({}),
        json!([]),
        None,
        hooks,
        |_, _| false,
        |deps| async move {
            let cfg = deps.cfg().await;
            let guard = deps.locks.guard("s").await;
            let stopping = deps.clone();
            let stop = tokio::spawn(async move {
                let req = crate::functions::stop::StopRequest {
                    session_id: "s".into(),
                    turn_id: None,
                    quiet_deadlines: false,
                };
                crate::functions::stop::stop_one(&stopping, req).await
            });
            while !deps.cancels.is_fired("t1") {
                tokio::task::yield_now().await;
            }
            let mut record =
                crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
                    .await
                    .unwrap()
                    .unwrap();
            record.status = TurnStatus::Completed;
            record.continues = true;
            crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms)
                .await
                .unwrap();
            drop(guard);
            let stopped = stop.await.unwrap().unwrap();
            (stopped, step_one(deps).await)
        },
    )
    .await;
    assert!(stopped.stopping);
    result.unwrap();
    assert_eq!(turn["turn_id"], "t1", "{turn}");
    assert_eq!(turn["status"], "cancelled", "{turn}");
    assert!(turn.get("continues").is_none(), "{turn}");
    assert_eq!(turn["abort"], true, "{turn}");
    assert_eq!(turn["exchange_finished"], true, "{turn}");
    assert!(
        !calls
            .iter()
            .any(
                |(function, data)| (function == "harness::turn" || function == "state::set")
                    && reseeds(data)
            ),
        "{calls:?}"
    );
}

/// A stop whose write is refused ends nothing: the stored promise would
/// otherwise be redriven into a turn after the stop's terminal completion.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_that_cannot_take_back_the_promised_turn_ends_nothing() {
    let hooks = json!({ "harness::on-turn-completed": {} });
    let refuse = |function: &str, data: &Value| {
        function == "state::set"
            && data["scope"] == "harness_turn"
            && data["value"]["abort"] == true
    };
    let (stopped, calls, turn) = run_on_faulty_engine(
        json!({}),
        json!([]),
        None,
        hooks,
        refuse,
        |deps| async move {
            let cfg = deps.cfg().await;
            let mut record =
                crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
                    .await
                    .unwrap()
                    .unwrap();
            record.status = TurnStatus::Failed;
            record.continues = true;
            crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms)
                .await
                .unwrap();
            let req = crate::functions::stop::StopRequest {
                session_id: "s".into(),
                turn_id: None,
                quiet_deadlines: false,
            };
            crate::functions::stop::stop_one(&deps, req).await
        },
    )
    .await;
    assert!(stopped.is_err());
    assert!(completions(&calls).is_empty(), "{calls:?}");
    assert_eq!(turn["continues"], true, "{turn}");
}

/// A stop that took back the promised turn and then failed to end the exchange
/// leaves the turn owing its completion: the redrive's step sends it, once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_that_cannot_end_the_exchange_leaves_its_completion_owed() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static LISTED: AtomicBool = AtomicBool::new(false);
    static CANCELLED: AtomicBool = AtomicBool::new(false);
    fn wake_list(function: &str, data: &Value) -> bool {
        function == "harness::state::list"
            && data["scope"] == crate::state::BINDING_SCOPE
            && !LISTED.swap(true, Ordering::SeqCst)
    }
    fn cancel_write(function: &str, data: &Value) -> bool {
        function == "state::set"
            && data["scope"] == "harness_turn"
            && data["value"]["status"] == "cancelled"
            && !CANCELLED.swap(true, Ordering::SeqCst)
    }
    let faults: [fn(&str, &Value) -> bool; 2] = [wake_list, cancel_write];
    for refuse in faults {
        let hooks = json!({ "harness::on-turn-completed": {} });
        let ((stopped, after, steps), calls, turn) = run_on_faulty_engine(
            json!({}),
            json!([]),
            None,
            hooks,
            refuse,
            |deps| async move {
                let cfg = deps.cfg().await;
                let mut record =
                    crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
                        .await
                        .unwrap()
                        .unwrap();
                record.status = TurnStatus::Completed;
                record.continues = true;
                crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms)
                    .await
                    .unwrap();
                let req = crate::functions::stop::StopRequest {
                    session_id: "s".into(),
                    turn_id: None,
                    quiet_deadlines: false,
                };
                let stopped = crate::functions::stop::stop_one(&deps, req).await;
                let after =
                    crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
                        .await
                        .unwrap()
                        .unwrap();
                let first = step_one(deps.clone()).await;
                (stopped, after, [first, step_one(deps.clone()).await])
            },
        )
        .await;
        assert!(stopped.is_err());
        assert!(after.abort && !after.continues, "{after:?}");
        assert!(after.awaits_step(), "{after:?}");
        assert!(crate::inflight::is_orphan_candidate(
            &after,
            false,
            AgentMessage::now_ms()
        ));
        for step in steps {
            assert!(step.unwrap().skipped);
        }
        assert_eq!(turn["turn_id"], "t1", "{turn}");
        assert_eq!(turn["status"], "cancelled", "{turn}");
        assert_eq!(turn["exchange_finished"], true, "{turn}");
        assert!(turn.get("continues").is_none(), "{turn}");
        assert!(turn.get("owes_completion").is_none(), "{turn}");
        let completed: Vec<(Value, Value)> = calls
            .iter()
            .filter(|(function, _)| function == "harness::on-turn-completed")
            .map(|(_, data)| (data["status"].clone(), data["terminal"].clone()))
            .collect();
        assert_eq!(completed, [(json!("cancelled"), json!(true))]);
        assert_eq!(completions(&calls), [(json!("t1"), json!(true))]);
        assert!(
            !calls
                .iter()
                .any(
                    |(function, data)| (function == "harness::turn" || function == "state::set")
                        && reseeds(data)
                ),
            "{calls:?}"
        );
    }
}

/// On the live finalize path the reseed holds the session lock with the
/// terminal write, so a stop that read the finished turn first finds the turn
/// it seeded once it gets the lock: that turn is the one it came for.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_that_waited_out_the_reseed_stops_the_turn_it_seeded() {
    let (outcome, calls, turn) =
        run_on_fake_engine(json!({}), json!([]), None, json!({}), |deps| async move {
            let cfg = deps.cfg().await;
            let mut record =
                crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
                    .await
                    .unwrap()
                    .unwrap();
            let mut reseeded = record.clone();
            reseeded.turn_id = "t2".into();
            record.status = TurnStatus::Completed;
            record.continues = true;
            crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms)
                .await
                .unwrap();
            let guard = deps.locks.guard("s").await;
            let stopping = tokio::spawn({
                let deps = deps.clone();
                async move {
                    let req = crate::functions::stop::StopRequest {
                        session_id: "s".into(),
                        turn_id: None,
                        quiet_deadlines: true,
                    };
                    crate::functions::stop::stop_one(&deps, req).await
                }
            });
            tokio::time::sleep(Duration::from_millis(200)).await;
            crate::state::put_turn(&deps.iii, &reseeded, cfg.session_timeout_ms)
                .await
                .unwrap();
            drop(guard);
            stopping.await.unwrap().unwrap()
        })
        .await;
    assert!(outcome.stopping);
    assert_eq!(turn["turn_id"], "t2", "{turn}");
    assert_eq!(turn["abort"], true, "{turn}");
    let finished = calls
        .iter()
        .position(|(function, data)| {
            function == "state::set"
                && data["scope"] == "harness_turn"
                && data["value"]["continues"] == true
        })
        .unwrap();
    let seeded = calls
        .iter()
        .position(|(function, data)| {
            function == "state::set"
                && data["scope"] == "harness_turn"
                && data["value"]["turn_id"] == "t2"
        })
        .unwrap();
    assert!(
        calls[finished..seeded]
            .iter()
            .any(|(function, data)| function.ends_with("state::get")
                && data["scope"] == "harness_turn"),
        "the stop read the finished turn before the reseed: {calls:?}"
    );
}

/// A store that fails the settling re-read after the reseed failed leaves the
/// redelivery unacknowledged: it is the only delivery that can still finish the
/// promised turn, so its retry does.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redelivered_finished_step_retries_when_settlement_cannot_reread_the_turn() {
    use std::sync::atomic::{AtomicU8, Ordering};
    // 0: refuse the reseed's write, then 1: refuse turn reads, 2: refuse nothing.
    static STAGE: AtomicU8 = AtomicU8::new(0);
    fn refuse(function: &str, data: &Value) -> bool {
        match STAGE.load(Ordering::SeqCst) {
            0 if function == "state::set" && reseeds(data) => {
                STAGE.store(1, Ordering::SeqCst);
                true
            }
            1 => function.ends_with("state::get") && data["scope"] == "harness_turn",
            _ => false,
        }
    }
    let hooks = json!({ "harness::on-turn-completed": {} });
    let ((first, settled, second), calls, turn) = run_on_faulty_engine(
        json!({}),
        json!([]),
        None,
        hooks,
        refuse,
        |deps| async move {
            let cfg = deps.cfg().await;
            let mut record =
                crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
                    .await
                    .unwrap()
                    .unwrap();
            record.status = TurnStatus::Completed;
            record.continues = true;
            crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms)
                .await
                .unwrap();
            let first = step_one(deps.clone()).await;
            STAGE.store(2, Ordering::SeqCst);
            let settled = crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
                .await
                .unwrap()
                .unwrap();
            let second = step_one(deps.clone()).await;
            (first, settled, second)
        },
    )
    .await;
    assert!(first.is_err(), "{first:?}");
    assert!(settled.continues, "nothing settled over the failed read");
    assert!(completions(&calls).is_empty(), "{calls:?}");
    assert!(second.unwrap().skipped);
    assert_ne!(turn["turn_id"], "t1", "{turn}");
    assert_eq!(turn["status"], "running", "{turn}");
    assert!(turn.get("continues").is_none(), "{turn}");
}

/// A finished turn left promising a reseed, with no redelivery coming, is an
/// orphan: the redrive re-enqueues its step and that step seeds the turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_orphan_redrive_redelivers_a_finished_turn_that_still_promises_one() {
    let hooks = json!({ "harness::on-turn-completed": {} });
    let ((redriven, step), calls, turn) =
        run_on_fake_engine(json!({}), json!([]), None, hooks, |deps| async move {
            let cfg = deps.cfg().await;
            let mut record =
                crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
                    .await
                    .unwrap()
                    .unwrap();
            record.status = TurnStatus::Completed;
            record.continues = true;
            record.updated_at = 1;
            crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms)
                .await
                .unwrap();
            let redriven = crate::inflight::redrive_orphans(&deps).await.unwrap();
            (redriven, step_one(deps.clone()).await)
        })
        .await;
    assert_eq!(redriven, 1, "{calls:?}");
    assert!(
        calls
            .iter()
            .any(|(function, data)| function == "harness::turn"
                && data["turn_id"] == "t1"
                && data["step"] == 1),
        "{calls:?}"
    );
    assert!(step.unwrap().skipped);
    assert_ne!(turn["turn_id"], "t1", "{turn}");
    assert_eq!(turn["status"], "running", "{turn}");
    assert!(turn.get("continues").is_none(), "{turn}");
}

/// A prompt body the store failed to read is not lost: the redelivery fails
/// without settling, and its retry reloads the prompt and seeds the turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redelivered_finished_step_retries_when_its_prompt_cannot_be_read() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static REFUSING: AtomicBool = AtomicBool::new(true);
    fn refuse(function: &str, data: &Value) -> bool {
        REFUSING.load(Ordering::SeqCst)
            && function == "state::get"
            && data["scope"] == crate::state::PROMPT_SCOPE
    }
    let hooks = json!({ "harness::on-turn-completed": {} });
    let ((first, settled, second), calls, turn) = run_on_faulty_engine(
        json!({}),
        json!([]),
        None,
        hooks,
        refuse,
        |deps| async move {
            let cfg = deps.cfg().await;
            let mut record =
                crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
                    .await
                    .unwrap()
                    .unwrap();
            record.status = TurnStatus::Completed;
            record.continues = true;
            // The body as another harness process (or this one before a
            // restart) stored it: no process cache answers for the store.
            let prompt = "a_redelivered_finished_step_retries_when_its_prompt_cannot_be_read";
            let digest = crate::state::prompt_digest(prompt);
            let body = json!({ "body": prompt, "created_at": 1 });
            let at = crate::state::PROMPT_SCOPE;
            crate::state::state_set(&deps.iii, at, &digest, body, cfg.session_timeout_ms)
                .await
                .unwrap();
            record.options.system_prompt_ref = Some(digest);
            crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms)
                .await
                .unwrap();
            let first = step_one(deps.clone()).await;
            let settled = crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
                .await
                .unwrap()
                .unwrap();
            REFUSING.store(false, Ordering::SeqCst);
            let second = step_one(deps.clone()).await;
            (first, settled, second)
        },
    )
    .await;
    assert!(first.is_err(), "{first:?}");
    assert!(settled.continues, "nothing settled over the failed read");
    assert!(completions(&calls).is_empty(), "{calls:?}");
    assert!(second.unwrap().skipped);
    assert_ne!(turn["turn_id"], "t1", "{turn}");
    assert_eq!(turn["status"], "running", "{turn}");
    assert!(turn.get("continues").is_none(), "{turn}");
}

/// Any stop takes back the promised turn, quiet or not. With a wake left the
/// exchange is not over: the stop only records the abort, ends nothing, and
/// still keeps the redelivery from seeding the turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_with_a_wake_left_takes_back_the_promised_turn_and_ends_nothing() {
    let hooks = json!({ "harness::on-turn-completed": {} });
    let ((stopping, result), calls, turn) =
        run_on_fake_engine(json!({}), json!([]), None, hooks, |deps| async move {
            let cfg = deps.cfg().await;
            let mut record =
                crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
                    .await
                    .unwrap()
                    .unwrap();
            record.status = TurnStatus::Completed;
            record.continues = true;
            crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms)
                .await
                .unwrap();
            let wake = json!({"id": "w", "owner": {"session_id": "s"},
                "target": {"function_id": crate::functions::SEND_ID},
                "lifecycle": {"once": true}, "created_at": 1});
            crate::state::state_set(
                &deps.iii,
                crate::state::BINDING_SCOPE,
                "w",
                wake,
                cfg.session_timeout_ms,
            )
            .await
            .unwrap();
            let req = crate::functions::stop::StopRequest {
                session_id: "s".into(),
                turn_id: None,
                quiet_deadlines: false,
            };
            let outcome = crate::functions::stop::stop_one(&deps, req).await.unwrap();
            (outcome.stopping, step_one(deps.clone()).await)
        })
        .await;
    assert!(stopping);
    assert!(result.unwrap().skipped);
    assert_eq!(turn["turn_id"], "t1", "{turn}");
    assert_eq!(turn["status"], "completed", "{turn}");
    assert!(turn.get("continues").is_none(), "{turn}");
    assert_eq!(turn["abort"], true, "{turn}");
    assert_ne!(turn["exchange_finished"], true, "{turn}");
    assert!(turn.get("owes_completion").is_none(), "{turn}");
    assert!(completions(&calls).is_empty(), "{calls:?}");
    assert!(
        !calls
            .iter()
            .any(
                |(function, data)| (function == "harness::turn" || function == "state::set")
                    && reseeds(data)
            ),
        "{calls:?}"
    );
}

/// Store `t1` finished with `status` and `continues`, as a finalize leaves it
/// when the process dies before its reseed, then (after a quiet session stop
/// when `stop`) deliver its step 1 again, refusing every engine call `refuse`
/// matches. Returns the step outcome and the status after it, the engine calls
/// and the stored turn.
async fn redeliver_finished(
    status: TurnStatus,
    continues: bool,
    stop: bool,
    refuse: fn(&str, &Value) -> bool,
) -> (
    (
        Result<TurnStepResult, HarnessError>,
        crate::functions::status::StatusReport,
    ),
    Vec<(String, Value)>,
    Value,
) {
    let hooks = json!({ "harness::on-turn-completed": {} });
    run_on_faulty_engine(
        json!({}),
        json!([]),
        None,
        hooks,
        refuse,
        |deps| async move {
            let cfg = deps.cfg().await;
            let mut record =
                crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
                    .await
                    .unwrap()
                    .unwrap();
            record.status = status;
            record.continues = continues;
            crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms)
                .await
                .unwrap();
            if stop {
                let req = crate::functions::stop::StopRequest {
                    session_id: "s".into(),
                    turn_id: None,
                    quiet_deadlines: true,
                };
                let outcome = crate::functions::stop::stop_one(&deps, req).await.unwrap();
                assert!(outcome.stopping);
            }
            let result = step_one(deps.clone()).await;
            let req = crate::functions::status::StatusRequest {
                session_id: "s".into(),
                verbose: false,
            };
            let report = crate::functions::status::handle(&deps, req)
                .await
                .unwrap()
                .unwrap();
            (result, report)
        },
    )
    .await
}

/// Whether an engine call is made for a turn other than `t1`: a reseed's.
fn reseeds(data: &Value) -> bool {
    data["value"]["turn_id"]
        .as_str()
        .or(data["turn_id"].as_str())
        .is_some_and(|id| id != "t1")
}

/// Finalize `t1` with a wake queued, completed or `failed`, refusing every
/// engine call `refuse` matches; returns the status after it, the engine
/// calls and the stored turn.
async fn finalize_with_a_queued_wake(
    refuse: fn(&str, &Value) -> bool,
    failed: bool,
) -> (
    crate::functions::status::StatusReport,
    Vec<(String, Value)>,
    Value,
) {
    let hooks = json!({ "harness::on-turn-completed": {} });
    run_on_faulty_engine(
        json!({}),
        json!([]),
        None,
        hooks,
        refuse,
        |deps| async move {
            finalize_t1_with_a_queued_wake(&deps, failed).await;
            let req = crate::functions::status::StatusRequest {
                session_id: "s".into(),
                verbose: false,
            };
            crate::functions::status::handle(&deps, req)
                .await
                .unwrap()
                .unwrap()
        },
    )
    .await
}

async fn finalize_t1_with_a_queued_wake(deps: &Deps, failed: bool) {
    let cfg = deps.cfg().await;
    let wake = crate::state::QueuedMessage {
        id: "q_wake".into(),
        session_id: "s".into(),
        message: AgentMessage::user_text("[notification] woke"),
        entry_id: "e_fire_sub_1".into(),
        origin: Some(json!({ "notification": true, "binding": "sub" })),
        queued_at: 1,
        channel: None,
    };
    crate::state::enqueue_message(&deps.iii, &wake, cfg.session_timeout_ms)
        .await
        .unwrap();
    let mut record = crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
        .await
        .unwrap()
        .unwrap();
    let session = deps.session().await;
    let finalized = if failed {
        finalize_failed(deps, &session, &mut record, "boom", INTERNAL_FAILURE).await
    } else {
        finalize_completed(deps, &session, &mut record, None).await
    };
    assert!(finalized.unwrap().status.is_terminal());
}

/// Whether an engine call writes `t1` settled: completed, no longer continuing.
fn settles(data: &Value) -> bool {
    data["scope"] == "harness_turn"
        && data["value"]["turn_id"] == "t1"
        && data["value"]["status"] == "completed"
        && data["value"].get("continues").is_none()
}

/// `(turn_id, terminal)` of every `harness::on-turn-completed`, in order.
fn completions(calls: &[(String, Value)]) -> Vec<(Value, Value)> {
    calls
        .iter()
        .filter(|(function, _)| function == "harness::on-turn-completed")
        .map(|(_, data)| (data["turn_id"].clone(), data["terminal"].clone()))
        .collect()
}

/// `continues` alone keeps `expects_wake` up once the wake's record is gone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_expects_a_wake_on_a_finished_turn_that_continues() {
    let (expects, _, _) =
        run_on_fake_engine(json!({}), json!([]), None, json!({}), |deps| async move {
            let cfg = deps.cfg().await;
            let mut record =
                crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
                    .await
                    .unwrap()
                    .unwrap();
            record.status = TurnStatus::Completed;
            let mut expects = Vec::new();
            for continues in [true, false] {
                record.continues = continues;
                crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms)
                    .await
                    .unwrap();
                let req = crate::functions::status::StatusRequest {
                    session_id: "s".into(),
                    verbose: false,
                };
                let report = crate::functions::status::handle(&deps, req).await.unwrap();
                expects.push(report.unwrap().expects_wake);
            }
            expects
        })
        .await;
    assert_eq!(expects, [true, false], "no binding is stored");
}

/// The ide's post_call answer after a `git worktree add`: move the session to
/// `root`.
fn moves_to(root: &str) -> Value {
    json!({"harness::hook::on-post-call": {"decision": "continue",
        "target": "shell::turns::on-post-call",
        "mutations": {"fs_scope": {"root": root}}}})
}

/// A reply calling `shell::exec` (`call_wt`) then `coder::read-file`
/// (`call_next`), after a `call_held` already held when `held`.
fn worktree_reply(held: bool) -> Value {
    let call = |id: &str, function: &str| {
        json!({"type": "function_call", "id": id, "function_id": "agent_call",
            "arguments": {"function": function, "payload": {}}})
    };
    let mut content = vec![
        call("call_wt", "shell::exec"),
        call("call_next", "coder::read-file"),
    ];
    if held {
        content.insert(0, call("call_held", "shell::exec"));
    }
    json!({"role": "assistant", "stop_reason": "function_call",
        "model": "fake", "provider": "fake", "timestamp": 2,
        "usage": {"input": 10, "output": 2, "cost_usd": 0.1}, "content": content})
}

/// Store `t1` rooted at `root` (unscoped when `None`).
async fn root_turn(deps: &Deps, root: Option<&str>) {
    let cfg = deps.cfg().await;
    let mut record = crate::state::get_turn_unhydrated(&deps.iii, "s", cfg.session_timeout_ms)
        .await
        .unwrap()
        .unwrap();
    if let Some(root) = root {
        record.options.set_filesystem_root(root);
    }
    crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms)
        .await
        .unwrap();
}

/// Run step 1 of `t1` rooted at `root` under `boundary`.
async fn step_rooted(
    root: Option<&'static str>,
    boundary: crate::filesystem_scope::BoundaryMode,
    checkpoints: Value,
    reply: Value,
    hooks: Value,
) -> (
    Result<TurnStepResult, HarnessError>,
    Vec<(String, Value)>,
    Value,
) {
    run_on_fake_engine(
        checkpoints,
        json!([]),
        Some(reply),
        hooks,
        |deps| async move {
            let cfg = deps.cfg().await;
            *deps.config.write().await = Arc::new(crate::config::WorkerConfig {
                filesystem_boundary: boundary,
                ..(*cfg).clone()
            });
            root_turn(&deps, root).await;
            step_one(deps).await
        },
    )
    .await
}

/// The `fs_scope` stamped on the first call to `function`.
fn stamped<'a>(calls: &'a [(String, Value)], function: &str) -> &'a Value {
    let (_, data) = calls.iter().find(|(f, _)| f == function).unwrap();
    &data["fs_scope"]
}

/// The `session::set-meta` writes, in order.
fn meta_writes(calls: &[(String, Value)]) -> Vec<&Value> {
    calls
        .iter()
        .filter(|(f, _)| f == "session::set-meta")
        .map(|(_, data)| data)
        .collect()
}

/// A root the ide reports re-stamps the step's next call and the record, and
/// reaches the session metadata with the root it left once the record is
/// stored, merged into what the ADE stored there. Under `workspace` a
/// worktree outside the root and grants is followed too: the exec that
/// created it could already write there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_worktree_the_ide_reports_moves_the_session() {
    use crate::filesystem_scope::BoundaryMode;
    for (mode, boundary) in [
        (BoundaryMode::ConfiguredRoots, "configured_roots"),
        (BoundaryMode::Workspace, "workspace"),
    ] {
        // The ADE's own metadata (`metadataFor`): every other key survives.
        let mut hooks = moves_to("/repo.feat");
        hooks["session::get"] = json!({"meta": {"session_id": "s", "metadata": {
            "surface": "console", "model": "fake", "fs_scope": {"root": "/repo"}}}});
        let (result, calls, turn) =
            step_rooted(Some("/repo"), mode, json!({}), worktree_reply(false), hooks).await;
        assert_eq!(result.unwrap().next_step, Some(2));
        let scope = |root: &str| json!({"root": root, "grants": [], "boundary": boundary});
        assert_eq!(stamped(&calls, "shell::exec"), &scope("/repo"));
        assert_eq!(stamped(&calls, "coder::read-file"), &scope("/repo.feat"));
        assert_eq!(
            turn["options"]["metadata"]["fs_scope"],
            json!({"root": "/repo.feat"})
        );
        assert_eq!(
            meta_writes(&calls),
            [
                &json!({"session_id": "s", "metadata": {"surface": "console", "model": "fake",
                "fs_scope": {"root": "/repo.feat", "moved_from": "/repo"}}})
            ],
            "one mirror: the next call's equal root moves nothing"
        );
        let stored = calls.iter().position(|(f, data)| {
            f == "state::set"
                && data["value"]["options"]["metadata"]["fs_scope"]["root"] == "/repo.feat"
        });
        let mirrored = calls.iter().position(|(f, _)| f == "session::set-meta");
        assert!(
            stored.unwrap() < mirrored.unwrap(),
            "record first, mirror second"
        );
    }
}

/// No move on a failed call, on an unscoped session, or while another call of
/// the turn awaits approval; the last one tells the model why.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_worktree_report_moves_nothing_on_a_failure_an_unscoped_session_or_a_held_sibling() {
    let mut failed = moves_to("/repo.feat");
    failed["harness::hook::on-post-call"]["mutations"]["is_error"] = json!(true);
    let held = json!({"call_held": {"state": "pending", "function_id": "shell::exec",
        "held_by": "test::gate", "held_arguments": {}}});
    for (root, checkpoints, hooks) in [
        (Some("/repo"), json!({}), failed),
        (None, json!({}), moves_to("/repo.feat")),
        (Some("/repo"), held, moves_to("/repo.feat")),
    ] {
        let sibling_held = !checkpoints.as_object().unwrap().is_empty();
        let (_, calls, turn) = step_rooted(
            root,
            crate::filesystem_scope::BoundaryMode::ConfiguredRoots,
            checkpoints,
            worktree_reply(sibling_held),
            hooks,
        )
        .await;
        let root = root.map_or(Value::Null, |root| json!(root));
        assert_eq!(turn["options"]["metadata"]["fs_scope"]["root"], root);
        assert_eq!(stamped(&calls, "coder::read-file")["root"], root);
        assert!(meta_writes(&calls).is_empty(), "{calls:?}");
        let told = calls.iter().any(|(f, data)| {
            f == "session::append"
                && data.to_string().contains("call_wt")
                && data
                    .to_string()
                    .contains("did not move to /repo.feat: another call of this turn")
        });
        assert_eq!(told, sibling_held, "{calls:?}");
    }
}

/// An approved `git worktree add` follows on release.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_released_worktree_add_moves_the_session() {
    let held = json!({"call_wt": {"state": "pending", "function_id": "shell::exec",
        "held_by": "test::gate", "held_arguments": {}}});
    let (result, calls, turn) = run_on_fake_engine(
        held,
        json!([]),
        None,
        moves_to("/repo.feat"),
        |deps| async move {
            root_turn(&deps, Some("/repo")).await;
            crate::deferred::resolve(&deps, resolve_request("call_wt", "execute")).await
        },
    )
    .await;
    assert!(result.unwrap().turn_resumed);
    assert_eq!(stamped(&calls, "shell::exec")["root"], "/repo");
    assert_eq!(
        turn["options"]["metadata"]["fs_scope"]["root"],
        "/repo.feat"
    );
    assert_eq!(
        meta_writes(&calls),
        [&json!({"session_id": "s",
            "metadata": {"fs_scope": {"root": "/repo.feat", "moved_from": "/repo"}}})]
    );
}

/// A released move is mirrored even when its next step cannot be queued: the
/// record already holds the new root and a retried resolve finds the call
/// settled, so nothing else would mirror it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_released_move_is_mirrored_when_the_next_step_cannot_be_queued() {
    let held = json!({"call_wt": {"state": "pending", "function_id": "shell::exec",
        "held_by": "test::gate", "held_arguments": {}}});
    let (result, calls, turn) = run_on_faulty_engine(
        held,
        json!([]),
        None,
        moves_to("/repo.feat"),
        |f, _| f == "harness::turn",
        |deps| async move {
            root_turn(&deps, Some("/repo")).await;
            crate::deferred::resolve(&deps, resolve_request("call_wt", "execute")).await
        },
    )
    .await;
    assert!(result.is_err(), "{result:?}");
    assert_eq!(
        turn["options"]["metadata"]["fs_scope"]["root"],
        "/repo.feat"
    );
    assert_eq!(turn["calls"]["call_wt"]["state"], "done");
    assert_eq!(
        meta_writes(&calls),
        [&json!({"session_id": "s",
            "metadata": {"fs_scope": {"root": "/repo.feat", "moved_from": "/repo"}}})]
    );
}

/// A folder granted for the session is stamped only on a call in the implicit
/// namespace; the root goes with every call, and a release in another
/// namespace carries only the grants of that release.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_in_another_namespace_carries_no_session_folder_grant() {
    let granted = json!({"harness_filesystem_grants": {"roots": ["/granted"]}});
    let call = |id: &str, namespace: Option<&str>| {
        let mut call = json!({"type": "function_call", "id": id, "function_id": "agent_call",
            "arguments": {"function": "shell::exec", "payload": {}}});
        if let Some(namespace) = namespace {
            call["arguments"]["namespace"] = json!(namespace);
        }
        call
    };
    let mut reply = worktree_reply(false);
    reply["content"] = json!([call("call_own", None), call("call_zz", Some("zz-a"))]);
    let (_, calls, _) = step_rooted(
        Some("/repo"),
        crate::filesystem_scope::BoundaryMode::Auto,
        json!({}),
        reply,
        granted.clone(),
    )
    .await;
    let stamps: Vec<&Value> = calls
        .iter()
        .filter(|(f, _)| f == "shell::exec")
        .map(|(_, data)| &data["fs_scope"])
        .collect();
    assert_eq!(stamps.len(), 2, "{calls:?}");
    assert_eq!(stamps[0]["grants"], json!(["/granted"]));
    assert_eq!(stamps[1]["grants"], json!([]));
    assert!(
        stamps.iter().all(|stamp| stamp["root"] == "/repo"),
        "{stamps:?}"
    );

    let mut held = held_call();
    held["function_id"] = json!("shell::exec");
    held["namespace"] = json!("zz-a");
    let (_, calls, _) = run_on_fake_engine(
        json!({"call_hold": held}),
        json!([]),
        None,
        granted,
        |deps| async move {
            root_turn(&deps, Some("/repo")).await;
            let mut req = resolve_request("call_hold", "execute");
            req.fs_scope = Some(crate::functions::function_resolve::ResolveFsScope {
                grants: vec!["/once".into()],
            });
            crate::deferred::resolve(&deps, req).await
        },
    )
    .await;
    assert_eq!(stamped(&calls, "shell::exec")["grants"], json!(["/once"]));
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
        ..Default::default()
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
    run_on_fake_engine(checkpoints, json!([]), None, hooks, |deps| async move {
        crate::deferred::resolve(&deps, resolve_request("call_hold", "execute")).await
    })
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
    run_on_fake_engine(checkpoints, stored, None, json!({}), step_one).await
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

/// [`generate`] with `hooks` (hook function -> chain outcome) observed. An
/// outcome's `target` names its observer's target (default `test::gate`).
async fn generate_observed(
    reply: Value,
    hooks: Value,
) -> (
    Result<TurnStepResult, HarnessError>,
    Vec<(String, Value)>,
    Value,
) {
    run_on_fake_engine(json!({}), json!([]), Some(reply), hooks, step_one).await
}

async fn step_one(deps: Deps) -> Result<TurnStepResult, HarnessError> {
    let payload =
        serde_json::from_value(json!({"session_id": "s", "turn_id": "t1", "step": 1})).unwrap();
    run_step(&deps, payload).await
}

/// Without a `reply` the provider's stream channel cannot open: the step ends
/// at that boundary.
async fn run_on_fake_engine<T, F: std::future::Future<Output = T>>(
    checkpoints: Value,
    stored: Value,
    reply: Option<Value>,
    hooks: Value,
    drive: impl FnOnce(Deps) -> F,
) -> (T, Vec<(String, Value)>, Value) {
    run_on_faulty_engine(checkpoints, stored, reply, hooks, |_, _| false, drive).await
}

/// [`run_on_fake_engine`], refusing every engine call `refuse` matches.
async fn run_on_faulty_engine<T, F: std::future::Future<Output = T>>(
    checkpoints: Value,
    stored: Value,
    reply: Option<Value>,
    hooks: Value,
    refuse: fn(&str, &Value) -> bool,
    drive: impl FnOnce(Deps) -> F,
) -> (T, Vec<(String, Value)>, Value) {
    run_on_misleading_engine(
        checkpoints,
        stored,
        reply,
        hooks,
        refuse,
        |_, _| false,
        drive,
    )
    .await
}

/// The hook points a test's `hooks` map observes. Any other key
/// (`session::get`) only answers that function, and the key
/// `harness_filesystem_grants` holds the session's stored folder grants. Kept out of
/// `run_on_misleading_engine`, whose debug poll frame sits near the 2 MiB
/// test-thread stack.
fn observed_hook_points(hooks: &Value) -> Vec<String> {
    hooks
        .as_object()
        .unwrap()
        .keys()
        .filter(|id| id.starts_with("harness::hook::"))
        .cloned()
        .collect()
}

/// [`run_on_faulty_engine`], storing a refused turn write that `lands` matches all
/// the same: the error does not tell whether the write happened.
async fn run_on_misleading_engine<T, F: std::future::Future<Output = T>>(
    checkpoints: Value,
    stored: Value,
    reply: Option<Value>,
    hooks: Value,
    refuse: fn(&str, &Value) -> bool,
    lands: fn(&str, &Value) -> bool,
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
            "functions": {"allow": ["*"], "deny": ["zz-denied/test::hold"], "expose": "agent_call"}},
        "calls": checkpoints,
        "created_at": 1, "updated_at": 1
    })));
    let stored_turn = turn.clone();
    let generated_turn = turn.clone();
    let narrow_during_generate = hooks["narrow_during_generate"].as_bool() == Some(true);
    let observed_hooks = observed_hook_points(&hooks);
    let server = tokio::spawn(async move {
        let mut queue = serde_json::Map::new();
        let mut prompts = serde_json::Map::new();
        let mut bindings = serde_json::Map::new();
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
                if narrow_during_generate {
                    let mut turn = generated_turn.lock().unwrap();
                    turn["options"]["functions"] = json!({
                        "allow": ["session::get"], "deny": [], "expose": "agent_call"
                    });
                    turn["functions_acknowledged"] = json!([999]);
                }
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
                    let target = hooks[id]["target"].as_str().unwrap_or("test::gate");
                    // `targets` observes the point once per target, in chain order.
                    let observers: Vec<Value> = match hooks[id]["targets"].as_array() {
                        Some(targets) => targets
                            .iter()
                            .map(|t| json!({"id": format!("o_{id}#{}", t.as_str().unwrap()), "target": t}))
                            .collect(),
                        None => vec![json!({"id": format!("o_{id}"), "target": target})],
                    };
                    let registered = json!({"type": "functionregistered", "id": id, "revision": 1,
                        "observers": observers});
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
                .and_then(|id| id.split('#').next())
                .map_or(function, str::to_string);
            let data = message["data"].clone();
            {
                let mut observed = observed.lock().unwrap();
                observed.push((function.clone(), data.clone()));
                observed.push((
                    format!("namespace of {function}"),
                    message["namespace"].clone(),
                ));
            }
            if refuse(&function, &data) {
                if lands(&function, &data) {
                    *stored_turn.lock().unwrap() = data["value"].clone();
                }
                let reply = json!({
                    "type": "invocationresult", "invocation_id": message["invocation_id"],
                    "function_id": function,
                    "error": {"code": "UNAVAILABLE", "message": "store busy"}
                });
                let frame = tokio_tungstenite::tungstenite::Message::Text(reply.to_string().into());
                if socket.send(frame).await.is_err() {
                    break;
                }
                continue;
            }
            let result = match function.as_str() {
                hook if !hooks[hook].is_null() => hooks[hook].clone(),
                "engine::workers::register" => json!({"success": true}),
                "state::get" | "harness::state::get" if data["scope"] == "harness_turn" => {
                    stored_turn.lock().unwrap().clone()
                }
                "state::get"
                    if data["scope"] == crate::filesystem_grants::FILESYSTEM_GRANTS_SCOPE =>
                {
                    hooks[crate::filesystem_grants::FILESYSTEM_GRANTS_SCOPE].clone()
                }
                "state::set" if data["scope"] == "harness_turn" => {
                    *stored_turn.lock().unwrap() = data["value"].clone();
                    json!({})
                }
                "state::set" if data["scope"] == crate::state::QUEUE_SCOPE => {
                    let key = data["key"].as_str().unwrap().to_string();
                    queue.insert(key, data["value"].clone());
                    json!({})
                }
                "state::delete" if data["scope"] == crate::state::QUEUE_SCOPE => {
                    queue.remove(data["key"].as_str().unwrap());
                    json!({})
                }
                "state::list" if data["scope"] == crate::state::QUEUE_SCOPE => {
                    Value::Array(queue.values().cloned().collect())
                }
                "state::set" if data["scope"] == crate::state::PROMPT_SCOPE => {
                    let key = data["key"].as_str().unwrap().to_string();
                    prompts.insert(key, data["value"].clone());
                    json!({})
                }
                "state::get" if data["scope"] == crate::state::PROMPT_SCOPE => prompts
                    .get(data["key"].as_str().unwrap())
                    .cloned()
                    .unwrap_or(Value::Null),
                "state::set" if data["scope"] == crate::state::BINDING_SCOPE => {
                    let key = data["key"].as_str().unwrap().to_string();
                    bindings.insert(key, data["value"].clone());
                    json!({})
                }
                "harness::state::list" if data["scope"] == crate::state::BINDING_SCOPE => {
                    Value::Array(bindings.values().cloned().collect())
                }
                "state::list" | "harness::state::list" => json!([]),
                "state::list_keys" if data["scope"] == "harness_turn" => json!({"keys": ["s"]}),
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
    // A hook chain is only called once its observer is active.
    tokio::time::timeout(Duration::from_secs(5), async {
        while observed_hooks.iter().any(|id| !iii.is_observed(id)) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the hooks' observers activate");
    let result = tokio::time::timeout(Duration::from_secs(10), drive(deps)).await;
    iii.shutdown();
    server.abort();
    let calls = calls.lock().unwrap().clone();
    let turn = turn.lock().unwrap().clone();
    (result.unwrap(), calls, turn)
}

/// A reply that calls `harness::set-description` through `agent_call`, with
/// or without reply text beside it.
fn describe_reply(with_text: bool) -> Value {
    let mut content = Vec::new();
    if with_text {
        content.push(json!({"type": "text", "text": "Fixed the login form."}));
    }
    content.push(json!({"type": "function_call", "id": "call_describe",
        "function_id": "agent_call",
        "arguments": {"function": "harness::set-description", "description": "Describe",
            "payload": {"description": "Fixing   the login form", "title": "Login form fix"}}}));
    json!({"role": "assistant", "stop_reason": "function_call",
        "model": "fake", "provider": "fake", "timestamp": 2,
        "usage": {"input": 10, "output": 2, "cost_usd": 0.1},
        "content": content})
}

/// chat-projects §1.2.3, the terminal-step rule: once the turn did other
/// work, a step whose only call is `harness::set-description`, beside reply
/// text, ends the turn once its result is written. One session::patch-meta
/// carries the description; the model is offered the tool and is not asked
/// again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_description_beside_the_reply_ends_the_turn_after_other_work() {
    let (result, calls, turn) = run_on_fake_engine(
        json!({"call_read": {"state": "done", "function_id": "shell::exec"}}),
        json!([]),
        Some(describe_reply(true)),
        json!({}),
        step_one,
    )
    .await;
    assert_eq!(result.unwrap().status, TurnStatus::Completed, "{turn}");
    assert!(
        !calls
            .iter()
            .any(|(function, _)| function == "harness::turn"),
        "no next step is queued: {calls:?}"
    );
    let patches: Vec<&Value> = calls
        .iter()
        .filter(|(function, _)| function == "session::patch-meta")
        .map(|(_, data)| data)
        .collect();
    assert_eq!(patches.len(), 1, "{calls:?}");
    assert_eq!(patches[0]["session_id"], "s");
    assert_eq!(patches[0]["description"], "Fixing the login form");
    assert_eq!(patches[0]["set"]["description_by"], "agent");
    assert_eq!(patches[0]["title"], "Login form fix");
    assert_eq!(patches[0]["set"]["title_by"], "agent");
    assert!(
        !calls
            .iter()
            .any(|(function, _)| function == "session::set-meta"),
        "descriptions are merged, never written whole"
    );
    let tools = calls
        .iter()
        .find(|(function, _)| function == "router::chat")
        .map(|(_, data)| data["tools"].clone())
        .expect("the model was asked once");
    let names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_eq!(names, ["agent_call", "harness::set-description"]);
    let result = calls
        .iter()
        .find(|(function, data)| {
            function == "session::append" && data["message"]["role"] == "function_result"
        })
        .map(|(_, data)| data["message"].clone())
        .expect("the call's result is written");
    assert_eq!(result["function_id"], "harness::set-description");
    assert_eq!(result["details"]["ok"], true);
}

/// At the turn's start the same step goes on: its text can be an
/// announcement ("I'll check the tests first") that a model making one call
/// per step pairs with the description. The result tells the model to end the
/// turn itself when the text was its answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_description_beside_the_reply_at_the_turns_start_asks_the_model_again() {
    let (result, calls, turn) = generate(describe_reply(true)).await;
    assert_eq!(result.unwrap().next_step, Some(2));
    assert!(
        calls
            .iter()
            .any(|(function, _)| function == "harness::turn"),
        "the next step is queued: {calls:?}"
    );
    let result = calls
        .iter()
        .find(|(function, data)| {
            function == "session::append" && data["message"]["role"] == "function_result"
        })
        .map(|(_, data)| data["message"].clone())
        .expect("the call's result is written");
    assert_eq!(result["details"]["ok"], true);
    assert_eq!(result["details"]["next"], crate::description::GO_ON_NOTE);
    assert_eq!(
        turn["calls"]["call_describe"]["function_id"], "harness::set-description",
        "the next step must not count the description as other work: {turn}"
    );
}

fn empty_reply() -> Value {
    json!({"role": "assistant", "stop_reason": "end",
        "model": "fake", "provider": "fake", "timestamp": 3,
        "usage": {"input": 10, "output": 0, "cost_usd": 0.1},
        "content": []})
}

/// After an opening description, an empty end keeps the opening reply as the
/// turn's result: the note asked for that empty end when the reply was the
/// answer (the fake engine serves the earlier message as "done").
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_end_after_an_opening_description_keeps_the_opening_reply() {
    let (result, _, turn) = run_on_fake_engine(
        json!({"call_describe": {"state": "done", "function_id": "harness::set-description",
            "entry_id": "e_t1_call_describe"}}),
        json!([]),
        Some(empty_reply()),
        json!({}),
        step_one,
    )
    .await;
    assert_eq!(result.unwrap().status, TurnStatus::Completed, "{turn}");
    assert_eq!(turn["result"], "done", "{turn}");
}

/// After other work an empty end is the turn's own (empty) result: an
/// announcement is never taken for the answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_end_after_other_work_keeps_its_empty_result() {
    let (result, _, turn) = run_on_fake_engine(
        json!({"call_describe": {"state": "done", "function_id": "harness::set-description",
            "entry_id": "e_t1_call_describe"},
            "call_read": {"state": "done", "function_id": "shell::exec"}}),
        json!([]),
        Some(empty_reply()),
        json!({}),
        step_one,
    )
    .await;
    assert_eq!(result.unwrap().status, TurnStatus::Completed, "{turn}");
    assert_eq!(turn["result"], "", "{turn}");
}

/// The same call with no reply text continues like any tool step.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_description_alone_in_its_step_asks_the_model_again() {
    let (result, calls, _) = generate(describe_reply(false)).await;
    assert_eq!(result.unwrap().next_step, Some(2));
    assert!(
        calls
            .iter()
            .any(|(function, _)| function == "harness::turn")
    );
}
