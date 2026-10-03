//! The backbone engine over the committed tiny GGUF (random qwen3 weights,
//! byte vocabulary, n_embd 32). One test, so the llama.cpp loads in this
//! binary stay sequential. Qwen3.5's recurrent carry across chunks is checked
//! end to end on the real checkpoint.
use judge_clef::engine::{Engine, Options, Stop};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};

fn tiny(batch_tokens: u32) -> Engine {
    let gguf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny/backbone.gguf");
    let options = Options {
        threads: 2,
        gpu_layers: Some(0),
        context_tokens: 1024,
    };
    Engine::spawn(&gguf, options, batch_tokens).expect("tiny GGUF loads")
}

fn states(engine: &Engine, ids: &[u32], cancel: bool) -> Result<Vec<f32>, Stop> {
    let deadline = Instant::now() + Duration::from_secs(60);
    engine
        .states(ids.to_vec(), deadline, Arc::new(AtomicBool::new(cancel)))
        .blocking_recv()
        .expect("the runtime answers")
}

#[test]
fn tiny_chunks_match_one_pass_and_requests_forget() {
    // Token id = byte value in the tiny vocabulary; the last chunk of 64 is partial.
    let ids: Vec<u32> = (0..300).map(|i| i * 37 % 256).collect();
    let (one_pass, chunked) = (tiny(1024), tiny(64));
    assert_eq!((one_pass.hidden, chunked.hidden), (32, 32));
    let whole = states(&one_pass, &ids, false).unwrap();
    let first = states(&chunked, &ids, false).unwrap();
    assert_eq!(first.len(), ids.len() * 32);
    // llama.cpp's flash-attention kernels change with the ubatch size (f16
    // on the CPU below 64 queries): 2.4e-2 measured on states up to 3.8. With
    // flash attention off the two are bit-equal; losing the carry between
    // chunks drifts every row by more than 1.
    let worst = whole
        .iter()
        .zip(&first)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(worst < 5e-2, "chunked states drift by {worst}");

    let other: Vec<u32> = ids.iter().rev().copied().collect();
    states(&chunked, &other, false).unwrap();
    assert_eq!(states(&chunked, &ids, false).unwrap(), first);

    assert_eq!(states(&chunked, &[1; 1025], false), Err(Stop::TooLong));
    assert_eq!(states(&chunked, &ids, true), Err(Stop::Cancelled));
}
