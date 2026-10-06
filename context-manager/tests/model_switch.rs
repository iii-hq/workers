//! MOT-4937: use the real handlers with a summarizer that enforces input capacity.
#[path = "common/fakes.rs"]
#[allow(dead_code)]
mod fakes;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use context_manager::config::WorkerConfig;
use context_manager::core::estimate::{Estimator, HeuristicEstimator};
use context_manager::core::summary::{render_user_prompt, strip_media};
use context_manager::functions::{assemble, compact};
use context_manager::ports::{lease_cell, Deps, SummarizeError, SummarizeRequest, Summarizer};
use context_manager::types::{AgentMessage, ContentBlock};
use serde_json::{json, Value};
use tokio::sync::RwLock;

struct CapacityLimitedSummarizer {
    budget: u64,
    fail_at: Option<usize>,
    calls: Mutex<Vec<SummarizeRequest>>,
    grow_anchor: bool,
}

#[async_trait]
impl Summarizer for CapacityLimitedSummarizer {
    async fn summarize(&self, req: SummarizeRequest) -> Result<String, SummarizeError> {
        let message = AgentMessage::User {
            content: vec![ContentBlock::Text {
                text: req.user_prompt.clone(),
            }],
            timestamp: i64::MAX,
        };
        let tokens =
            HeuristicEstimator.text(&req.system_prompt) + HeuristicEstimator.message(&message);
        let mut calls = self.calls.lock().unwrap();
        calls.push(req);
        if tokens > self.budget || self.fail_at == Some(calls.len()) {
            return Err(SummarizeError::Failed("router/context_overflow".into()));
        }
        let summary = if self.grow_anchor {
            format!("## Goal\n- {}", "x".repeat(calls.len() * 600))
        } else {
            format!("## Goal\n- anchored summary {}", calls.len())
        };
        Ok(summary)
    }
}

fn setup(
    budget: u64,
    fail_at: Option<usize>,
) -> (
    Deps,
    Arc<CapacityLimitedSummarizer>,
    Arc<fakes::InMemoryLeaseStore>,
) {
    setup_with_anchor_growth(budget, fail_at, false)
}

fn setup_with_anchor_growth(
    budget: u64,
    fail_at: Option<usize>,
    grow_anchor: bool,
) -> (
    Deps,
    Arc<CapacityLimitedSummarizer>,
    Arc<fakes::InMemoryLeaseStore>,
) {
    let summarizer = Arc::new(CapacityLimitedSummarizer {
        budget,
        fail_at,
        grow_anchor,
        calls: Mutex::new(Vec::new()),
    });
    let leases = Arc::new(fakes::InMemoryLeaseStore::new());
    let deps = Deps {
        config: Arc::new(RwLock::new(Arc::new(WorkerConfig::default()))),
        resolver: Arc::new(fakes::FakeModelResolver::new()),
        summarizer: summarizer.clone(),
        leases: lease_cell(leases.clone()),
        clock: Arc::new(fakes::FakeClock::new()),
    };
    (deps, summarizer, leases)
}

fn history() -> Vec<Value> {
    vec![
        json!({"role":"user","content":[{"type":"text","text":format!("START {} END", "東京🙂\\\"\n".repeat(4000))}],"timestamp":1}),
        json!({"role":"assistant","content":[{"type":"text","text":"Acknowledged."}],"model":"source","provider":"source-provider","stop_reason":"end","timestamp":2}),
        json!({"role":"user","content":[{"type":"text","text":"Continue."}],"timestamp":3}),
    ]
}

fn model() -> Value {
    json!({"id":"destination","provider":"destination-provider","limits":{"context_window":10000,"max_output_tokens":1000}})
}

