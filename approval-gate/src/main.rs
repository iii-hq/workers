//! Binary entrypoint for the approval-gate worker.

use std::sync::Arc;

use anyhow::{Context, Result};
use approval_gate::{
    config::{load_config, WorkerConfig},
    manifest, register,
};
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, WorkerMetadata};

#[derive(Parser, Debug)]
#[command(
    name = "approval-gate",
    about = "Approval gate subscriber for agent::before_function_call"
)]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,

    #[arg(long)]
    manifest: bool,
}

fn apply_runtime_env_overrides(cfg: &mut WorkerConfig) {
    if let Ok(t) = std::env::var("APPROVAL_GATE_TOPIC") {
        let t = t.trim();
        if !t.is_empty() {
            cfg.topic = t.to_string();
        }
    }
    if let Ok(s) = std::env::var("APPROVAL_GATE_STATE_SCOPE") {
        let s = s.trim();
        if !s.is_empty() {
            cfg.approval_state_scope = s.to_string();
        }
    }
    if let Ok(raw) = std::env::var("APPROVAL_GATE_TIMEOUT_MS") {
        match raw.parse::<u64>() {
            Ok(n) => cfg.default_timeout_ms = n,
            Err(err) => tracing::warn!(
                "APPROVAL_GATE_TIMEOUT_MS={raw:?} is not a valid u64 ({err}); keeping {}ms from config files",
                cfg.default_timeout_ms
            ),
        }
    }
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    match signal(SignalKind::terminate()) {
        Ok(mut sigterm) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = sigterm.recv() => {},
            }
        }
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    tokio::signal::ctrl_c().await.ok();
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
        let m = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m)?);
        return Ok(());
    }

    let mut cfg = match load_config(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %cli.config,
                "failed to load config; using defaults"
            );
            WorkerConfig::default()
        }
    };
    apply_runtime_env_overrides(&mut cfg);

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "approval-gate".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    let _refs = register(&iii, &cfg).context("approval-gate register failed")?;

    tracing::info!(
        topic = %cfg.topic,
        state_scope = %cfg.approval_state_scope,
        default_timeout_ms = cfg.default_timeout_ms,
        "approval-gate subscribed (approval::*, policy::approval_gate)",
    );

    shutdown_signal().await;

    tracing::info!("approval-gate shutting down");
    iii.shutdown_async().await;

    Ok(())
}
