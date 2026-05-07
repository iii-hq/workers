//! Shared boot harness for the harness crate's Phase A E2E acceptance test.
//!
//! Spawns an `iii` engine and the `iii-session-tree` worker, returning a
//! handle whose [`Drop`] kills both. Tests skip gracefully if the `iii`
//! binary is not on PATH or the session-tree worker binary cannot be
//! located, so plain `cargo test` stays green.
//!
//! Mirrors `turn-orchestrator/tests/common/mod.rs` — both crates sit at the
//! workspace root and reference `../session-tree/target/...` symmetrically.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iii_sdk::{register_worker, InitOptions, III};
use tokio::time::sleep;

pub const ENGINE_WS: &str = "ws://127.0.0.1:49134";

pub struct Harness {
    pub iii: III,
    engine: Child,
    worker: Child,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.worker.kill();
        let _ = self.worker.wait();
        let _ = self.engine.kill();
        let _ = self.engine.wait();
    }
}

impl Harness {
    /// Boot iii engine + iii-session-tree worker. Returns `None` and logs to
    /// stderr if the test environment is missing prerequisites.
    pub async fn boot() -> Option<Self> {
        let Ok(iii_bin) = which::which("iii") else {
            eprintln!("skipping: `iii` binary not on PATH");
            return None;
        };

        let Some(worker_bin) = find_session_tree_binary() else {
            eprintln!(
                "skipping: `iii-session-tree` binary not found; \
                 run `cargo build --bin iii-session-tree` in ../session-tree first"
            );
            return None;
        };

        let Some(config_path) = find_session_tree_memory_config() else {
            eprintln!("skipping: session-tree integration_memory.yaml not found");
            return None;
        };

        let engine = Command::new(&iii_bin)
            .arg("--use-default-config")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        sleep(Duration::from_millis(800)).await;

        let worker = Command::new(&worker_bin)
            .args([
                "--url",
                ENGINE_WS,
                "--config",
                config_path.to_str().unwrap_or(""),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        sleep(Duration::from_millis(1500)).await;

        let iii = register_worker(ENGINE_WS, InitOptions::default());
        sleep(Duration::from_millis(500)).await;

        Some(Self {
            iii,
            engine,
            worker,
        })
    }
}

fn find_session_tree_binary() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("IIITEST_SESSION_TREE_BIN") {
        let buf = PathBuf::from(p);
        if buf.is_file() {
            return Some(buf);
        }
    }
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let session_tree_dir = manifest_dir.parent()?.join("session-tree");
    for profile in ["debug", "release"] {
        let candidate = session_tree_dir
            .join("target")
            .join(profile)
            .join("iii-session-tree");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn find_session_tree_memory_config() -> Option<PathBuf> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest_dir
        .parent()?
        .join("session-tree")
        .join("tests")
        .join("integration_memory.yaml");
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}

pub fn nonce() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos}")
}