#[tokio::test]
async fn switching_to_a_smaller_model_compacts_in_bounded_calls_and_keeps_the_tail() {
    let (deps, summarizer, leases) = setup(8000, None);
    let messages = history();
    let source = assemble::handle(&deps, serde_json::from_value(json!({
        "messages":messages,"model":{"id":"source","limits":{"context_window":200000,"max_output_tokens":1000}}
    })).unwrap()).await.unwrap();
    assert!(!source.applied.compacted);
    assert!(summarizer.calls.lock().unwrap().is_empty());

    let result = assemble::handle(
        &deps,
        serde_json::from_value(json!({
            "messages":messages,"model":model(),"options":{"tail_turns":1}
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert!(result.applied.compacted);
    assert!(result.token_count <= result.usable);
    assert_eq!(result.usable, 8000);
    assert_eq!(result.applied.tail_start_index, Some(Some(2)));
    assert_eq!(
        serde_json::to_value(&result.messages).unwrap(),
        json!([messages[2]])
    );
    let calls = summarizer.calls.lock().unwrap();
    assert!(
        calls.len() > 1,
        "oversized single user messages must also be split"
    );
    assert!(
        calls
            .iter()
            .all(|r| r.model == "destination"
                && r.provider.as_deref() == Some("destination-provider"))
    );
    assert!(calls[1].system_prompt.contains("anchored summary 1"));
    let head: Vec<AgentMessage> = messages[..2]
        .iter()
        .cloned()
        .map(|m| serde_json::from_value(m).unwrap())
        .collect();
    let rendered = render_user_prompt(&strip_media(
        &head,
        WorkerConfig::default().max_output_chars,
    ));
    assert_eq!(
        calls
            .iter()
            .map(|r| r.user_prompt.as_str())
            .collect::<String>(),
        rendered,
        "all rendered history bytes must be consumed exactly once, in order"
    );
    assert!(leases.keys().is_empty());
}

#[tokio::test]
async fn explicit_compaction_uses_the_same_bounded_path_and_preserves_steering() {
    let (deps, summarizer, leases) = setup(8000, None);
    let result = compact::handle(&deps, serde_json::from_value(json!({
        "messages":history(),"model":model(),
        "options":{"tail_turns":1,"previous_summary":"PRIOR ANCHOR","instructions":"Keep exact paths"}
    })).unwrap()).await.unwrap();
    assert!(matches!(
        result,
        compact::CompactResponse::Ok {
            tail_start_index: Some(2),
            used_prior_summary: true,
            ..
        }
    ));
    let calls = summarizer.calls.lock().unwrap();
    assert!(calls.len() > 1);
    assert!(calls[0].system_prompt.contains("PRIOR ANCHOR"));
    assert!(calls
        .iter()
        .all(|r| r.system_prompt.contains("Keep exact paths")));
    assert!(leases.keys().is_empty());
}

#[tokio::test]
async fn a_failed_later_chunk_returns_overflow_without_committing_a_partial_summary() {
    let (deps, summarizer, leases) = setup(8000, Some(2));
    let result = assemble::handle(
        &deps,
        serde_json::from_value(json!({
            "messages":history(),"model":model(),"options":{"tail_turns":1}
        }))
        .unwrap(),
    )
    .await;
    assert!(result.unwrap_err().to_string().contains("context/overflow"));
    assert_eq!(summarizer.calls.lock().unwrap().len(), 2);
    assert!(leases.keys().is_empty());
}

#[tokio::test]
async fn short_destination_history_does_not_invoke_the_summarizer() {
    let (deps, summarizer, _) = setup(8000, None);
    let result = assemble::handle(
        &deps,
        serde_json::from_value(json!({
            "messages":[history()[2]],"model":model()
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert!(!result.applied.compacted);
    assert!(summarizer.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_summary_that_cannot_fit_as_an_anchor_fails_without_dropping_the_anchor() {
    let (deps, summarizer, leases) = setup(8000, None);
    let result = compact::handle(
        &deps,
        serde_json::from_value(json!({
            "messages":history(),"model":model(),
            "options":{"tail_turns":1,"previous_summary":"x".repeat(40000)}
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert!(matches!(result, compact::CompactResponse::Overflow));
    assert!(summarizer.calls.lock().unwrap().is_empty());
    assert!(leases.keys().is_empty());
}

#[tokio::test]
async fn fitting_head_is_summarized_once_without_fragment_instructions() {
    let (deps, summarizer, _) = setup(8000, None);
    let result = compact::handle(
        &deps,
        serde_json::from_value(json!({
            "messages":[history()[2]],"model":model(),"options":{"tail_turns":0}
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert!(matches!(result, compact::CompactResponse::Ok { .. }));
    let calls = summarizer.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(!calls[0].system_prompt.contains("fragments"));
}

#[tokio::test]
async fn explicit_input_limit_and_reserve_bound_every_summary_call() {
    let (deps, summarizer, _) = setup(3000, None);
    let mut destination = model();
    destination["limits"]["input_limit"] = json!(4000);
    let result = assemble::handle(&deps, serde_json::from_value(json!({
        "messages":history(),"model":destination,"options":{"tail_turns":1,"reserved_tokens":1000}
    })).unwrap()).await.unwrap();
    assert_eq!(result.usable, 3000);
    assert!(result.applied.compacted);
    assert!(summarizer.calls.lock().unwrap().len() > 2);
}

#[tokio::test]
async fn excessive_chunk_counts_fail_closed_and_release_the_lease() {
    let (deps, summarizer, leases) = setup(1000, None);
    let mut messages = history();
    messages[0]["content"][0]["text"] = json!("large history ".repeat(50000));
    let result = compact::handle(&deps, serde_json::from_value(json!({
        "messages":messages,"model":{"id":"tiny","limits":{"context_window":2000,"max_output_tokens":800}},
        "options":{"tail_turns":1}
    })).unwrap()).await.unwrap();
    assert!(matches!(result, compact::CompactResponse::Overflow));
    assert_eq!(summarizer.calls.lock().unwrap().len(), 0);
    assert!(leases.keys().is_empty());
}

#[tokio::test]
async fn viable_history_near_the_preflight_boundary_is_not_rejected() {
    let (deps, summarizer, leases) = setup(2200, None);
    let mut messages = history();
    messages[0]["content"][0]["text"] = json!("near-boundary ".repeat(8_000));
    let result = compact::handle(
        &deps,
        serde_json::from_value(json!({
            "messages":messages,"model":{"id":"small","limits":{"context_window":3000,"max_output_tokens":500}},
            "options":{"tail_turns":1}
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert!(matches!(result, compact::CompactResponse::Ok { .. }));
    assert!(!summarizer.calls.lock().unwrap().is_empty());
    assert!(summarizer.calls.lock().unwrap().len() <= 32);
    assert!(leases.keys().is_empty());
}

#[tokio::test]
async fn a_long_initial_anchor_can_shrink_without_preflight_false_positive() {
    let (deps, summarizer, leases) = setup(8000, None);
    let result = compact::handle(
        &deps,
        serde_json::from_value(json!({
            "messages":history(),"model":model(),
            "options":{"tail_turns":1,"previous_summary":"prior anchor ".repeat(900)}
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    let compact::CompactResponse::Ok { summary, .. } = result else {
        panic!("long initial anchor should remain viable");
    };
    assert!(summary.len() < 100);
    assert!(summarizer.calls.lock().unwrap()[0]
        .system_prompt
        .contains("prior anchor"));
    assert!(leases.keys().is_empty());
}

#[tokio::test]
async fn anchor_growth_reaches_the_runtime_chunk_guard_without_partial_summary() {
    let (deps, summarizer, leases) = setup_with_anchor_growth(8000, None, true);
    let mut messages = history();
    messages[0]["content"][0]["text"] = json!("growing anchor ".repeat(45_000));
    let result = compact::handle(
        &deps,
        serde_json::from_value(json!({
            "messages":messages,"model":model(),"options":{"tail_turns":1}
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert!(matches!(result, compact::CompactResponse::Overflow));
    assert_eq!(summarizer.calls.lock().unwrap().len(), 32);
    assert!(leases.keys().is_empty());
}
struct PendingSummarizer;

#[async_trait]
impl Summarizer for PendingSummarizer {
    async fn summarize(&self, _: SummarizeRequest) -> Result<String, SummarizeError> {
        std::future::pending().await
    }
}

#[tokio::test]
async fn deadline_returns_overflow_and_releases_the_lease() {
    let (mut deps, _, leases) = setup(8000, None);
    deps.summarizer = Arc::new(PendingSummarizer);
    let config = WorkerConfig {
        summarizer_timeout_ms: 1,
        ..WorkerConfig::default()
    };
    *deps.config.write().await = Arc::new(config);
    let result = compact::handle(
        &deps,
        serde_json::from_value(json!({
            "messages":history(),"model":model(),"options":{"tail_turns":1}
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert!(matches!(result, compact::CompactResponse::Overflow));
    assert!(leases.keys().is_empty());
}

#[tokio::test]
async fn preview_reports_overflow_without_summarizing_or_acquiring_a_lease() {
    let (deps, summarizer, leases) = setup(8000, Some(1));
    let result = assemble::handle(
        &deps,
        serde_json::from_value(json!({
            "messages":history(), "model":model(),
            "options":{"preview_only":true,"allow_compaction":false,"tail_turns":1}
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert!(result.token_count > result.usable);
    assert_eq!(result.usable, 8000);
    assert!(!result.applied.compacted);
    assert!(summarizer.calls.lock().unwrap().is_empty());
    assert!(leases.keys().is_empty());
}
