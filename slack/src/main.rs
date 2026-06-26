use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use tokio::sync::RwLock;

use slack::approval_gate::{self, ApprovalGateStatus};
use slack::clients::slack as slack_client;
use slack::configuration::{self, ConfigCell};
use slack::deps::Deps;
use slack::functions::{self, bindings};
use slack::{ingress, WorkerConfig};

#[derive(Parser, Debug)]
#[command(
    name = "slack",
    about = "Slack Web API + harness bridge as iii functions."
)]
struct Cli {
    #[arg(long)]
    config: Option<String>,

    #[arg(long, default_value = "ws://127.0.0.1:49134")]
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
        let m = slack::manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m).unwrap());
        return Ok(());
    }

    let seed = cli.config.as_deref().and_then(|path| match WorkerConfig::from_file(path) {
        Ok(c) => Some(c),
        Err(e) => {
            tracing::warn!(error = %e, path, "failed to parse --config seed; continuing without seed");
            None
        }
    });

    let iii = register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "slack".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering slack configuration schema")?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading slack configuration")?;
    cfg.validate()
        .map_err(anyhow::Error::msg)
        .context("slack requires a non-empty bot_token in configuration")?;

    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg)));
    let gate = Arc::new(ApprovalGateStatus::new());
    let deps = Arc::new(Deps::new(iii.clone(), cell.clone(), gate.clone()));

    functions::register_all(&iii, &deps);

    // Probe the optional approval-gate worker; bind approval triggers if present.
    gate.set_present(approval_gate::probe_presence(&iii).await);
    bindings::bind_session_triggers(&iii);
    if gate.is_available() {
        bindings::bind_approval_triggers(&iii);
        tracing::info!("approval-gate present — inline approvals enabled");
    } else {
        tracing::info!("approval-gate absent — approvals disabled");
    }

    configuration::register_config_trigger(&iii, cell, deps.clone())
        .map_err(anyhow::Error::msg)
        .context("binding configuration trigger")?;

    // Best-effort identity discovery (does not block boot if Slack is unreachable).
    match slack_client::auth_test(&deps).await {
        Ok(id) => {
            tracing::info!(team = ?id.team, bot = ?id.user_id, "slack identity resolved");
            *deps.identity.write().await = Some(id);
        }
        Err(e) => tracing::warn!(error = %e, "auth.test failed at boot (check bot_token)"),
    }

    // Register the bridge HTTP routes if the bridge is configured.
    ingress::apply(&deps).await;

    tracing::info!("slack worker ready, waiting for invocations");
    tokio::signal::ctrl_c().await?;
    tracing::info!("slack worker shutting down");
    ingress::shutdown(&deps).await;
    iii.shutdown_async().await;
    Ok(())
}
