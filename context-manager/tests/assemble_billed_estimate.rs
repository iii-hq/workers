//! The harness-e2e context-capacity lane (usable window cut to 128k): a
//! single long turn reading 48 documents of random hex sent a 256k prompt
//! uncompacted. chars/4 counted about 2.25x fewer tokens than the provider
//! billed, so the estimate never crossed the window. Past a third of the
//! window the provider's count now sets the budget the relief steps use.
//! Once it compacted, the summary kept only the markers the summariser could
//! see in results cut to `max_output_chars` (2,000 chars); the agent had noted
//! them all in its thinking, which the summariser never got. It now gets the
//! thinking, and each result under the `max_result_tokens` cap the agent saw.

#[path = "common/fakes.rs"]
#[allow(dead_code)]
mod fakes;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use context_manager::config::WorkerConfig;
use context_manager::functions::assemble;
use context_manager::ports::{lease_cell, Deps};
use fakes::{FakeClock, FakeModelResolver, FakeSummarizer, InMemoryLeaseStore};
use serde_json::{json, Value};
use tokio::sync::RwLock;

/// deepseek-flash: (1,000,000 - 384,000) - 488,000 reserved = 128,000 usable.
const RESERVED: u64 = 488_000;

fn deps(resolver: Arc<FakeModelResolver>, summarizer: Arc<FakeSummarizer>) -> Deps {
    Deps {
        config: Arc::new(RwLock::new(Arc::new(WorkerConfig::default()))),
        resolver,
        summarizer,
        leases: lease_cell(Arc::new(InMemoryLeaseStore::new())),
        clock: Arc::new(FakeClock::new()),
    }
}

/// One prompt, then `documents` steps of one call answered by ~8 KiB.
fn single_long_turn(documents: usize) -> Vec<Value> {
    let mut messages = vec![json!({
        "role": "user", "content": [{ "type": "text", "text": "Read every document in order." }],
        "timestamp": 1
    })];
    for i in 0..documents {
        let id = format!("c{i}");
        messages.push(json!({
            "role": "assistant",
            "content": [{ "type": "function_call", "id": id, "function_id": "e2etest::segment",
                          "arguments": { "index": i } }],
            "stop_reason": "end", "model": "deepseek-flash", "provider": "deepseek", "timestamp": 2
        }));
        messages.push(json!({
            "role": "function_result", "function_call_id": id, "function_id": "e2etest::segment",
            "content": [{ "type": "text", "text": "a3f9 ".repeat(1_640) }], "timestamp": 3
        }));
    }
    messages
}

async fn assemble(resolver: Arc<FakeModelResolver>, documents: usize) -> Value {
    assemble_with_overhead(resolver, documents, 0).await
}

async fn assemble_with_overhead(
    resolver: Arc<FakeModelResolver>,
    documents: usize,
    request_overhead_tokens: u64,
) -> Value {
    let summarizer = Arc::new(FakeSummarizer::new());
    let messages = single_long_turn(documents);
    assemble_messages(resolver, summarizer, messages, request_overhead_tokens).await
}

async fn assemble_messages(
    resolver: Arc<FakeModelResolver>,
    summarizer: Arc<FakeSummarizer>,
    messages: Vec<Value>,
    request_overhead_tokens: u64,
) -> Value {
    let options =
        json!({ "reserved_tokens": RESERVED, "request_overhead_tokens": request_overhead_tokens });
    assemble_with_options(resolver, summarizer, messages, options).await
}

async fn assemble_with_options(
    resolver: Arc<FakeModelResolver>,
    summarizer: Arc<FakeSummarizer>,
    messages: Vec<Value>,
    options: Value,
) -> Value {
    let request = serde_json::from_value(json!({
        "messages": messages,
        "model": { "id": "deepseek-flash", "provider": "deepseek",
                   "limits": { "context_window": 1_000_000, "max_output_tokens": 384_000 } },
        "system_prompt": "You are an iii agent worker.",
        "options": options,
    }))
    .unwrap();
    serde_json::to_value(
        assemble::handle(&deps(resolver, summarizer), request)
            .await
            .unwrap(),
    )
    .unwrap()
}

