use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use context_compaction::{CompactionError, SummariseFn};
use harness_types::AgentMessage;
use iii_sdk::{register_worker, InitOptions};

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

/// Default summariser: produces a placeholder so the proactive compactor
/// can run without a model wired up. Production deployments swap in an
/// LLM-backed summariser at startup.
struct NoopSummariser;

#[async_trait]
impl SummariseFn for NoopSummariser {
    async fn summarise(
        &self,
        _messages: Vec<AgentMessage>,
        _instructions: Option<String>,
    ) -> Result<String, CompactionError> {
        Ok("(compacted; no summariser configured)".into())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let engine_url = std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
    let iii = register_worker(&engine_url, InitOptions::default());

    let _handles = context_compaction::register_with_iii(&iii, Arc::new(NoopSummariser))
        .context("context-compaction register failed")?;
    log::info!(
        "context-compaction registered (context_compaction::watcher, context_compaction::compactor)"
    );

    tokio::signal::ctrl_c().await.ok();
    Ok(())
}
