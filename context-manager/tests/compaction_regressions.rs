//! `/compact` regressions from the console-session investigation: a parent
//! session coordinating many sub-agent jobs, where compaction failed
//! intermittently and, when it worked, the context meter never moved.
//!
//! Every test drives the real `context::compact` handler over deterministic
//! fakes — no engine, no model.
//!
//! Each test names the defect it pins; the rest pin the contracts the fixes
//! rely on.

#[path = "common/fakes.rs"]
#[allow(dead_code)]
mod fakes;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use context_manager::adapters::fs_lease::FsLeaseStore;
use context_manager::config::WorkerConfig;
use context_manager::core::estimate::{Estimator, HeuristicEstimator};
use context_manager::core::lease::LEASE_SCOPE;
use context_manager::functions::compact;
use context_manager::ports::{
    lease_cell, Deps, LeaseRecord, LeaseStore, SummarizeError, SummarizeRequest, Summarizer,
};
use context_manager::types::{AgentMessage, ContentBlock, Model};
use fakes::{FakeClock, FakeModelResolver, FakeSummarizer, InMemoryLeaseStore, SummarizerScript};
use serde_json::{json, Value};
use tokio::sync::RwLock;

/// Inline limits the tool-heavy tests use: usable input
/// 200_000 - 8_000 - min(20_000, 10%) = 172_000; verbatim-tail budget 8_000.
const WINDOW: u64 = 200_000;
const MAX_OUTPUT: u64 = 8_000;
const USABLE: u64 = 172_000;

fn deps(summarizer: Arc<dyn Summarizer>) -> Deps {
    deps_with(
        summarizer,
        Arc::new(FakeModelResolver::new()),
        Arc::new(InMemoryLeaseStore::new()),
    )
}

fn deps_with(
    summarizer: Arc<dyn Summarizer>,
    resolver: Arc<FakeModelResolver>,
    leases: Arc<dyn LeaseStore>,
) -> Deps {
    Deps {
        config: Arc::new(RwLock::new(Arc::new(WorkerConfig::default()))),
        resolver,
        summarizer,
        leases: lease_cell(leases),
        clock: Arc::new(FakeClock::new()),
    }
}

fn user(text: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "text", "text": text }], "timestamp": 1 })
}

fn assistant(text: &str) -> Value {
    json!({
        "role": "assistant", "content": [{ "type": "text", "text": text }],
        "stop_reason": "end", "model": "m", "provider": "p", "timestamp": 2
    })
}

fn call(id: &str, function_id: &str) -> Value {
    json!({
        "role": "assistant",
        "content": [{ "type": "function_call", "id": id, "function_id": function_id, "arguments": {} }],
        "stop_reason": "function_call", "model": "m", "provider": "p", "timestamp": 3
    })
}

fn result(id: &str, function_id: &str, chars: usize) -> Value {
    json!({
        "role": "function_result", "function_call_id": id, "function_id": function_id,
        "content": [{ "type": "text", "text": "r".repeat(chars) }], "timestamp": 4
    })
}

/// A sub-agent completion as the harness injects it into the parent:
/// a plain user message (`trigger_deliver::wake_target`).
fn notification(child: &str) -> Value {
    user(&format!(
        "[notification] {child}: {{\"event_type\":\"state:updated\",\"key\":\"{child}\",\"scope\":\"results\"}}"
    ))
}

fn tokens(messages: &[Value]) -> u64 {
    messages
        .iter()
        .map(|m| {
            HeuristicEstimator.message(&serde_json::from_value::<AgentMessage>(m.clone()).unwrap())
        })
        .sum()
}

async fn compact_json(deps: &Deps, messages: &[Value], model: Value, options: Value) -> Value {
    let request = serde_json::from_value(json!({
        "messages": messages, "model": model, "options": options
    }))
    .unwrap();
    serde_json::to_value(compact::handle(deps, request).await.unwrap()).unwrap()
}

fn inline_model() -> Value {
    json!({ "id": "claude-x", "provider": "anthropic",
            "limits": { "context_window": WINDOW, "max_output_tokens": MAX_OUTPUT } })
}

