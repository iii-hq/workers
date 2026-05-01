//! Standalone `guardrails` worker. Registers `guardrails::*` (input/output
//! checks, classify) on the iii engine and runs until Ctrl-C.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use iii_sdk::{register_worker, FunctionInfo, InitOptions, TriggerRequest, Value, III};
use serde_json::json;

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

async fn list_functions(iii: &III) -> Result<Vec<FunctionInfo>> {
    let value = iii
        .trigger(TriggerRequest {
            function_id: "engine::functions::list".to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: None,
        })
        .await?;
    Ok(serde_json::from_value(
        value
            .get("functions")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    )?)
}

fn parse_args(args: Vec<String>) -> Result<String> {
    let mut engine_url = DEFAULT_ENGINE_URL.to_string();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--engine-url" | "--url" => {
                engine_url = iter
                    .next()
                    .ok_or_else(|| anyhow!("--engine-url requires a value"))?;
            }
            "--help" | "-h" => {
                println!("iii-guardrails [--engine-url <ws>]");
                std::process::exit(0);
            }
            "--version" | "-V" => {
                println!("iii-guardrails {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            other => return Err(anyhow!("unknown flag: {other}")),
        }
    }
    Ok(engine_url)
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let engine_url = parse_args(std::env::args().skip(1).collect())?;
    log::info!("connecting to iii engine at {engine_url}");
    let iii = register_worker(&engine_url, InitOptions::default());
    let iii = Arc::new(iii);

    list_functions(&iii)
        .await
        .with_context(|| format!("engine unreachable at {engine_url}"))?;
    log::info!("engine connection ok");

    let _refs = guardrails::register_with_iii(&iii)
        .await
        .context("guardrails register failed")?;
    log::info!("registered: guardrails (3 guardrails::* fns)");

    log::info!("guardrails ready — waiting for requests (Ctrl-C to exit)");
    tokio::signal::ctrl_c().await.ok();
    log::info!("shutdown requested");
    Ok(())
}
