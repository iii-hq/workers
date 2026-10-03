//! Test helpers: the committed tiny checkpoint (random qwen3 backbone with a
//! byte vocabulary, n_embd 32; a seeded joint head; a byte-level tokenizer).
#![allow(dead_code)]
use judge_clef::{download, engine, ClefClient};
use std::path::PathBuf;

pub fn tiny_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny")
}

/// One engine per test binary, like the worker: several llama.cpp contexts
/// loading at once in one process abort under the Vulkan backend.
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
