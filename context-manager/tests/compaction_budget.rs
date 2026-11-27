//! Phase 3 (autocompact), D160-D162: `context::assemble`'s soft
//! `compaction_budget` with its echo and report, the gate's earlier provider
//! count, the inline compaction's instructions and `provider_options`, and the
//! summariser's failure cause on `context::compact`, `compaction.failure` and
//! the `context/overflow` error.
//!
//! Every test drives the real handlers over deterministic fakes — no engine,
//! no model. `router_stub.rs` runs the production router adapter against a
//! stub `router::chat`.

#[path = "common/fakes.rs"]
#[allow(dead_code)]
mod fakes;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use async_trait::async_trait;
use context_manager::config::WorkerConfig;
use context_manager::core::lease::LEASE_SCOPE;
use context_manager::error::{ContextError, OVERFLOW_FAILURE_MARKER};
use context_manager::functions::{assemble, compact};
use context_manager::ports::{
    Deps, LeaseRecord, SummarizeError, SummarizeRequest, Summarizer, lease_cell,
};
use context_manager::types::{ErrorDetails, Failure, FailureKind, ProviderOptions, UsageWindow};
use fakes::{
    CLOCK_START_MS, FakeClock, FakeModelResolver, FakeSummarizer, InMemoryLeaseStore,
    SummarizerScript,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::RwLock;

/// Inline limits with `reserved_tokens: 0`: usable = 100_000 - 10_000.
const USABLE: u64 = 90_000;
/// The soft threshold most tests send.
const BUDGET: u64 = 30_000;
/// About 2,500 estimated tokens per message.
const MESSAGE_CHARS: usize = 10_000;
const SUMMARY: &str = "## Goal\n- (scripted summary)";

struct Fixture {
    resolver: Arc<FakeModelResolver>,
    summarizer: Arc<FakeSummarizer>,
    leases: Arc<InMemoryLeaseStore>,
    config: WorkerConfig,
}

impl Fixture {
    fn new() -> Self {
        Self::with_resolver(FakeModelResolver::new())
    }

    fn with_resolver(resolver: FakeModelResolver) -> Self {
        let summarizer = FakeSummarizer::new();
        summarizer.set_script(SummarizerScript::Return(SUMMARY.into()));
        Self {
            resolver: Arc::new(resolver),
            summarizer: Arc::new(summarizer),
            leases: Arc::new(InMemoryLeaseStore::new()),
            config: WorkerConfig::default(),
        }
    }

    fn deps(&self) -> Deps {
        self.deps_with(self.summarizer.clone())
    }

    fn deps_with(&self, summarizer: Arc<dyn Summarizer>) -> Deps {
        Deps {
            config: Arc::new(RwLock::new(Arc::new(self.config.clone()))),
            resolver: self.resolver.clone(),
            summarizer,
            leases: lease_cell(self.leases.clone()),
            clock: Arc::new(FakeClock::new()),
        }
    }

    async fn assemble(&self, messages: Vec<Value>, options: Value) -> Result<Value, ContextError> {
        self.assemble_on(messages, options, 100_000, 10_000).await
    }

    async fn assemble_on(
        &self,
        messages: Vec<Value>,
        mut options: Value,
        context_window: u64,
        max_output_tokens: u64,
    ) -> Result<Value, ContextError> {
        options["reserved_tokens"] = json!(0);
        let request = serde_json::from_value(json!({
            "messages": messages,
            "model": { "id": "claude-x", "provider": "claude-code",
                       "limits": { "context_window": context_window,
                                   "max_output_tokens": max_output_tokens } },
            "system_prompt": "You are an iii agent.",
            "options": options,
        }))
        .unwrap();
        assemble::handle(&self.deps(), request)
            .await
            .map(|response| serde_json::to_value(response).unwrap())
    }
}

fn text(i: usize, role: &str) -> String {
    format!("{role} {i}: {}", "lorem ".repeat(MESSAGE_CHARS / 6))
}

/// `pairs` user/assistant turns of prose, about 5,000 tokens a turn.
fn prose(pairs: usize) -> Vec<Value> {
    let mut messages = Vec::new();
    for i in 0..pairs {
        messages.push(json!({
            "role": "user", "content": [{ "type": "text", "text": text(i, "user") }],
            "timestamp": 1
        }));
        messages.push(json!({
            "role": "assistant", "content": [{ "type": "text", "text": text(i, "assistant") }],
            "stop_reason": "end", "model": "claude-x", "provider": "claude-code", "timestamp": 2
        }));
    }
    messages
}

/// `prose(pairs)` with one ~12k-token function result in the middle: under
/// the result cap, inside prune's protected recent window, and the first
/// thing emergency reduction would replace.
fn prose_with_result(pairs: usize) -> Vec<Value> {
    let mut messages = prose(pairs);
    let at = messages.len() / 2;
    messages.insert(
        at,
        json!({
            "role": "assistant",
            "content": [{ "type": "function_call", "id": "c1", "function_id": "shell::run",
                          "arguments": {} }],
            "stop_reason": "function_call", "model": "claude-x", "provider": "claude-code",
            "timestamp": 3
        }),
    );
    messages.insert(
        at + 1,
        json!({
            "role": "function_result", "function_call_id": "c1", "function_id": "shell::run",
            "content": [{ "type": "text", "text": "x".repeat(48_000) }], "timestamp": 4
        }),
    );
    messages
}

fn usage_limit() -> Failure {
    Failure::new(
        FailureKind::UsageLimit,
        "Provider \"claude-code\" reached the usage limit of its account. Try again after the limit resets.",
    )
    .with_details(Some(ErrorDetails {
        usage_limit: true,
        window: Some(UsageWindow::FiveHour),
        resets_at_ms: Some(1_700_000_000_000),
    }))
}

fn account() -> ProviderOptions {
    ProviderOptions::from([("claude-code".to_string(), json!({ "account": "acct-work" }))])
}

// ---------------------------------------------------------------------------
// The soft budget (D160)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_budget_compacts_a_request_still_within_usable() {
    let fixture = Fixture::new();
    // About 40k tokens: past the 30k budget, well within the 90k usable.
    let out = fixture
        .assemble(prose(8), json!({ "compaction_budget": BUDGET }))
        .await
        .unwrap();

    let initial = out["applied"]["initial_token_count"].as_u64().unwrap();
    assert!(initial > BUDGET && initial < USABLE, "initial {initial}");
    assert_eq!(out["applied"]["compacted"], true);
    assert_eq!(
        out["compaction"],
        json!({ "trigger": "budget", "status": "compacted" })
    );
    assert_eq!(out["compaction_budget"], BUDGET);
    assert_eq!(out["usable"], USABLE);
    assert!(out["token_count"].as_u64().unwrap() < BUDGET, "{out}");
    assert_eq!(fixture.summarizer.calls().len(), 1);
}

