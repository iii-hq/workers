//! Test helpers: the committed tiny checkpoint (random weights, real tokenizer).
#![allow(dead_code)]
use candle_core::Device;
use judge_laya::{download, LayaClient};
use std::path::PathBuf;

pub fn tiny_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny")
}

pub fn tiny_client() -> LayaClient {
    tiny_client_named(&["laya"])
}

/// The tiny checkpoint loaded once per name (routing tests need several).
pub fn tiny_client_named(names: &[&str]) -> LayaClient {
    let checkpoints: Vec<_> = names
        .iter()
        .map(|name| download::local(name, &tiny_dir()).expect("tiny fixture present"))
        .collect();
    LayaClient::load(&checkpoints, Device::Cpu).expect("tiny checkpoint loads")
}
