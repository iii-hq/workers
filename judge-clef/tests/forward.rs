//! The engine over the committed tiny clef GGUF (random weights, byte
//! vocabulary, n_embd 32) on the CPU: one forward per prompt.
use judge_clef::{
    encode::{encode, Encoded},
    engine::{Engine, Options, Stop},
};
use judge_contract::Evaluation;
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};

/// A choice of 3 options and a noul, tokenized as bytes (the tiny vocabulary).
fn prompt(state: &str) -> Encoded {
    let evaluation: Evaluation = serde_json::from_value(json!({
        "id": "e", "state": state,
        "questions": {
            "team": {"type": "choice", "instructions": "Which team?", "criteria": {"billing": null, "sales": null, "support": "bugs"}},
            "urgent": {"type": "noul", "instructions": "Urgent?"}
        }
    }))
    .unwrap();
    encode(&evaluation, 1024, |text| {
        Ok(text.bytes().map(u32::from).collect())
    })
    .unwrap()
}

async fn decide(
    engine: &Engine,
    prompt: &Encoded,
    deadline: Instant,
    cancel: bool,
) -> Result<Vec<f32>, Stop> {
    let cancel = Arc::new(AtomicBool::new(cancel));
    engine.decide(prompt.clone(), deadline, cancel).await
}

#[tokio::test]
async fn one_forward_per_prompt_is_deterministic_and_forgets() {
    let gguf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny/model.gguf");
    let options = Options {
        threads: 2,
        gpu_layers: Some(0),
        context_tokens: 1024,
    };
    let engine = Engine::load(&gguf, options).expect("tiny GGUF loads");
    assert_eq!(&*engine.device, "CPU");
    let later = Instant::now() + Duration::from_secs(60);
    let (a, b) = (
        prompt("Billed twice, refund now."),
        prompt("The site is down."),
    );

    let first = decide(&engine, &a, later, false).await.unwrap();
    // One score per option in prompt order: billing, sales, support, true, false.
    assert_eq!(first.len(), 5);
    assert!(first.iter().all(|score| score.is_finite()), "{first:?}");
    let other = decide(&engine, &b, later, false).await.unwrap();
    assert_ne!(other, first, "the scores follow the state");
    // Concurrent calls take turns and match the sequential ones bit for bit.
    let (again, other_again) = tokio::join!(
        decide(&engine, &a, later, false),
        decide(&engine, &b, later, false)
    );
    assert_eq!(
        (again.unwrap(), other_again.unwrap()),
        (first.clone(), other)
    );

    // Refused before the pass, and the engine still answers afterwards.
    let mut long = a.clone();
    long.ids.resize(1025, u32::from(b'x'));
    assert_eq!(
        decide(&engine, &long, later, false).await,
        Err(Stop::TooLong)
    );
    assert_eq!(decide(&engine, &a, later, true).await, Err(Stop::Cancelled));
    assert_eq!(
        decide(&engine, &a, Instant::now(), false).await,
        Err(Stop::Deadline)
    );
    assert_eq!(decide(&engine, &a, later, false).await.unwrap(), first);
}
