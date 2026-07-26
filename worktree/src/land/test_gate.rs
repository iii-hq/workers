//! The land test gate. The worker never execs arbitrary commands itself;
//! production delegates to `shell::exec` with the worktree as `cwd` and as
//! the `fs_scope` root, so the shell worker's allowlist, denylist, jail,
//! and scope enforcement apply. Behind a trait so the land machine is
//! testable in-process.

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::json;

use crate::error::{codes, WError};

#[derive(Debug, Clone)]
pub struct TestOutcome {
    pub exit_code: i32,
    /// Trailing output kept for the land-blocked event message.
    pub tail: String,
}

#[async_trait]
pub trait TestRunner: Send + Sync {
    async fn run(
        &self,
        cmd: &str,
        worktree_path: &str,
        timeout_ms: u64,
    ) -> Result<TestOutcome, WError>;
}

/// Production gate: `shell::exec` with `cwd` and the `fs_scope` root pinned
/// to the worktree (this worker calls shell directly as a trusted peer, so
/// it stamps its own scope). A jail rejection (`S215`) means `worktree_root`
/// is not inside the shell worker's `fs.host_roots`; surface that
/// misconfiguration explicitly instead of a generic failure.
pub struct ShellExecRunner {
    iii: Arc<IIIClient>,
}

impl ShellExecRunner {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

const TRIGGER_MARGIN_MS: u64 = 30_000;

#[async_trait]
impl TestRunner for ShellExecRunner {
    async fn run(
        &self,
        cmd: &str,
        worktree_path: &str,
        timeout_ms: u64,
    ) -> Result<TestOutcome, WError> {
        let request = TriggerRequest {
            function_id: "shell::exec".into(),
            payload: json!({
                "command": cmd,
                "cwd": worktree_path,
                "fs_scope": { "root": worktree_path, "grants": [] },
                "timeout_ms": timeout_ms,
            }),
            action: None,
            timeout_ms: Some(timeout_ms.saturating_add(TRIGGER_MARGIN_MS)),
        };
        let result = match self.iii.namespace() {
            Some(ns) => self.iii.trigger(request.namespace(ns)).await,
            None => self.iii.trigger(request).await,
        };
        match result {
            Ok(v) => {
                let timed_out = v["timed_out"].as_bool().unwrap_or(false);
                let exit_code = v["exit_code"].as_i64().map(|c| c as i32).unwrap_or(-1);
                let exit_code = if timed_out { -1 } else { exit_code };
                let stderr = v["stderr"].as_str().unwrap_or("");
                let stdout = v["stdout"].as_str().unwrap_or("");
                let source = if stderr.trim().is_empty() {
                    stdout
                } else {
                    stderr
                };
                let tail: String = source
                    .lines()
                    .rev()
                    .take(20)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n");
                let tail = if timed_out {
                    format!("test command timed out after {timeout_ms} ms\n{tail}")
                } else {
                    tail
                };
                Ok(TestOutcome { exit_code, tail })
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("S215") {
                    return Err(WError::new(
                        codes::TEST_FAILED,
                        format!(
                            "shell::exec rejected the worktree path (S215): worktree_root is \
                             not inside the shell worker's fs.host_roots — add it to \
                             fs.host_roots or move worktree_root. Original error: {msg}"
                        ),
                    ));
                }
                Err(WError::new(
                    codes::STATE_UNAVAILABLE,
                    format!("shell::exec dispatch failed: {msg}"),
                ))
            }
        }
    }
}
