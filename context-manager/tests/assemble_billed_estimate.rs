//! The harness-e2e context-capacity lane (usable window cut to 128k): a
//! single long turn reading 48 documents of random hex sent a 256k prompt
//! uncompacted. chars/4 counted about 2.25x fewer tokens than the provider
//! billed, so the estimate never crossed the window. Past a third of the
//! window the provider's count now sets the budget the relief steps use.

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

fn deps(resolver: Arc<FakeModelResolver>) -> Deps {
    Deps {
        config: Arc::new(RwLock::new(Arc::new(WorkerConfig::default()))),
        resolver,
        summarizer: Arc::new(FakeSummarizer::new()),
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
    let request = serde_json::from_value(json!({
        "messages": single_long_turn(documents),
        "model": { "id": "deepseek-flash", "provider": "deepseek",
                   "limits": { "context_window": 1_000_000, "max_output_tokens": 384_000 } },
        "system_prompt": "You are an iii agent worker.",
        "options": { "reserved_tokens": RESERVED, "request_overhead_tokens": request_overhead_tokens },
    }))
    .unwrap();
    serde_json::to_value(assemble::handle(&deps(resolver), request).await.unwrap()).unwrap()
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
async fn a_provider_counting_no_more_than_the_estimate_changes_nothing() {
    let resolver = Arc::new(FakeModelResolver::billing(0.9));
    let response = assemble(resolver.clone(), 48).await;

    assert_eq!(response["applied"]["compacted"], false);
    assert_eq!(resolver.counts.load(Ordering::SeqCst), 1);
}
