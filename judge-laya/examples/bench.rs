//! CPU throughput spike: forward time per batch size on the real checkpoint.
//! LAYA_DIR points at ~/.cache/judge-laya/laya (flat layout from the parity spike).
use anyhow::Result;
use candle_core::Device;
use judge_laya::encode::{render_options, Encoder, QType, Question};
use judge_laya::model::LayaModel;
use serde_json::json;
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<()> {
    let dir =
        PathBuf::from(std::env::var("LAYA_DIR").unwrap_or_else(|_| {
            format!("{}/.cache/judge-laya/laya", std::env::var("HOME").unwrap())
        }));
    let checkpoint = judge_laya::download::Checkpoint {
        model: "laya".into(),
        revision: "local".into(),
        weights: dir.join("model.safetensors"),
        encoder_config: dir.join("encoder-config.json"),
        agent_config: dir.join("rl_agent_config.json"),
        tokenizer: dir.join("tokenizer.json"),
    };
    let model = LayaModel::load(&checkpoint, Device::Cpu)?;
    let tok = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json"))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let enc = Encoder::new(tok, model.agent.max_len, model.agent.head_max_len)?;
    let state = json!({"subject": "Duplicate charge on invoice #4411", "body": "Hi, we were billed twice for March. Please refund the duplicate today or we will cancel our plan. ".repeat(4)});
    let q = Question {
        qtype: QType::Choice,
        instructions: "Which department should handle this request?".into(),
        options: render_options(
            QType::Choice,
            Some(
                &json!({"billing": "invoices, payments, refunds", "technical": "bugs, outages", "sales": "pricing", "other": null}),
            ),
        )?,
    };
    let seq = enc.build(&state, &q)?;
    println!(
        "tokens per row: {} (threads: {})",
        seq.ids.len(),
        std::env::var("RAYON_NUM_THREADS").unwrap_or_else(|_| "all".into())
    );
    let row = (seq.ids, seq.markers, QType::Choice as u32);
    // warmup
    model.logits(std::slice::from_ref(&row))?;
    for batch in [1usize, 4, 16, 32] {
        let rows: Vec<_> = (0..batch).map(|_| row.clone()).collect();
        let runs = if batch >= 16 { 2 } else { 4 };
        let t = Instant::now();
        for _ in 0..runs {
            model.logits(&rows)?;
        }
        let per_batch = t.elapsed().as_secs_f32() * 1000.0 / runs as f32;
        println!(
            "batch {batch:>2}: {per_batch:7.0} ms/forward  {:6.0} ms/question",
            per_batch / batch as f32
        );
    }
    Ok(())
}