#[tokio::test]
async fn without_a_budget_the_same_request_is_left_alone() {
    let fixture = Fixture::new();
    let out = fixture.assemble(prose(8), json!({})).await.unwrap();

    assert_eq!(out["applied"]["compacted"], false);
    assert!(out.get("compaction").is_none(), "{out}");
    // Present and null: the echo an older context-manager never sends.
    assert_eq!(out.get("compaction_budget"), Some(&Value::Null));
    assert!(fixture.summarizer.calls().is_empty());
}

#[tokio::test]
async fn the_echo_is_the_budget_it_applied() {
    let fixture = Fixture::new();
    // Under the threshold: no compaction tried, the echo still sent.
    let out = fixture
        .assemble(prose(2), json!({ "compaction_budget": BUDGET }))
        .await
        .unwrap();
    assert_eq!(out["compaction_budget"], BUDGET);
    assert!(out.get("compaction").is_none(), "{out}");

    // A budget past the model's limit applies the limit.
    let out = fixture
        .assemble(prose(2), json!({ "compaction_budget": 2_000_000 }))
        .await
        .unwrap();
    assert_eq!(out["compaction_budget"], USABLE);
}

#[tokio::test]
async fn past_usable_the_trigger_is_the_model_limit() {
    let fixture = Fixture::new();
    // About 100k tokens: past both the budget and usable.
    let out = fixture
        .assemble(prose(20), json!({ "compaction_budget": BUDGET }))
        .await
        .unwrap();
    assert_eq!(
        out["compaction"],
        json!({ "trigger": "usable", "status": "compacted" })
    );

    // With no budget, a compaction is always at the model's limit.
    let out = fixture.assemble(prose(20), json!({})).await.unwrap();
    assert_eq!(
        out["compaction"],
        json!({ "trigger": "usable", "status": "compacted" })
    );
    assert_eq!(out["compaction_budget"], Value::Null);
}

