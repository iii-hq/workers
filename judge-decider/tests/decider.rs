//! decider-4b v2's layout against decider's own systemone rows and
//! probabilities (`tests/fixtures/decider_prompts.json`, from
//! `make_decider_prompts.py`).
use judge_contract::{Answer, EvaluateRequest, EvaluateResponse, Question};
use judge_decider::{client::prompts, prompt::render_state};
use serde_json::{json, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/decider_prompts.json")).unwrap()
}

/// Every case's prompts, in decider's row order.
fn rows(case: &Value) -> Vec<(String, Vec<String>)> {
    case["questions"]
        .as_object()
        .unwrap()
        .values()
        .flat_map(|question| {
            let question: Question = serde_json::from_value(question.clone()).unwrap();
            prompts(&question).unwrap()
        })
        .collect()
}

/// The decider layout end to end on the tiny GGUF (random weights, byte
/// vocabulary: 26 one-token labels).
#[tokio::test]
async fn tiny_decider_answers_every_type() {
    use judge_contract::ErrorCode;
    use judge_decider::{download, engine, DeciderClient};
    let gguf =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny-qwen3.gguf");
    let client = DeciderClient::load(
        &download::local("decider-4b-v2", &gguf).unwrap(),
        engine::Options {
            threads: 2,
            gpu_layers: Some(0),
            context_tokens: 4096,
            parallel: 4,
        },
    )
    .unwrap();
    let evaluate = |questions: Value| {
        let request: EvaluateRequest = serde_json::from_value(json!({
            "timeout_ms": 30000,
            "evaluations": [{"id": "e", "state": {"items": (0..9).collect::<Vec<_>>()}, "questions": questions}],
        }))
        .unwrap();
        client.evaluate(request)
    };
    let areas: serde_json::Map<String, Value> = (0..12)
        .map(|i| (format!("area{i:02}"), json!(null)))
        .collect();
    let EvaluateResponse::Ok { results, stats, .. } = evaluate(json!({
        "urgent": {"type": "noul", "instructions": "Is this urgent?"},
        "area": {"type": "choice", "criteria": areas},
        "severity": {"type": "score", "criteria": ["routine", "degraded", {"impact": "blocking"}]},
    }))
    .await
    else {
        panic!("evaluation failed");
    };
    assert_eq!(stats.questions, 3);
    let answers = &results["e"].answers;
    let Answer::Score { probabilities, .. } = &answers["severity"] else {
        panic!("score answer");
    };
    assert_eq!(probabilities.len(), 3);
    assert!((probabilities.values().sum::<f64>() - 1.0).abs() < 1e-9);
    let Answer::Choice { probabilities, .. } = &answers["area"] else {
        panic!("choice answer");
    };
    assert_eq!(probabilities.len(), 12);
    // Past the vocabulary's one-token labels.
    let wide: serde_json::Map<String, Value> =
        (0..27).map(|i| (format!("o{i:02}"), json!(null))).collect();
    let EvaluateResponse::Error { code, .. } =
        evaluate(json!({"q": {"type": "choice", "criteria": wide}})).await
    else {
        panic!("27 options exceed the tiny vocabulary's labels");
    };
    assert_eq!(code, ErrorCode::PayloadTooLarge);
}

#[test]
fn prompts_match_decider() {
    for case in fixture()["cases"].as_array().unwrap() {
        assert_eq!(
            format!("Context:\n{}", render_state(&case["state"])),
            case["context"].as_str().unwrap()
        );
        let expected: Vec<(String, Vec<String>)> = case["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["question"].as_str().unwrap().to_owned(),
                    serde_json::from_value(row["options"].clone()).unwrap(),
                )
            })
            .collect();
        assert_eq!(rows(case), expected, "case {}", case["id"]);
    }
}

/// Token ids and answers from a real GGUF against decider's tokenizer and bf16
/// weights. `DECIDER_GGUF=/path/model.gguf`; `DECIDER_TOLERANCE` bounds
/// |Δp| and |Δconfidence| (default 0.01). Measured on the CPU: an f16
/// conversion 0.0025, Q8_0 0.0074, the shipped Q4_K_M 0.12 (the near-flat
/// 12-option question).
#[tokio::test]
#[ignore]
async fn gguf_matches_the_reference() {
    use judge_decider::{download, engine, DeciderClient};
    let gguf = std::env::var("DECIDER_GGUF").expect("DECIDER_GGUF points at the decider GGUF");
    let checkpoint = download::local("decider-4b-v2", gguf.as_ref()).unwrap();
    let options = engine::Options {
        threads: 8,
        gpu_layers: Some(0),
        context_tokens: 4096,
        parallel: 4,
    };
    let fixture = fixture();
    let cases = fixture["cases"].as_array().unwrap();
    let engine = engine::Engine::spawn(&checkpoint, options).unwrap();
    for case in cases {
        let expected: Vec<Vec<i32>> = case["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| serde_json::from_value(row["token_ids"].clone()).unwrap())
            .collect();
        let ids = engine
            .prompt_tokens(case["state"].clone(), rows(case))
            .unwrap();
        assert_eq!(ids, expected, "case {}", case["id"]);
    }
    drop(engine);
    let client = DeciderClient::load(&checkpoint, options).unwrap();
    let request: EvaluateRequest = serde_json::from_value(json!({
        "timeout_ms": 120000,
        "evaluations": cases.iter().map(|case| json!({
            "id": case["id"], "state": case["state"], "questions": case["questions"],
        })).collect::<Vec<_>>(),
    }))
    .unwrap();
    let EvaluateResponse::Ok { results, .. } = client.evaluate(request).await else {
        panic!("evaluation failed");
    };
    let tolerance: f64 = std::env::var("DECIDER_TOLERANCE").map_or(0.01, |t| t.parse().unwrap());
    let mut worst = 0f64;
    for case in cases {
        for (qid, reference) in case["answers"].as_object().unwrap() {
            let answer = &results[case["id"].as_str().unwrap()].answers[qid];
            let expected: Vec<f64> = match answer {
                Answer::Noul { .. } => vec![reference["noul"].as_f64().unwrap()],
                _ => reference["probabilities"]
                    .as_object()
                    .unwrap()
                    .values()
                    .map(|p| p.as_f64().unwrap())
                    .collect(),
            };
            let got: Vec<f64> = match answer {
                Answer::Noul { noul } => vec![*noul],
                Answer::Choice { probabilities, .. } | Answer::Score { probabilities, .. } => {
                    let keys = reference["probabilities"].as_object().unwrap().keys();
                    keys.map(|k| probabilities[k]).collect()
                }
            };
            if let (Answer::Choice { choice, .. }, Some(best)) = (answer, reference.get("choice")) {
                assert_eq!(choice, best.as_str().unwrap(), "{}/{qid}", case["id"]);
            }
            for (g, e) in got.iter().zip(&expected) {
                worst = worst.max((g - e).abs());
            }
            // decider-ai 1.5.0's confidences (TypeSafe's definitions).
            if let (
                Answer::Choice { confidence, .. } | Answer::Score { confidence, .. },
                Some(expected),
            ) = (answer, reference["confidence"].as_f64())
            {
                worst = worst.max((confidence - expected).abs());
            }
            eprintln!("{}/{qid}: {got:.4?} vs {expected:.4?}", case["id"]);
        }
    }
    eprintln!("max |Δp| = {worst:.4}");
    assert!(worst < tolerance, "max |Δp| {worst}");
}
