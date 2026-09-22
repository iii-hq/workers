//! Test helpers: the committed tiny GGUF (random qwen3 weights, byte vocabulary).
#![allow(dead_code)]
use judge_semif::{download, engine, SemifClient};
use std::path::PathBuf;

pub fn tiny_gguf() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny-qwen3.gguf")
}

/// One engine per test binary, like the worker: several llama.cpp contexts
/// loading at once in one process abort under the Vulkan backend.
pub fn tiny_client() -> SemifClient {
    static CLIENT: std::sync::OnceLock<SemifClient> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            let checkpoint =
                download::local("qwen3.5-4b", &tiny_gguf()).expect("tiny fixture present");
            SemifClient::load(
                &checkpoint,
                engine::Options {
                    threads: 2,
                    gpu_layers: Some(0),
                    context_tokens: 2048,
                },
            )
            .expect("tiny GGUF loads")
        })
        .clone()
}
