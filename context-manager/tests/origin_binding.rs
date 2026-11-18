//! Pure production-handler provenance tests (`options.provenance`); no
//! engine or model.
#[path = "common/fakes.rs"]
#[allow(dead_code)]
mod fakes;
mod support;
use context_manager::{
    config::WorkerConfig,
    core::summary::render_system_prompt_with_span,
    functions::assemble,
    ports::{Deps, lease_cell},
};
use fakes::{FakeClock, FakeModelResolver, FakeSummarizer, InMemoryLeaseStore, SummarizerScript};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::RwLock;

fn deps(summary: Arc<FakeSummarizer>) -> Deps {
    Deps {
        config: Arc::new(RwLock::new(Arc::new(WorkerConfig::default()))),
        resolver: Arc::new(FakeModelResolver::new()),
        summarizer: summary,
        leases: lease_cell(Arc::new(InMemoryLeaseStore::new())),
        clock: Arc::new(FakeClock::new()),
    }
}
fn user(text: &str) -> Value {
    json!({"role":"user","content":[{"type":"text","text":text}],"timestamp":0})
}
async fn run(messages: Vec<Value>, options: Value, window: u64) -> (Value, Arc<FakeSummarizer>) {
    let summarizer = Arc::new(FakeSummarizer::new());
    summarizer.set_script(SummarizerScript::Return(
        "synthetic-turn-fact\nother fact".into(),
    ));
    let req = serde_json::from_value(json!({"messages":messages,"system_prompt":"synthetic-turn-fact\nother fact",
        "model":{"id":"synthetic","limits":{"context_window":window,"max_output_tokens":128}},"options":options})).unwrap();
    (
        serde_json::to_value(
            assemble::handle(&deps(summarizer.clone()), req)
                .await
                .unwrap(),
        )
        .unwrap(),
        summarizer,
    )
}
#[tokio::test]
async fn origin_map_duplicates_custom_filter_and_limits() {
    let custom = json!({"role":"custom","custom_type":"note","content":[],"data":{},"timestamp":0});
    let messages = vec![user("same"), custom, user("same")];
    let (out, _) = run(
        messages.clone(),
        json!({"preview_only":true,"provenance":true}),
        100_000,
    )
    .await;
    assert_eq!(out["input_indices"], json!([0, 2]));
    assert_eq!(out["messages"][0], out["messages"][1]);
    let (out, _) = run(
        vec![user("x"); 2049],
        json!({"preview_only":true,"provenance":true}),
        100_000,
    )
    .await;
    assert!(out.get("input_indices").is_none());
    // Off by default: an ordinary assembly carries no provenance.
    let (out, _) = run(messages, json!({"preview_only":true}), 100_000).await;
    assert!(out.get("input_indices").is_none(), "{out}");
}
#[tokio::test]
async fn origin_map_survives_real_result_cap_without_retyping_neighbors() {
    let result = json!({"role":"function_result","function_call_id":"c","function_id":"read",
        "content":[{"type":"text","text":"x".repeat(10000)}],"timestamp":0});
    let (out, _) = run(
        vec![user("same"), result.clone(), user("same")],
        json!({"preview_only":true,"max_result_tokens":32,"provenance":true}),
        100_000,
    )
    .await;
    assert_eq!(out["input_indices"], json!([0, 1, 2]));
    assert_ne!(out["messages"][1], result);
    assert_eq!(out["messages"][0], out["messages"][2]);
}
#[tokio::test]
async fn origin_map_real_age_pruning_keeps_exact_input_ordinals() {
    let summarizer = Arc::new(FakeSummarizer::new());
    let deps = deps(summarizer);
    *deps.config.write().await = Arc::new(WorkerConfig {
        protect_recent_tokens: 0,
        protected_user_turns: 0,
        decay_user_turns: 1,
        min_free_tokens: 0,
        max_output_chars: 32,
        max_result_tokens: 0,
        ..Default::default()
    });
    let messages = vec![
        user("duplicate"),
        json!({"role":"function_result","function_call_id":"c","function_id":"read",
        "content":[{"type":"text","text":"r".repeat(12000)}],"timestamp":0}),
        user("duplicate"),
    ];
    let req = serde_json::from_value(json!({"messages":messages,"model":{"id":"synthetic","limits":{"context_window":2000,"max_output_tokens":128}},
        "options":{"reserved_tokens":0,"allow_compaction":false,"max_result_tokens":0,"provenance":true}})).unwrap();
    let out = serde_json::to_value(assemble::handle(&deps, req).await.unwrap()).unwrap();
    assert_eq!(out["applied"]["pruned"], true);
    assert_eq!(out["input_indices"], json!([0, 1, 2]));
    assert_eq!(out["messages"][0], out["messages"][2]);
    assert_ne!(out["messages"][1], messages[1]);
}

