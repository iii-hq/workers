//! Parity spike: load the laya checkpoint from ~/.cache/judge-laya/laya and score
//! three typed questions over one ticket. Prints logits, probabilities, confidence.
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
    let t0 = Instant::now();
    let model = LayaModel::load(&checkpoint, Device::Cpu)?;
    eprintln!("loaded in {:.1}s", t0.elapsed().as_secs_f32());
    let tok = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json"))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let enc = Encoder::new(tok, model.agent.max_len, model.agent.head_max_len)?;

    let state = json!({
        "from": "user@acme.com",
        "subject": "Duplicate charge on invoice #4411",
        "body": "Hi, we were billed twice for March. Please refund the duplicate today or we will cancel our plan."
    });
    let questions = [
        (
            QType::Choice,
            "Which department should handle this request?",
            json!({
            "billing": "invoices, payments, refunds", "technical": "bugs, outages, system errors",
            "sales": "pricing, new contracts", "other": "everything else"}),
        ),
        (
            QType::Score,
            "How urgent is this request?",
            json!(["not urgent", "soon", "critical deadline or blocking issue"]),
        ),
        (
            QType::Noul,
            "Does the user threaten to cancel or leave?",
            json!(null),
        ),
        (
            QType::Noul,
            "Does the user explicitly request a refund?",
            json!(null),
        ),
    ];
    let mut rows = Vec::new();
    let mut qs = Vec::new();
    for (qtype, ins, criteria) in &questions {
        let q = Question {
            qtype: *qtype,
            instructions: ins.to_string(),
            options: render_options(*qtype, Some(criteria))?,
        };
        let seq = enc.build(&state, &q)?;
        println!(
            "{:<6} tokens={} markers={:?}",
            qtype.name(),
            seq.ids.len(),
            seq.markers
        );
        rows.push((seq.ids, seq.markers, *qtype as u32));
        qs.push(q);
    }
    let t1 = Instant::now();
    let logits = model.logits(&rows)?;
    eprintln!(
        "forward ({} rows) in {:.2}s",
        rows.len(),
        t1.elapsed().as_secs_f32()
    );
    for (q, z) in qs.iter().zip(&logits) {
        let k = z.len();
        let t = model.temperature(q.qtype as u32, k);
        let m = z.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let e: Vec<f32> = z
            .iter()
            .map(|v| ((v - m) as f64 / t).exp() as f32)
            .collect();
        let sum: f32 = e.iter().sum();
        let p: Vec<f32> = e.iter().map(|v| v / sum).collect();
        let ent: f32 = -p.iter().map(|&x| x * x.max(1e-12).ln()).sum::<f32>();
        let conf = if k < 2 {
            1.0
        } else {
            (1.0 - ent / (k as f32).ln()).clamp(0.0, 1.0)
        };
        println!(
            "\n{} ({}) T={t:.3}\n  logits={:?}\n  probs={:?}\n  confidence={conf:.4}",
            q.qtype.name(),
            q.instructions,
            z,
            p
        );
        for (o, pi) in q.options.iter().zip(&p) {
            println!("    {pi:.3}  {o}");
        }
    }
    Ok(())
}