fn is_safe_cut(message: &Value) -> bool {
    match message["role"].as_str() {
        Some("assistant") => true,
        Some("user") => !message["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|block| block["type"] == "function_result"),
        _ => false,
    }
}

/// One user prompt followed by an orchestrator run of `spawns` sub-agent
/// calls, each answered by a large result, with no closing assistant text:
/// the turn was stopped, failed, or overflowed mid-run.
fn single_tool_heavy_turn(spawns: usize, result_chars: usize) -> Vec<Value> {
    let mut messages = vec![user(
        "Implement the spec and coordinate executor/reviewer sub-agents.",
    )];
    for i in 0..spawns {
        let id = format!("c{i}");
        messages.push(call(&id, "harness::spawn"));
        messages.push(result(&id, "harness::spawn", result_chars));
    }
    messages
}

/// A result answered in several text blocks, each under the summariser's
/// per-block cut, together well over `max_result_tokens` (20,000).
#[tokio::test]
async fn a_result_split_across_blocks_reaches_the_summariser_under_the_result_cap() {
    let blocks: Vec<Value> = (0..5)
        .map(|i| json!({ "type": "text", "text": format!("{i}").repeat(60_000) }))
        .collect();
    let messages = vec![
        user("Collect the shards."),
        call("c0", "shards::read"),
        json!({
            "role": "function_result", "function_call_id": "c0", "function_id": "shards::read",
            "content": blocks, "timestamp": 4
        }),
        assistant("Collected."),
        user("q2"),
        assistant("a2"),
        user("q3"),
    ];

    let summarizer = Arc::new(FakeSummarizer::new());
    let response = compact_json(
        &deps(summarizer.clone()),
        &messages,
        inline_model(),
        json!({}),
    )
    .await;

    assert_eq!(response["status"], "ok", "{response}");
    let prompt: String = summarizer
        .calls()
        .iter()
        .map(|call| call.user_prompt.as_str())
        .collect();
    // Capped as a whole, as assemble caps it: head, marker, tail.
    assert!(prompt.contains("result capped"), "no cap marker");
    assert!(prompt.contains("0000") && prompt.contains("4444"));
    assert!(
        HeuristicEstimator.text(&prompt) <= 20_000 + 1_000,
        "{} estimated tokens reached the summariser",
        HeuristicEstimator.text(&prompt)
    );
}

// ---------------------------------------------------------------------------
// The verbatim-tail selection
// ---------------------------------------------------------------------------

/// `select()` only falls back to *turn starts* when no suffix fits the 8k
/// verbatim budget. A single long run (one user prompt, many large results)
/// has no turn start above index 0, so the head is empty and `/compact`
/// answers `empty` — "session is too small to summarise" — for a history of
/// ~120k tokens. Safe cuts exist (every assistant message); none is tried.
#[tokio::test]
async fn a_single_tool_heavy_turn_is_compacted_instead_of_reported_empty() {
    let messages = single_tool_heavy_turn(12, 40_000);
    let total = tokens(&messages);
    assert!(
        total > 100_000,
        "fixture is a large history ({total} tokens)"
    );

    let summarizer = Arc::new(FakeSummarizer::new());
    let response = compact_json(
        &deps(summarizer.clone()),
        &messages,
        inline_model(),
        json!({}),
    )
    .await;

    assert_eq!(
        response["status"], "ok",
        "a {total}-token history must compact, got {response}"
    );
    let tail = response["tail_start_index"]
        .as_u64()
        .expect("a verbatim tail") as usize;
    assert!(
        tail > 0 && is_safe_cut(&messages[tail]),
        "tail must start at a safe cut, got {tail}"
    );
    assert!(response["tokens_after"].as_u64().unwrap() < total);
    assert_eq!(summarizer.calls().len(), 1);
}

/// The same run closed by an assistant reply compacts today: the closing
/// message is a safe cut that fits the verbatim budget. Pinned so the
/// contrast with the stopped run above stays visible.
#[tokio::test]
async fn a_tool_heavy_turn_that_closed_with_a_reply_compacts() {
    let mut messages = single_tool_heavy_turn(12, 40_000);
    messages.push(assistant("All sub-agents reported back."));

    let response = compact_json(
        &deps(Arc::new(FakeSummarizer::new())),
        &messages,
        inline_model(),
        json!({}),
    )
    .await;

    assert_eq!(response["status"], "ok", "{response}");
    assert_eq!(response["tail_start_index"], json!(messages.len() - 1));
}

/// The case that sends users to `/compact`: the last turn overflowed the
/// window (the harness failed it with `context/overflow`). Selection keeps
/// that whole turn verbatim when no suffix fits the verbatim budget, so the
/// compaction "succeeds" by summarising only the small turns before it and
/// the window is still over budget — the next turn fails the same way.
#[tokio::test]
async fn compacting_after_an_oversized_turn_brings_the_window_under_budget() {
    let mut messages = vec![
        user("Plan the migration."),
        assistant("Here is the plan."),
        user("Run the whole migration with sub-agents."),
    ];
    for i in 0..18 {
        let id = format!("m{i}");
        messages.push(call(&id, "harness::spawn"));
        messages.push(result(&id, "harness::spawn", 48_000));
    }
    let total = tokens(&messages);
    assert!(
        total > USABLE,
        "fixture overflows the usable budget ({total} > {USABLE})"
    );

    let response = compact_json(
        &deps(Arc::new(FakeSummarizer::new())),
        &messages,
        inline_model(),
        json!({}),
    )
    .await;

    assert_eq!(response["status"], "ok", "{response}");
    let after = response["tokens_after"].as_u64().unwrap();
    assert!(
        after <= USABLE,
        "compaction left {after} tokens for a {USABLE}-token budget (tail starts at {})",
        response["tail_start_index"]
    );
}

/// A parent session shaped like the investigated one: one prompt, then a
/// wake turn per sub-agent notification, each relaying a large report.
/// Compaction must summarise most of it and keep a small recent tail.
#[tokio::test]
async fn an_orchestrator_session_with_many_child_notifications_compacts_to_a_small_tail() {
    let mut messages = vec![user(
        "Coordinate OBS-01..OBS-06 with executor and reviewer sub-agents.",
    )];
    for i in 0..10 {
        messages.push(call(&format!("s{i}"), "harness::spawn"));
        messages.push(result(&format!("s{i}"), "harness::spawn", 200));
    }
    messages.push(assistant("Spawned every executor."));
    for i in 0..16 {
        messages.push(notification(&format!("obs0{}-exec-r{i}", i % 6)));
        messages.push(call(&format!("k{i}"), "kanban::ticket::get"));
        messages.push(result(&format!("k{i}"), "kanban::ticket::get", 60_000));
        messages.push(assistant("Reviewed the round; next steps queued."));
    }
    let total = tokens(&messages);

    let response = compact_json(
        &deps(Arc::new(FakeSummarizer::new())),
        &messages,
        inline_model(),
        json!({}),
    )
    .await;

    assert_eq!(response["status"], "ok", "{response}");
    let after = response["tokens_after"].as_u64().unwrap();
    assert!(
        after <= USABLE && after * 10 < total,
        "tail too large: {after} of {total}"
    );
}

// ---------------------------------------------------------------------------
// Status mapping
// ---------------------------------------------------------------------------

/// A blank summary is a summariser failure, yet it maps to `empty` — the
/// same status as "nothing to compact" — so the console tells the user the
/// session is too small to summarise.
#[tokio::test]
async fn a_blank_summary_is_a_failure_not_an_empty_history() {
    let messages = vec![
        user("first question"),
        assistant(&"a".repeat(40_000)),
        user("second question"),
        assistant(&"b".repeat(40_000)),
        user("third question"),
        assistant("short answer"),
    ];
    let summarizer = Arc::new(FakeSummarizer::new());
    summarizer.set_script(SummarizerScript::Return("  \n ".into()));

    let response = compact_json(
        &deps(summarizer.clone()),
        &messages,
        inline_model(),
        json!({}),
    )
    .await;

    assert_eq!(summarizer.calls().len(), 1, "the summariser ran");
    assert_ne!(
        response["status"], "empty",
        "a summariser that returned nothing must not read as an empty history"
    );
}

// ---------------------------------------------------------------------------
// Summariser input budget
// ---------------------------------------------------------------------------

/// The provider model behind the router: a 200k window with a 64k output
/// allowance, which the router forwards as `max_tokens` when the summariser
/// request names none (llm-router `resolve_max_output_tokens`).
const PROVIDER_WINDOW: u64 = 200_000;
const PROVIDER_MAX_OUTPUT: u64 = 64_000;

/// Accepts a request only when its input plus the forwarded output
/// allowance fits the provider window, as the provider API does.
struct ProviderWindowSummarizer {
    calls: Mutex<Vec<u64>>,
}

#[async_trait]
impl Summarizer for ProviderWindowSummarizer {
    async fn summarize(&self, req: SummarizeRequest) -> Result<String, SummarizeError> {
        let message = AgentMessage::User {
            content: vec![ContentBlock::Text {
                text: req.user_prompt.clone(),
            }],
            timestamp: i64::MAX,
        };
        let input =
            HeuristicEstimator.text(&req.system_prompt) + HeuristicEstimator.message(&message);
        self.calls.lock().unwrap().push(input);
        if input + PROVIDER_MAX_OUTPUT > PROVIDER_WINDOW {
            return Err(SummarizeError::Failed("prompt is too long".into()));
        }
        Ok("## Goal\n- summary".into())
    }
}

/// Long prose turns: function results are cut to 2,000 chars in the
/// summariser prompt, so only text makes a large prompt.
fn long_prose_history() -> Vec<Value> {
    let mut messages = Vec::new();
    for i in 0..12 {
        messages.push(user(&format!("{i}: {}", "q".repeat(40_000))));
        messages.push(assistant(&format!("{i}: {}", "a".repeat(40_000))));
    }
    messages
}

fn provider_model() -> Model {
    serde_json::from_value(json!({
        "id": "claude-x", "provider": "anthropic",
        "context_window": PROVIDER_WINDOW, "max_output_tokens": PROVIDER_MAX_OUTPUT
    }))
    .unwrap()
}

/// Router-resolved limits size every summariser fragment to what the
/// provider accepts: `context::assemble` in the harness compacts this way.
#[tokio::test]
async fn router_resolved_limits_keep_summariser_fragments_inside_the_provider_window() {
    let resolver = Arc::new(FakeModelResolver::new());
    resolver.insert(provider_model());
    let summarizer = Arc::new(ProviderWindowSummarizer {
        calls: Mutex::new(Vec::new()),
    });
    let deps = deps_with(
        summarizer.clone(),
        resolver,
        Arc::new(InMemoryLeaseStore::new()),
    );

    let response = compact_json(
        &deps,
        &long_prose_history(),
        json!({ "id": "claude-x", "provider": "anthropic" }),
        json!({}),
    )
    .await;

    assert_eq!(response["status"], "ok", "{response}");
    assert!(
        summarizer.calls.lock().unwrap().len() > 1,
        "the head is consumed in fragments"
    );
}

/// Inline limits are trusted as given. The console's `/compact` sends the
/// catalog window with a made-up `max_output_tokens: 4096`, which inflates
/// the input budget by ~60k tokens: the first fragment plus the provider's
/// real output allowance overruns the window and compaction reports
/// `overflow` for a history the router-resolved path compacts.
#[tokio::test]
async fn inline_limits_that_understate_max_output_overrun_the_provider_window() {
    let summarizer = Arc::new(ProviderWindowSummarizer {
        calls: Mutex::new(Vec::new()),
    });

    let response = compact_json(
        &deps(summarizer.clone()),
        &long_prose_history(),
        json!({ "id": "claude-x", "provider": "anthropic",
                "limits": { "context_window": PROVIDER_WINDOW, "max_output_tokens": 4_096 } }),
        json!({}),
    )
    .await;

    assert_eq!(response["status"], "overflow", "{response}");
    let first = summarizer.calls.lock().unwrap()[0];
    assert!(
        first + PROVIDER_MAX_OUTPUT > PROVIDER_WINDOW,
        "first fragment: {first} tokens"
    );
}

// ---------------------------------------------------------------------------
// The session lease
// ---------------------------------------------------------------------------

/// The lease is a file under `lease_dir` keyed by the session id and freed
/// only by its holder or by the 300s TTL. A worker that died mid-summary
/// (crash, restart, OOM) used to leave its claim behind, and every
/// `/compact` on that session answered `busy` — "another compaction is in
/// progress" — for up to five minutes although nothing was running.
#[tokio::test]
async fn a_claim_left_by_a_dead_worker_does_not_block_compaction_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    {
        let crashed = FsLeaseStore::new(dir.path()).unwrap();
        let claim = LeaseRecord {
            nonce: "4242-crashed".into(),
            ts: fakes::CLOCK_START_MS,
        };
        crashed
            .set(LEASE_SCOPE, "session-1", Some(claim))
            .await
            .unwrap();
    }
    let restarted: Arc<dyn LeaseStore> = Arc::new(FsLeaseStore::new(dir.path()).unwrap());
    let deps = Deps {
        config: Arc::new(RwLock::new(Arc::new(WorkerConfig::default()))),
        resolver: Arc::new(FakeModelResolver::new()),
        summarizer: Arc::new(FakeSummarizer::new()),
        leases: lease_cell(restarted),
        clock: Arc::new(FakeClock::new()),
    };
    let messages = vec![
        user("q1"),
        assistant(&"a".repeat(40_000)),
        user("q2"),
        assistant("a2"),
        user("q3"),
        assistant("a3"),
    ];
    let options = json!({ "lease_key": "session-1" });

    let response = compact_json(&deps, &messages, inline_model(), options).await;
    assert_eq!(response["status"], "ok", "{response}");
}
