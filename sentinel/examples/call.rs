//! A one-shot client for calling this worker on a running stack.
//!
//! There is no `iii` call CLI, and the console cannot reach a worker that has
//! no page yet — so proving a change against a real engine needs *something*
//! that speaks the protocol. This is that something, and it is how the
//! investigation path was proven end to end: open one, watch the agent read
//! the mapped checkout, and see the diagnosis land.
//!
//! ```text
//! III_NAMESPACE=my-project cargo run --example call -- \
//!   sentinel::groups::list '{"limit":10}'
//! ```
//!
//! The namespace matters: a call routes to the caller's namespace, so a probe
//! registered in `default` cannot reach a worker running in `my-project`.

use std::env;
use std::time::Duration;

use anyhow::{Context, Result};
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};

/// Long enough for an investigation to open, which is the slowest call here.
const TIMEOUT_MS: u64 = 120_000;

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let function_id = args.next().context("usage: call <function_id> [json]")?;
    let payload = match args.next() {
        Some(raw) => serde_json::from_str(&raw).context("the payload is not JSON")?,
        None => serde_json::json!({}),
    };

    let iii = register_worker(
        &env::var("III_URL").unwrap_or_else(|_| "ws://127.0.0.1:49134".into()),
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                version: "0.0.0".into(),
                name: "sentinel-probe".into(),
                os: std::env::consts::OS.into(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    // Registration is asynchronous; a call sent before it lands is refused.
    tokio::time::sleep(Duration::from_millis(400)).await;

    let response = iii
        .trigger(TriggerRequest {
            function_id,
            payload,
            action: None,
            timeout_ms: Some(TIMEOUT_MS),
        })
        .await;
    match response {
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value)?),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
    iii.shutdown_async().await;
    Ok(())
}