#[tokio::test]
async fn a_failed_summary_at_the_budget_drops_nothing_and_does_not_fail() {
    let fixture = Fixture::new();
    fixture
        .summarizer
        .set_script(SummarizerScript::FailWith(usage_limit()));
    let messages = prose_with_result(8);
    let count = messages.len();

    let out = fixture
        .assemble(messages, json!({ "compaction_budget": BUDGET }))
        .await
        .unwrap();

    assert_eq!(
        out["compaction"],
        json!({
            "trigger": "budget",
            "status": "failed",
            "failure": {
                "kind": "usage_limit",
                "message": usage_limit().message,
                "error_details": { "usage_limit": true, "window": "five_hour",
                                   "resets_at_ms": 1_700_000_000_000u64 }
            }
        })
    );
    // Emergency reduction holds to usable: the function result is intact.
    assert_eq!(out["applied"]["compacted"], false);
    assert_eq!(out["applied"]["pruned"], false);
    assert_eq!(out["messages"].as_array().unwrap().len(), count);
    assert_eq!(out["token_count"], out["applied"]["initial_token_count"]);
    assert!(out["token_count"].as_u64().unwrap() > BUDGET);
}

#[tokio::test]
async fn emergency_reduction_still_runs_past_usable() {
    let fixture = Fixture::new();
    fixture.summarizer.set_script(SummarizerScript::Unavailable);
    // About 92k tokens with a 12k function result: over usable, and the
    // result is what emergency reduction takes off.
    let out = fixture
        .assemble(
            prose_with_result(16),
            json!({ "compaction_budget": BUDGET }),
        )
        .await
        .unwrap();
    assert!(out["applied"]["initial_token_count"].as_u64().unwrap() > USABLE);
    assert_eq!(out["applied"]["pruned"], true);
    assert!(out["token_count"].as_u64().unwrap() <= USABLE);
    assert_eq!(out["compaction"]["trigger"], "usable");
    assert_eq!(out["compaction"]["status"], "failed");
    assert_eq!(out["compaction"]["failure"]["kind"], "unavailable");
}

#[tokio::test]
async fn an_overflow_after_a_failed_summary_carries_the_failure() {
    let fixture = Fixture::new();
    fixture
        .summarizer
        .set_script(SummarizerScript::FailWith(usage_limit()));

    // Prose only, past usable: nothing for emergency reduction to take.
    let error = fixture
        .assemble(prose(20), json!({ "compaction_budget": BUDGET }))
        .await
        .unwrap_err();
    let message = error.to_string();
    assert!(
        message.starts_with("context/overflow: assembled context requires "),
        "{message}"
    );
    let (head, json) = message.split_once(OVERFLOW_FAILURE_MARKER).unwrap();
    assert!(
        head.ends_with(&format!("but usable budget is {USABLE}")),
        "{head}"
    );
    assert_eq!(
        serde_json::from_str::<Failure>(json).unwrap(),
        usage_limit()
    );
}

#[tokio::test]
async fn an_overflow_with_no_failed_summary_reads_as_before() {
    let fixture = Fixture::new();
    let error = fixture
        .assemble(
            prose(20),
            json!({ "compaction_budget": BUDGET, "allow_compaction": false }),
        )
        .await
        .unwrap_err();
    let message = error.to_string();
    assert!(message.starts_with("context/overflow: "), "{message}");
    assert!(
        message.ends_with(&format!("but usable budget is {USABLE}")),
        "{message}"
    );
}

#[tokio::test]
async fn a_held_lease_reports_busy() {
    let fixture = Fixture::new();
    fixture.leases.put(
        LEASE_SCOPE,
        "session-1",
        LeaseRecord {
            nonce: "someone-else".into(),
            ts: CLOCK_START_MS,
        },
    );
    let out = fixture
        .assemble(
            prose(8),
            json!({ "compaction_budget": BUDGET, "lease_key": "session-1" }),
        )
        .await
        .unwrap();
    assert_eq!(
        out["compaction"],
        json!({ "trigger": "budget", "status": "busy" })
    );
    assert_eq!(out["applied"]["compacted"], false);
    assert!(fixture.summarizer.calls().is_empty());
}

#[tokio::test]
async fn a_head_that_would_take_everything_is_skipped() {
    let fixture = Fixture::new();
    let out = fixture
        .assemble(
            prose(8),
            json!({ "compaction_budget": BUDGET, "tail_turns": 0 }),
        )
        .await
        .unwrap();
    assert_eq!(
        out["compaction"],
        json!({ "trigger": "budget", "status": "skipped" })
    );
    assert!(fixture.summarizer.calls().is_empty());
}

#[tokio::test]
async fn a_preview_tries_no_compaction_but_echoes_the_budget() {
    let fixture = Fixture::new();
    let out = fixture
        .assemble(
            prose(8),
            json!({ "compaction_budget": BUDGET, "preview_only": true }),
        )
        .await
        .unwrap();
    assert!(out.get("compaction").is_none(), "{out}");
    assert_eq!(out["compaction_budget"], BUDGET);
    assert!(fixture.summarizer.calls().is_empty());
}

