//! Test helpers: the committed tiny checkpoint (random weights, real tokenizer).
#![allow(dead_code)]
use judge_laya::{download, engine::Options, LayaClient};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

pub fn tiny_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny")
}

pub fn tiny_client() -> LayaClient {
    tiny_client_named(&["laya"])
}

/// The tiny checkpoint loaded once per name set and shared by every test in
/// the process: several llama.cpp contexts loading at once crash on Vulkan,
/// and the CPU keeps the tests deterministic.
pub fn tiny_client_named(names: &[&str]) -> LayaClient {
    static CLIENTS: OnceLock<Mutex<BTreeMap<Vec<String>, LayaClient>>> = OnceLock::new();
    let key: Vec<String> = names.iter().map(|name| name.to_string()).collect();
    let mut cache = CLIENTS.get_or_init(Default::default).lock().unwrap();
    cache
        .entry(key)
        .or_insert_with(|| {
            let checkpoints: Vec<_> = names
                .iter()
                .map(|name| download::local(name, &tiny_dir()).expect("tiny fixture present"))
                .collect();
            let options = Options {
                gpu_layers: Some(0),
                ..Options::default()
            };
            LayaClient::load(&checkpoints, options).expect("tiny checkpoint loads")
        })
        .clone()
}
