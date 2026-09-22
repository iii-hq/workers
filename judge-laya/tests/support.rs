//! Test helpers: the committed tiny checkpoint (random weights, real tokenizer).
#![allow(dead_code)]
use candle_core::Device;
use judge_laya::{download, LayaClient};
use std::path::PathBuf;

pub fn tiny_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny")
}

pub fn tiny_client() -> LayaClient {
    let checkpoint = download::local("laya", &tiny_dir()).expect("tiny fixture present");
    LayaClient::load(&checkpoint, Device::Cpu).expect("tiny checkpoint loads")
}