// ---------------------------------------------------------------------------
// The gate (D160)
// ---------------------------------------------------------------------------

/// A 1M window, usable 900k with no reserve: a third of it is 300k, so on
/// the estimate alone a 100k budget would decide this ~85k request is fine.
/// From 80 % of the budget the provider's count decides, and it bills 1.5x.
#[tokio::test]
async fn the_provider_counts_from_80_percent_of_the_budget() {
    let fixture = Fixture::with_resolver(FakeModelResolver::billing(1.5));
    let out = fixture
        .assemble_on(
            prose(17),
            json!({ "compaction_budget": 100_000 }),
            1_000_000,
            100_000,
        )
        .await
        .unwrap();
    let estimate = out["applied"]["initial_token_count"].as_u64().unwrap();
    assert!((80_000..100_000).contains(&estimate), "estimate {estimate}");
    assert!(fixture.resolver.counts.load(Ordering::SeqCst) >= 1);
    assert_eq!(
        out["compaction"],
        json!({ "trigger": "budget", "status": "compacted" })
    );
}

#[tokio::test]
async fn below_both_thresholds_the_estimate_alone_decides() {
    // ~75k: under 80 % of the 100k budget and under a third of usable.
    let fixture = Fixture::with_resolver(FakeModelResolver::billing(1.5));
    let out = fixture
        .assemble_on(
            prose(15),
            json!({ "compaction_budget": 100_000 }),
            1_000_000,
            100_000,
        )
        .await
        .unwrap();
    assert_eq!(fixture.resolver.counts.load(Ordering::SeqCst), 0);
    assert!(out.get("compaction").is_none(), "{out}");

    // Without the budget the ~85k request is never counted either.
    let fixture = Fixture::with_resolver(FakeModelResolver::billing(1.5));
    let out = fixture
        .assemble_on(prose(17), json!({}), 1_000_000, 100_000)
        .await
        .unwrap();
    assert_eq!(fixture.resolver.counts.load(Ordering::SeqCst), 0);
    assert!(out.get("compaction").is_none(), "{out}");
}

// ---------------------------------------------------------------------------
// Instructions and provider_options reach the inline compaction (D161)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_instructions_reach_the_inline_compaction() {
    let fixture = Fixture::new();
    fixture
        .assemble(
            prose(8),
            json!({ "compaction_budget": BUDGET,
                    "compaction_instructions": "  Keep the deploy steps.  " }),
        )
        .await
        .unwrap();
    let calls = fixture.summarizer.calls();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0]
            .system_prompt
            .contains("<instructions>\nKeep the deploy steps.\n</instructions>"),
        "{}",
        calls[0].system_prompt
    );
}

#[tokio::test]
async fn long_instructions_are_cut_at_2000_characters() {
    let fixture = Fixture::new();
    let long = format!("{}{}", "k".repeat(2_000), "DROPPED");
    fixture
        .assemble(
            prose(8),
            json!({ "compaction_budget": BUDGET, "compaction_instructions": long }),
        )
        .await
        .unwrap();
    let prompt = &fixture.summarizer.calls()[0].system_prompt;
    assert!(prompt.contains(&format!("{}... [truncated]", "k".repeat(2_000))));
    assert!(!prompt.contains("DROPPED"));
}

#[tokio::test]
async fn without_instructions_the_inline_prompt_has_none() {
    let fixture = Fixture::new();
    fixture
        .assemble(prose(8), json!({ "compaction_budget": BUDGET }))
        .await
        .unwrap();
    assert!(
        !fixture.summarizer.calls()[0]
            .system_prompt
            .contains("<instructions>")
    );
}

#[tokio::test]
async fn provider_options_ride_every_inline_summary_call() {
    let fixture = Fixture::new();
    fixture
        .assemble(
            prose(8),
            json!({ "compaction_budget": BUDGET, "provider_options": account() }),
        )
        .await
        .unwrap();
    let calls = fixture.summarizer.calls();
    assert!(!calls.is_empty());
    for call in calls {
        assert_eq!(call.provider_options, Some(account()));
    }

    let fixture = Fixture::new();
    fixture
        .assemble(prose(8), json!({ "compaction_budget": BUDGET }))
        .await
        .unwrap();
    assert_eq!(fixture.summarizer.calls()[0].provider_options, None);
}

// ---------------------------------------------------------------------------
// context::compact: provider_options and the failure cause (D161, D162)
// ---------------------------------------------------------------------------

async fn compact_with(deps: &Deps, messages: Vec<Value>, options: Value) -> Value {
    compact_on(deps, messages, options, 100_000, 10_000).await
}

