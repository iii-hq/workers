//! Clef-Flash end to end against Cloudflare's own pipeline (bf16 backbone and
//! head through `joint_schema_model.py`): `tests/fixtures/clef_reference.json`,
//! from `make_reference.py`.
use judge_clef::{download, encode, engine, ClefClient};
use judge_contract::{Answer, EvaluateRequest, EvaluateResponse, Question};
use serde_json::{json, Value};
use std::time::Instant;

fn argmax(p: &[f64]) -> usize {
    (0..p.len()).fold(0, |best, i| if p[i] > p[best] { i } else { best })
}

/// Every reference evaluation through `ClefClient::evaluate` on the pinned
/// GGUF (the fixture's `gguf`): same token count, every probability within the
/// fixture's `tolerance`, and the same top option wherever the reference's top
/// two are more than 2x tolerance apart (the misses, orders/region and
/// deploy_log/severity, are 0.005 and 0.017 near-ties).
///
/// `CLEF_CHECKPOINT_DIR` holds `model.gguf` and `tokenizer.json`. It runs on
/// the CPU (max |Δp| 0.065); `CLEF_GPU_LAYERS=all` runs it on the GPU (0.080
/// on Vulkan), a number offloads that many layers:
///
/// ```sh
/// CLEF_CHECKPOINT_DIR=<dir> cargo test --release --test clef -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore]
async fn gguf_matches_the_reference() {
    let dir = std::env::var("CLEF_CHECKPOINT_DIR").expect("CLEF_CHECKPOINT_DIR holds a checkpoint");
    let options = engine::Options {
        threads: 8,
        gpu_layers: std::env::var("CLEF_GPU_LAYERS").map_or(Some(0), |n| n.parse().ok()),
        context_tokens: 16384,
    };
    let checkpoint = download::local("clef-flash", dir.as_ref()).unwrap();
    let client = ClefClient::load(&checkpoint, options).unwrap();
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/clef_reference.json")).unwrap();
    let tolerance = fixture["tolerance"].as_f64().unwrap();
    let (mut worst, mut sum, mut count, mut flips) = (0f64, 0f64, 0usize, vec![]);
    for record in fixture["records"].as_array().unwrap() {
        let evaluation = &record["evaluation"];
        let id = evaluation["id"].as_str().unwrap();
        let request: EvaluateRequest =
            serde_json::from_value(json!({"timeout_ms": 300000, "evaluations": [evaluation]}))
                .unwrap();
        let started = Instant::now();
        let EvaluateResponse::Ok { results, stats, .. } = client.evaluate(request).await else {
            panic!("{id}: evaluation failed");
        };
        let seconds = started.elapsed().as_secs_f64();
        assert_eq!(
            stats.input_tokens,
            record["tokens"].as_u64().unwrap(),
            "{id}"
        );
        let (mut record_worst, mut record_sum, mut record_count) = (0f64, 0f64, 0usize);
        for (qid, reference) in record["probabilities"].as_object().unwrap() {
            let question: Question =
                serde_json::from_value(evaluation["questions"][qid].clone()).unwrap();
            let expected: Vec<f64> = serde_json::from_value(reference.clone()).unwrap();
            let got: Vec<f64> = match &results[id].answers[qid] {
                Answer::Noul { noul } => vec![*noul, 1.0 - noul],
                Answer::Choice { probabilities, .. } | Answer::Score { probabilities, .. } => {
                    let options = encode::options(&question).1;
                    options.iter().map(|(key, _)| probabilities[key]).collect()
                }
            };
            assert_eq!(got.len(), expected.len(), "{id}/{qid}");
            for (g, e) in got.iter().zip(&expected) {
                record_worst = record_worst.max((g - e).abs());
                record_sum += (g - e).abs();
            }
            record_count += got.len();
            let mut top = expected.clone();
            top.sort_by(|a, b| b.total_cmp(a));
            let margin = top[0] - top.get(1).unwrap_or(&0.0);
            if margin > 2.0 * tolerance && argmax(&got) != argmax(&expected) {
                flips.push(format!("{id}/{qid}"));
            }
            eprintln!("{id}/{qid}: {got:.4?} vs {expected:.4?}");
        }
        eprintln!(
            "{id}: {} tokens in {seconds:.1}s, max |Δp| {record_worst:.4}, mean {:.4}",
            stats.input_tokens,
            record_sum / record_count as f64
        );
        worst = worst.max(record_worst);
        (sum, count) = (sum + record_sum, count + record_count);
    }
    eprintln!("max |Δp| = {worst:.4}, mean {:.4}", sum / count as f64);
    assert!(worst <= tolerance, "max |Δp| {worst} > {tolerance}");
    assert!(flips.is_empty(), "top option changed: {flips:?}");
}