fn marker(i: usize) -> String {
    format!("CTX-N{i}-{:012x}", 0xe624_f146_3731_u64 + i as u64)
}

/// `context_pressure` as the agent ran it: each step notes the previous
/// document's marker in its thinking, and the marker sits past the first
/// 2,000 characters (the prune threshold) of its ~8 KiB document.
fn noted_long_turn(documents: usize) -> Vec<Value> {
    let mut messages = vec![json!({
        "role": "user", "content": [{ "type": "text", "text": "Read every document in order." }],
        "timestamp": 1
    })];
    for i in 0..documents {
        let id = format!("c{i}");
        let mut content = Vec::new();
        if i > 0 {
            let note = format!("Segment {}: {}.", i - 1, marker(i - 1));
            content.push(json!({ "type": "thinking", "text": note }));
        }
        content.push(
            json!({ "type": "function_call", "id": id, "function_id": "e2etest::segment",
                             "arguments": { "index": i } }),
        );
        messages.push(json!({
            "role": "assistant", "content": content,
            "stop_reason": "end", "model": "deepseek-flash", "provider": "deepseek", "timestamp": 2
        }));
        let text = format!(
            "{}{} {}",
            "a3f9 ".repeat(1_000),
            marker(i),
            "a3f9 ".repeat(640)
        );
        messages.push(json!({
            "role": "function_result", "function_call_id": id, "function_id": "e2etest::segment",
            "content": [{ "type": "text", "text": text }], "timestamp": 3
        }));
    }
    messages
}

#[tokio::test]
async fn a_provider_billing_more_than_the_estimate_compacts_inside_the_window() {
    // 48 documents: about 100k by chars/4, about 225k billed.
    let resolver = Arc::new(FakeModelResolver::billing(2.25));
    let response = assemble(resolver.clone(), 48).await;

    assert_eq!(response["usable"], 128_000);
    assert_eq!(response["applied"]["compacted"], true);
    assert!(resolver.counts.load(Ordering::SeqCst) >= 1);
    // What goes out fits the window as the provider counts it.
    let sent = response["token_count"].as_u64().unwrap() as f64 * 2.25;
    assert!(sent <= 128_000.0, "sent {sent} provider tokens");
}

#[tokio::test]
async fn the_request_overhead_counts_with_the_provider_count() {
    // 22 documents: about 103k billed, inside the window on their own; a 40k
    // request overhead (response format, provider options) takes them past.
    const OVERHEAD: u64 = 40_000;
    let resolver = Arc::new(FakeModelResolver::billing(2.25));
    let response = assemble_with_overhead(resolver, 22, OVERHEAD).await;

    assert_eq!(response["applied"]["compacted"], true);
    let messages = response["token_count"].as_u64().unwrap() - OVERHEAD;
    let sent = messages as f64 * 2.25 + OVERHEAD as f64;
    assert!(sent <= 128_000.0, "sent {sent} provider tokens");
}