async fn compact_on(
    deps: &Deps,
    messages: Vec<Value>,
    options: Value,
    context_window: u64,
    max_output_tokens: u64,
) -> Value {
    let request = serde_json::from_value(json!({
        "messages": messages,
        "model": { "id": "claude-x", "provider": "claude-code",
                   "limits": { "context_window": context_window,
                               "max_output_tokens": max_output_tokens } },
        "options": options,
    }))
    .unwrap();
    serde_json::to_value(compact::handle(deps, request).await.unwrap()).unwrap()
}

#[tokio::test]
async fn compact_forwards_provider_options() {
    let fixture = Fixture::new();
    let out = compact_with(
        &fixture.deps(),
        prose(4),
        json!({ "provider_options": account() }),
    )
    .await;
    assert_eq!(out["status"], "ok");
    let calls = fixture.summarizer.calls();
    assert!(!calls.is_empty());
    for call in calls {
        assert_eq!(call.provider_options, Some(account()));
    }
}

/// A summariser that never answers within the pipeline's time budget.
struct Slow;

#[async_trait]
impl Summarizer for Slow {
    async fn summarize(&self, _: SummarizeRequest) -> Result<String, SummarizeError> {
        tokio::time::sleep(Duration::from_secs(30)).await;
        Ok(SUMMARY.into())
    }
}

#[tokio::test]
async fn every_failure_kind_reaches_compacts_overflow() {
    let cases = [
        (SummarizerScript::FailWith(usage_limit()), "usage_limit"),
        (
            SummarizerScript::FailWith(Failure::new(FailureKind::RateLimited, "busy")),
            "rate_limited",
        ),
        (SummarizerScript::ContextOverflow, "input_too_large"),
        (SummarizerScript::Unavailable, "unavailable"),
        (SummarizerScript::Fail("boom".into()), "other"),
        (SummarizerScript::Empty, "other"),
    ];
    for (script, kind) in cases {
        let fixture = Fixture::new();
        fixture.summarizer.set_script(script.clone());
        let out = compact_with(&fixture.deps(), prose(4), json!({})).await;
        assert_eq!(out["status"], "overflow", "{script:?}");
        assert_eq!(out["failure"]["kind"], kind, "{script:?}: {out}");
        assert!(
            !out["failure"]["message"].as_str().unwrap().is_empty(),
            "{out}"
        );
    }

    // Only a usage limit carries the router's details.
    let fixture = Fixture::new();
    fixture
        .summarizer
        .set_script(SummarizerScript::FailWith(usage_limit()));
    let out = compact_with(&fixture.deps(), prose(4), json!({})).await;
    assert_eq!(
        out["failure"]["error_details"],
        json!({ "usage_limit": true, "window": "five_hour", "resets_at_ms": 1_700_000_000_000u64 })
    );
}

#[tokio::test]
async fn a_summary_past_its_time_budget_is_a_timeout() {
    let mut fixture = Fixture::new();
    fixture.config.summarizer_timeout_ms = 50;
    let out = compact_with(&fixture.deps_with(Arc::new(Slow)), prose(4), json!({})).await;
    assert_eq!(out["status"], "overflow");
    assert_eq!(out["failure"]["kind"], "timeout", "{out}");
}

#[tokio::test]
async fn a_history_too_large_to_split_is_input_too_large() {
    // usable 4_000 - 500 - 400 = 3_100: about 10 KB of history a fragment,
    // so ~1 MB needs far more than the 32 calls the pipeline allows.
    let fixture = Fixture::new();
    let out = compact_on(&fixture.deps(), prose(100), json!({}), 4_000, 500).await;
    assert_eq!(out["status"], "overflow");
    assert_eq!(out["failure"]["kind"], "input_too_large", "{out}");
    assert!(fixture.summarizer.calls().is_empty());
}

/// The harness's `CompactOutput` before 1.3.0: `overflow` as a unit variant.
#[derive(Debug, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[allow(dead_code)]
enum OlderCompactOutput {
    Ok { summary: String },
    Busy,
    Empty,
    Overflow,
}

#[tokio::test]
async fn an_older_reader_still_reads_overflow() {
    let fixture = Fixture::new();
    fixture
        .summarizer
        .set_script(SummarizerScript::FailWith(usage_limit()));
    let out = compact_with(&fixture.deps(), prose(4), json!({})).await;
    assert!(out.get("failure").is_some());
    assert!(matches!(
        serde_json::from_value::<OlderCompactOutput>(out).unwrap(),
        OlderCompactOutput::Overflow
    ));
}