const MEMORY: &str = "<memory bank=\"synthetic-atlas\">\n- Atlas deploys with ember.\n</memory>";
const FACTS: &str = "synthetic-turn-fact\nother fact";

/// The real compaction assembly is also the Harness generation-capture
/// fixture: drift fails here, and `UPDATE_GOLDENS=1` rewrites
/// `harness/tests/fixtures/generation_capture/native-context.json`.
#[tokio::test]
async fn origin_real_compaction_renderer_and_harness_fixture() {
    let messages = vec![
        user(FACTS),
        user(FACTS),
        user(&"padding ".repeat(1200)),
        json!({"role":"assistant","content":[{"type":"text","text":"done"}],"model":"synthetic","provider":"synthetic","stop_reason":"end","timestamp":0}),
        user("tail"),
    ];
    let options = json!({"reserved_tokens":0,"tail_turns":1,"allow_prune":false});
    let (plain, _) = run(messages.clone(), options.clone(), 2000).await;
    assert_eq!(plain["applied"]["compacted"], true, "{plain}");
    assert!(plain.get("summary_origin").is_none() && plain.get("input_indices").is_none());
    let mut options = options;
    options["provenance"] = json!(true);
    let (mut out, summarizer) = run(messages.clone(), options, 2000).await;
    assert_eq!(out["applied"]["compacted"], true, "{out}");
    assert_eq!(out["system_prompt"], plain["system_prompt"]);
    let indices = out["input_indices"].as_array().unwrap();
    let head = out["summary_origin"]["summarized_indices"]
        .as_array()
        .unwrap();
    assert!(head.contains(&json!(0)) && head.contains(&json!(1)));
    assert_eq!(head.len() + indices.len(), messages.len());
    for (i, index) in indices.iter().enumerate() {
        assert_eq!(
            out["messages"][i],
            messages[index.as_u64().unwrap() as usize]
        );
    }
    assert!(
        summarizer.calls().iter().any(|call| call
            .user_prompt
            .matches("synthetic-turn-fact")
            .count()
            >= 2)
    );
    let span = &out["summary_origin"]["span"];
    let start = span["byte_start"].as_u64().unwrap() as usize;
    let end = span["byte_end"].as_u64().unwrap() as usize;
    let system = out["system_prompt"].as_str().unwrap();
    assert_eq!(&system[start..end], FACTS);
    assert!(start > FACTS.len());

    // The random correlation id is the only normalized output field.
    out["summary_origin"]["assembly_id"] = json!("native-assembly-fixture");
    let mut renderers = serde_json::Map::new();
    for (name, base, summary) in [
        ("memory-ordered", "policy", MEMORY),
        (
            "memory-reordered",
            "policy",
            "</memory>\n- Atlas deploys with ember.\n<memory bank=\"synthetic-atlas\">",
        ),
        (
            "memory-noncontiguous",
            "policy",
            "<memory bank=\"synthetic-atlas\">\ninterloper\n- Atlas deploys with ember.\n</memory>",
        ),
        ("memory-basecopy", MEMORY, "unrelated summary"),
        (
            "summary-reordered",
            FACTS,
            "other fact\nsynthetic-turn-fact",
        ),
        (
            "summary-noncontiguous",
            FACTS,
            "synthetic-turn-fact\ninterloper\nother fact",
        ),
        ("summary-basecopy", FACTS, "unrelated"),
    ] {
        let (system, span) = render_system_prompt_with_span(Some(base), Some(summary));
        renderers.insert(
            name.into(),
            json!({
                "system_prompt":system,
                "summary_origin":{"assembly_id":format!("native-render-{name}"),
                    "summarized_indices":[1],"span":span.unwrap()}
            }),
        );
    }
    let fixture = json!({"assembly":{"input":messages,"output":out},"renderers":renderers});
    let mut text = serde_json::to_string_pretty(&fixture).unwrap();
    text.push('\n');
    support::check_golden(
        "../../../harness/tests/fixtures/generation_capture/native-context.json",
        &text,
    )
    .unwrap();
}