#[tokio::test]
async fn without_a_provider_count_the_estimate_still_decides() {
    let resolver = Arc::new(FakeModelResolver::new());
    let response = assemble(resolver.clone(), 48).await;

    assert_eq!(response["applied"]["compacted"], false);
    assert_eq!(resolver.counts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_small_context_is_not_counted_by_the_provider() {
    // 8 documents: about 17k by chars/4, under a third of 128k.
    let resolver = Arc::new(FakeModelResolver::billing(2.25));
    let response = assemble(resolver.clone(), 8).await;

    assert_eq!(response["applied"]["compacted"], false);
    assert_eq!(resolver.counts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn markers_noted_in_thinking_reach_the_summary() {
    let summarizer = Arc::new(FakeSummarizer::new());
    let response = assemble_messages(
        Arc::new(FakeModelResolver::billing(2.25)),
        summarizer.clone(),
        noted_long_turn(48),
        0,
    )
    .await;

    assert_eq!(response["applied"]["compacted"], true);
    let prompts: String = summarizer
        .calls()
        .iter()
        .map(|call| call.user_prompt.as_str())
        .collect();
    for i in 0..10 {
        assert!(
            prompts.contains(&format!("[thinking] Segment {i}: {}.", marker(i))),
            "the note for segment {i} never reached the summariser"
        );
    }
}

/// The same turn from a model whose reasoning comes back without text
/// (hidden or summarised): only the results carry the markers.
fn unnoted_long_turn(documents: usize) -> Vec<Value> {
    let mut messages = noted_long_turn(documents);
    for message in &mut messages {
        if let Some(content) = message["content"].as_array_mut() {
            content.retain(|block| block["type"] != "thinking");
        }
    }
    messages
}

#[tokio::test]
async fn markers_past_the_prune_threshold_reach_the_summary_without_notes() {
    let summarizer = Arc::new(FakeSummarizer::new());
    let response = assemble_messages(
        Arc::new(FakeModelResolver::billing(2.25)),
        summarizer.clone(),
        unnoted_long_turn(48),
        0,
    )
    .await;

    assert_eq!(response["applied"]["compacted"], true);
    let prompts: String = summarizer
        .calls()
        .iter()
        .map(|call| call.user_prompt.as_str())
        .collect();
    for i in 0..10 {
        assert!(
            prompts.contains(&marker(i)),
            "segment {i}'s marker never reached the summariser"
        );
    }
}

#[tokio::test]
async fn a_request_without_a_result_cap_reaches_the_summariser_whole() {
    // The first document runs to 100,000 chars, its marker past the
    // configured cap (20,000 tokens, 80,000 chars); the request disables it.
    let mut messages = unnoted_long_turn(48);
    let text = format!("{}{}", "a3f9 ".repeat(20_000), marker(0));
    messages[2]["content"] = json!([{ "type": "text", "text": text }]);
    let summarizer = Arc::new(FakeSummarizer::new());
    let response = assemble_with_options(
        Arc::new(FakeModelResolver::billing(2.25)),
        summarizer.clone(),
        messages,
        json!({ "reserved_tokens": RESERVED, "max_result_tokens": 0 }),
    )
    .await;

    assert_eq!(response["applied"]["compacted"], true);
    let prompts: String = summarizer
        .calls()
        .iter()
        .map(|call| call.user_prompt.as_str())
        .collect();
    assert!(
        prompts.contains(&marker(0)),
        "the summariser cut a result the agent saw whole"
    );
}

#[tokio::test]
async fn a_provider_counting_no_more_than_the_estimate_changes_nothing() {
    let resolver = Arc::new(FakeModelResolver::billing(0.9));
    let response = assemble(resolver.clone(), 48).await;

    assert_eq!(response["applied"]["compacted"], false);
    assert_eq!(resolver.counts.load(Ordering::SeqCst), 1);
}

/// A provider that bills prose at one token per estimate unit and the hex
/// documents at 2.25: the budget scales the whole request by its mean
/// density, and relief can keep the denser part.
mod mixed_density {
    use super::*;
    use async_trait::async_trait;
    use context_manager::core::estimate::{Estimator, HeuristicEstimator};
    use context_manager::ports::{ModelBudget, ModelResolver};
    use context_manager::types::{AgentFunction, AgentMessage, ContentBlock};
    use fakes::SummarizerScript;

    struct MixedDensity;

    fn billed(message: &AgentMessage) -> f64 {
        let dense = message
            .content()
            .iter()
            .any(|block| matches!(block, ContentBlock::Text { text } if text.contains("a3f9")));
        HeuristicEstimator.message(message) as f64 * if dense { 2.25 } else { 1.0 }
    }

    #[async_trait]
    impl ModelResolver for MixedDensity {
        async fn get_model_budget(
            &self,
            _provider: Option<&str>,
            _id: &str,
        ) -> Result<Option<ModelBudget>, String> {
            Ok(None)
        }

        async fn count_prompt_tokens(
            &self,
            _provider: Option<&str>,
            _id: &str,
            system_prompt: &str,
            tools: &[AgentFunction],
            messages: &[AgentMessage],
        ) -> Result<u64, String> {
            let estimator = HeuristicEstimator;
            let tools = tools
                .iter()
                .map(|tool| estimator.function(tool))
                .sum::<u64>();
            let messages = messages.iter().map(billed).sum::<f64>() as u64;
            Ok(messages + estimator.text(system_prompt) + tools)
        }
    }

    /// One user prompt, then one call per document, answered by it.
    fn read(prompt: String, documents: Vec<String>) -> Vec<Value> {
        let mut messages = vec![json!({
            "role": "user", "content": [{ "type": "text", "text": prompt }], "timestamp": 1
        })];
        for (i, text) in documents.into_iter().enumerate() {
            let id = format!("c{i}");
            messages.push(json!({
                "role": "assistant",
                "content": [{ "type": "function_call", "id": id, "function_id": "e2etest::segment",
                              "arguments": { "index": i } }],
                "stop_reason": "end", "model": "deepseek-flash", "provider": "deepseek", "timestamp": 2
            }));
            messages.push(json!({
                "role": "function_result", "function_call_id": id, "function_id": "e2etest::segment",
                "content": [{ "type": "text", "text": text }], "timestamp": 3
            }));
        }
        messages
    }

    /// With the summariser down only the emergency pass relieves.
    async fn assemble_without_summariser(messages: Vec<Value>) -> Result<Value, String> {
        let summarizer = Arc::new(FakeSummarizer::new());
        summarizer.set_script(SummarizerScript::Unavailable);
        let deps = Deps {
            config: Arc::new(RwLock::new(Arc::new(WorkerConfig::default()))),
            resolver: Arc::new(MixedDensity),
            summarizer,
            leases: lease_cell(Arc::new(InMemoryLeaseStore::new())),
            clock: Arc::new(FakeClock::new()),
        };
        let request = serde_json::from_value(json!({
            "messages": messages,
            "model": { "id": "deepseek-flash", "provider": "deepseek",
                       "limits": { "context_window": 1_000_000, "max_output_tokens": 384_000 } },
            "system_prompt": "You are an iii agent worker.",
            "options": { "reserved_tokens": RESERVED },
        }))
        .unwrap();
        assemble::handle(&deps, request)
            .await
            .map(|response| serde_json::to_value(response).unwrap())
            .map_err(|error| format!("{error:?}"))
    }

    fn sent_billed(response: &Value) -> f64 {
        let sent: Vec<AgentMessage> = serde_json::from_value(response["messages"].clone()).unwrap();
        sent.iter().map(billed).sum()
    }

    /// 30 prose documents, then 20 hex documents: the emergency pass removes
    /// the largest estimates first, the prose.
    #[tokio::test]
    async fn what_relief_keeps_is_counted_again_and_fits_the_window() {
        let documents = (0..50)
            .map(|i| {
                if i < 30 {
                    "plain words ".repeat(700)
                } else {
                    "a3f9 ".repeat(1_640)
                }
            })
            .collect();
        let messages = read("Read every document in order.".into(), documents);
        let response = assemble_without_summariser(messages).await.unwrap();

        assert_eq!(response["applied"]["compacted"], false);
        assert_eq!(response["applied"]["pruned"], true);
        let billed = sent_billed(&response);
        assert!(billed <= 128_000.0, "sent {billed} provider tokens");
    }

    /// A 120k prose prompt and five hex documents. The first count is over
    /// the window; the emergency pass reduces results, not prompts, so it
    /// cannot reach the tightened budget, but what it leaves fits as counted.
    #[tokio::test]
    async fn a_reduced_request_that_fits_as_counted_is_not_an_overflow() {
        let documents = (0..5).map(|_| "a3f9 ".repeat(1_640)).collect();
        let messages = read("plain words ".repeat(40_000), documents);
        let response = assemble_without_summariser(messages)
            .await
            .unwrap_or_else(|error| panic!("assemble failed: {error}"));

        assert_eq!(response["applied"]["pruned"], true);
        let billed = sent_billed(&response);
        assert!(billed <= 128_000.0, "sent {billed} provider tokens");
    }
}
