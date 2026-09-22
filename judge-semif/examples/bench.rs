//! Throughput on the real GGUF: `SEMIF_GGUF=… PARALLEL=8 cargo run --release
//! --example bench -- <request.json>...` (each file holds an
//! EvaluateRequest, or `{"request": EvaluateRequest}` as the probe saves it).
use judge_contract::{EvaluateRequest, EvaluateResponse};
use judge_semif::{download, engine, SemifClient};
use std::time::Instant;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let env = |k: &str, d: usize| {
        std::env::var(k)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(d)
    };
    let checkpoint = download::local("qwen3.5-4b", std::env::var("SEMIF_GGUF")?.as_ref())?;
    let client = SemifClient::load(
        &checkpoint,
        engine::Options {
            threads: env("THREADS", 8),
            gpu_layers: std::env::var("GPU_LAYERS")
                .ok()
                .and_then(|v| v.parse().ok()),
            context_tokens: env("CONTEXT", 16384) as u32,
            parallel: env("PARALLEL", 8),
        },
    )?;
    for path in std::env::args().skip(1) {
        let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
        let mut request: EvaluateRequest =
            serde_json::from_value(value.get("request").cloned().unwrap_or(value))?;
        request.timeout_ms = 300_000;
        request.expires_at_unix_ms = None;
        client.evaluate(request.clone()).await; // warm-up
        let started = Instant::now();
        let reps = env("REPS", 3);
        let mut last = None;
        for _ in 0..reps {
            last = Some(client.evaluate(request.clone()).await);
        }
        let ms = started.elapsed().as_secs_f64() * 1000.0 / reps as f64;
        match last.unwrap() {
            EvaluateResponse::Ok { stats, .. } => println!(
                "{path}: {} questions, {} tokens, {ms:.0} ms/request, {:.1} ms/question on {}",
                stats.questions,
                stats.input_tokens,
                ms / stats.questions as f64,
                client.device()
            ),
            other => println!("{path}: {other:?}"),
        }
    }
    Ok(())
}
