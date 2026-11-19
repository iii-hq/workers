//! Summariser fragments sized against the provider's own token count.
//!
//! Live incident (2026-11-19, console session "Compose"): explicit
//! compaction sent a 1M-token Claude window a fragment the provider
//! counted at 1.45M tokens. The heuristic estimate undercounted Claude's
//! tokenizer ~1.66x, fragments were cut in estimate units only, and every
//! attempt ended in `overflow`. These tests drive the real
//! `context::compact` handler over deterministic fakes.

#[path = "common/fakes.rs"]
#[allow(dead_code)]
mod fakes;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use context_manager::config::WorkerConfig;
use context_manager::core::estimate::{Estimator, HeuristicEstimator};
use context_manager::functions::compact;
use context_manager::ports::{Deps, SummarizeError, SummarizeRequest, Summarizer, lease_cell};
use context_manager::types::{AgentMessage, ContentBlock, Model};
use fakes::{FakeClock, FakeModelResolver, FakeSummarizer, InMemoryLeaseStore, SummarizerScript};
use serde_json::{Value, json};
use tokio::sync::RwLock;

/// Router-resolved limits: usable input 200_000 - 8_000 - min(20_000, 10%) = 172_000.
const WINDOW: u64 = 200_000;
const MAX_OUTPUT: u64 = 8_000;
const USABLE: u64 = 172_000;
/// How much more than the heuristic estimate the provider's tokenizer bills.
const BILLED_PER_ESTIMATE: f64 = 1.7;

/// A provider whose tokenizer bills `BILLED_PER_ESTIMATE` times the
/// heuristic estimate (as `FakeModelResolver::billing` counts it) and that
/// rejects a request it bills past the usable input, as the router does.
struct BillingProvider {
    /// Billed input of every request, and whether it was accepted.
    calls: Mutex<Vec<(u64, bool)>>,
}

impl BillingProvider {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<(u64, bool)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl Summarizer for BillingProvider {
    async fn summarize(&self, req: SummarizeRequest) -> Result<String, SummarizeError> {
        let message = AgentMessage::User {
            content: vec![ContentBlock::Text {
                text: req.user_prompt.clone(),
            }],
            timestamp: i64::MAX,
        };
        let estimate =
            HeuristicEstimator.text(&req.system_prompt) + HeuristicEstimator.message(&message);
        let billed = (estimate as f64 * BILLED_PER_ESTIMATE) as u64;
        let fits = billed <= USABLE;
        let mut calls = self.calls.lock().unwrap();
        calls.push((billed, fits));
        if !fits {
            return Err(SummarizeError::ContextOverflow(format!(
                "prompt is too long: {billed} tokens > {USABLE} maximum"
            )));
        }
        Ok(format!("## Goal\n- anchored summary {}", calls.len()))
    }
}

fn provider_model() -> Model {
    serde_json::from_value(json!({
        "id": "claude-x", "provider": "anthropic",
        "context_window": WINDOW, "max_output_tokens": MAX_OUTPUT
    }))
    .unwrap()
}

fn deps(summarizer: Arc<dyn Summarizer>, resolver: Arc<FakeModelResolver>) -> Deps {
    resolver.insert(provider_model());
    Deps {
        config: Arc::new(RwLock::new(Arc::new(WorkerConfig::default()))),
        resolver,
        summarizer,
        leases: lease_cell(Arc::new(InMemoryLeaseStore::new())),
        clock: Arc::new(FakeClock::new()),
    }
}

/// Long prose turns, about 240k estimated tokens: more than one fragment
/// by the estimate, more still by the provider's count.
fn long_prose_history() -> Vec<Value> {
    let mut messages = Vec::new();
    for i in 0..12 {
        messages.push(json!({
            "role": "user", "timestamp": 1,
            "content": [{ "type": "text", "text": format!("{i}: {}", "q".repeat(40_000)) }]
        }));
        messages.push(json!({
            "role": "assistant", "stop_reason": "end", "model": "m", "provider": "p",
            "timestamp": 2,
            "content": [{ "type": "text", "text": format!("{i}: {}", "a".repeat(40_000)) }]
        }));
    }
    messages
}

async fn compact_history(deps: &Deps) -> Value {
    let request = serde_json::from_value(json!({
        "messages": long_prose_history(),
        "model": { "id": "claude-x", "provider": "anthropic" },
        "options": {}
    }))
    .unwrap();
    serde_json::to_value(compact::handle(deps, request).await.unwrap()).unwrap()
}

/// The provider's count of each large fragment shrinks it before it is
/// sent, so the provider never rejects one.
#[tokio::test]
async fn provider_count_keeps_every_fragment_inside_the_window() {
    let provider = BillingProvider::new();
    let resolver = Arc::new(FakeModelResolver::billing(BILLED_PER_ESTIMATE));
    let deps = deps(provider.clone(), resolver.clone());

    let response = compact_history(&deps).await;

    assert_eq!(response["status"], "ok", "{response}");
    let calls = provider.calls();
    assert!(
        calls.iter().all(|(_, fits)| *fits),
        "no fragment is sent past the window: {calls:?}"
    );
    assert!(
        calls.len() > 2,
        "the provider's count cuts more fragments than the estimate: {calls:?}"
    );
    assert!(resolver.counts.load(Ordering::SeqCst) > 0);
}

/// Without a provider counter, an overflow reply is retried with a smaller
/// fragment instead of failing the whole compaction.
#[tokio::test]
async fn an_overflow_reply_retries_a_smaller_fragment() {
    let provider = BillingProvider::new();
    let deps = deps(provider.clone(), Arc::new(FakeModelResolver::new()));

    let response = compact_history(&deps).await;

    assert_eq!(response["status"], "ok", "{response}");
    let calls = provider.calls();
    assert!(
        calls.iter().any(|(_, fits)| !fits),
        "the estimate alone overruns the window: {calls:?}"
    );
    for pair in calls.windows(2) {
        if !pair[0].1 {
            assert!(
                pair[1].0 < pair[0].0,
                "a rejected fragment is retried smaller: {calls:?}"
            );
        }
    }
    assert!(calls.last().is_some_and(|(_, fits)| *fits));
}

/// Only an overflow is retried: any other summariser failure still ends
/// the compaction after one call.
#[tokio::test]
async fn other_summariser_failures_are_not_retried() {
    let summarizer = Arc::new(FakeSummarizer::new());
    summarizer.set_script(SummarizerScript::Fail("router/rate_limited".into()));
    let deps = deps(summarizer.clone(), Arc::new(FakeModelResolver::new()));

    let response = compact_history(&deps).await;

    assert_eq!(response["status"], "overflow", "{response}");
    assert_eq!(summarizer.calls().len(), 1);
}
