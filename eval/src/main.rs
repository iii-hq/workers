use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_helpers::observability::OtelConfig;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::trigger::Trigger;
use iii_sdk::{register_worker, InitOptions};
use serde_json::json;

use eval::events::EvalEvents;
use eval::locks::EvalLocks;
use eval::runtime::Deps;
use eval::{functions, manifest, queue, state, ui};

#[derive(Parser, Debug)]
#[command(
    name = "eval",
    about = "Live comparison of Harness session metrics, with prompt experiments as an advanced surface."
)]
struct Cli {
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    #[arg(long)]
    manifest: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest::build_manifest()).unwrap()
        );
        return Ok(());
    }

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "eval".into(),
                os: std::env::consts::OS.into(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            otel: Some(OtelConfig::default()),
            ..InitOptions::default()
        },
    ));

    let events = EvalEvents::register(&iii);
    let deps = Deps {
        iii: iii.clone(),
        locks: EvalLocks::default(),
        events,
    };
    functions::register_all(&iii, &deps);
    ui::register(&iii);
    queue::ensure_run_queue(&iii)
        .await
        .context("ensuring eval-run queue")?;

    let _trigger_handles = bind_triggers(&iii);

    match state::list_jobs(&iii).await {
        Ok(jobs) => {
            for job in jobs.iter().filter(|job| !job.status.is_terminal()) {
                if let Err(error) = queue::enqueue_step(&iii, &job.evaluation_id, job.step).await {
                    tracing::warn!(
                        evaluation_id = %job.evaluation_id,
                        %error,
                        "eval startup recovery could not enqueue step"
                    );
                }
            }
        }
        Err(error) => tracing::warn!(%error, "eval startup recovery could not list jobs"),
    }

    tracing::info!("eval ready");
    tokio::signal::ctrl_c().await?;
    iii.shutdown_async().await;
    Ok(())
}

fn bind_triggers(iii: &Arc<iii_sdk::IIIClient>) -> Vec<Trigger> {
    let mut handles = Vec::new();
    for (trigger_type, function_id, config) in [
        ("harness::turn-completed", functions::WAKE_ID, json!({})),
        (
            "cron",
            functions::SWEEP_ID,
            json!({ "expression": "*/15 * * * * *" }),
        ),
    ] {
        match iii.register_trigger(RegisterTriggerInput {
            trigger_type: trigger_type.into(),
            function_id: function_id.into(),
            config,
            metadata: None,
        }) {
            Ok(handle) => handles.push(handle),
            Err(error) => tracing::warn!(
                trigger_type,
                function_id,
                %error,
                "eval trigger binding failed; recovery remains available on restart"
            ),
        }
    }
    handles
}
