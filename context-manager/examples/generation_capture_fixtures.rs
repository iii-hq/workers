//! Regenerate the Harness consumer fixtures using the production Context APIs.
//! No engine, network, model, or ignored build artifact is used.
#[path = "../tests/common/fakes.rs"]
#[allow(dead_code)]
mod fakes;

use anyhow::{bail, ensure, Context, Result};
use context_manager::{
    config::WorkerConfig,
    core::summary::render_system_prompt_with_span,
    functions::assemble,
    ports::{lease_cell, Deps},
};
use fakes::{FakeClock, FakeModelResolver, FakeSummarizer, InMemoryLeaseStore, SummarizerScript};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::RwLock;

const MEMORY: &str = "<memory bank=\"synthetic-atlas\">\n- Atlas deploys with ember.\n</memory>";
const FACTS: &str = "synthetic-turn-fact\nother fact";

fn user(text: &str) -> Value {
    json!({"role":"user","content":[{"type":"text","text":text}],"timestamp":0})
}

async fn fixtures() -> Result<Value> {
    // Same production assembly seam as tests/origin_binding.rs; only the
    // summarizer and external ports are deterministic in-memory fakes.
    let summarizer = Arc::new(FakeSummarizer::new());
    summarizer.set_script(SummarizerScript::Return(FACTS.into()));
    let deps = Deps {
        config: Arc::new(RwLock::new(Arc::new(WorkerConfig::default()))),
        resolver: Arc::new(FakeModelResolver::new()),
        summarizer: summarizer.clone(),
        leases: lease_cell(Arc::new(InMemoryLeaseStore::new())),
        clock: Arc::new(FakeClock::new()),
    };
    let input = vec![
        user(FACTS),
        user(FACTS),
        user(&"padding ".repeat(1200)),
        json!({"role":"assistant","content":[{"type":"text","text":"done"}],"model":"synthetic","provider":"synthetic","stop_reason":"end","timestamp":0}),
        user("tail"),
    ];
    let request = serde_json::from_value(json!({
        "messages":input,"system_prompt":FACTS,
        "model":{"id":"synthetic","limits":{"context_window":2000,"max_output_tokens":128}},
        "options":{"reserved_tokens":0,"tail_turns":1,"allow_prune":false}
    }))?;
    let mut output = serde_json::to_value(assemble::handle(&deps, request).await?)?;
    ensure!(
        output["applied"]["compacted"] == true,
        "assembly did not compact"
    );
    ensure!(!summarizer.calls().is_empty(), "assembly did not summarize");
    let span = &output["summary_origin"]["span"];
    let start = span["byte_start"].as_u64().context("missing span start")? as usize;
    let end = span["byte_end"].as_u64().context("missing span end")? as usize;
    ensure!(output["system_prompt"].as_str().unwrap().get(start..end) == Some(FACTS));
    // Production creates a random correlation UUID, not content provenance.
    // Normalize ONLY this opaque ID so byte-for-byte regeneration is stable.
    output["summary_origin"]["assembly_id"] = json!("native-assembly-fixture");

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
                    "summarized_indices":[1],"span":span.context("renderer omitted span")?}
            }),
        );
    }
    Ok(json!({"assembly":{"input":input,"output":output},"renderers":renderers}))
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let default_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../harness/tests/fixtures/generation_capture");
    let (check, dir) = match args.next() {
        None => (false, default_dir),
        Some(arg) if arg == "--check" => (true, default_dir),
        Some(arg) if arg == "--output" => (
            false,
            PathBuf::from(args.next().context("--output needs a directory")?),
        ),
        _ => bail!("usage: generation_capture_fixtures [--check | --output DIR]"),
    };
    ensure!(args.next().is_none(), "unexpected argument");
    let mut bytes = serde_json::to_vec_pretty(&fixtures().await?)?;
    bytes.push(b'\n');
    let name = "native-context.json";
    if check {
        // Exercise actual filesystem generation in isolation, never replacing
        // the checked-in oracle while checking it.
        let temp = tempfile::tempdir()?;
        let generated = temp.path().join(name);
        std::fs::write(&generated, &bytes)?;
        ensure!(
            std::fs::read(&generated)? == std::fs::read(dir.join(name))?,
            "fixtures differ; regenerate with this example and review the producer change"
        );
    } else {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(name), &bytes)?;
    }
    println!(
        "{} {name}: {} bytes sha256={:x}; assembly=1 renderers=7",
        if check { "checked" } else { "generated" },
        bytes.len(),
        Sha256::digest(&bytes)
    );
    Ok(())
}
