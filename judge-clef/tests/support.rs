//! Test helpers: the committed tiny checkpoint (a random clef GGUF, n_embd 32,
//! with a byte vocabulary, and its byte-level tokenizer).
#![allow(dead_code)]
use judge_clef::{download, engine, ClefClient};
use std::path::PathBuf;

pub fn tiny_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny")
}

/// One model per test binary, like the worker.
pub fn tiny_client() -> ClefClient {
    static CLIENT: std::sync::OnceLock<ClefClient> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            let checkpoint =
                download::local("clef-flash", &tiny_dir()).expect("tiny fixture present");
            ClefClient::load(
                &checkpoint,
                engine::Options {
                    threads: 2,
                    gpu_layers: Some(0),
                    context_tokens: 2048,
                },
            )
            .expect("tiny checkpoint loads")
        })
        .clone()
}
