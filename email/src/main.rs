use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, runtime::WorkerMetadata, InitOptions, RegisterTriggerType};
use std::sync::Arc;

use email::configuration::{self, AppState};

#[derive(Parser, Debug)]
#[command(
    name = "email",
    about = "Email worker — SMTP send and real-time IMAP read with IDLE push"
)]
struct Cli {
    #[arg(long)]
    config: Option<String>,

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

    let _ = rustls::crypto::ring::default_provider().install_default();

    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&email::manifest::build_manifest())?
        );
        return Ok(());
    }

    let seed = match cli.config.as_deref() {
        Some(path) => {
            let cfg = email::config::WorkerConfig::from_file(path)
                .map_err(|e| anyhow::anyhow!("seed config {path}: {e}"))?;
            cfg.validate()
                .map_err(|e| anyhow::anyhow!("seed config {path}: {e}"))?;
            tracing::info!(
                accounts = cfg.accounts.len(),
                path,
                "seeding configuration from file"
            );
            Some(cfg)
        }
        None => None,
    };

    tracing::info!(url = %cli.url, "connecting to iii engine");
    let iii = register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: email::worker_name().to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(|e| anyhow::anyhow!("configuration::register failed: {e}"))?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(|e| anyhow::anyhow!("configuration::get failed: {e}"))?;
    cfg.validate()
        .map_err(|e| anyhow::anyhow!("configuration `{}`: {e}", configuration::config_id()))?;
    tracing::info!(
        accounts = cfg.accounts.len(),
        entry = configuration::config_id(),
        "loaded configuration from the configuration worker"
    );

    email::provider::imap::connection::install_iii_handle(iii.clone());
    let trig_registry = Arc::new(email::triggers::registry::TriggerRegistry::new());
    let dispatcher: Arc<dyn email::triggers::dispatcher::EventDispatcher> = Arc::new(
        email::triggers::dispatcher::EngineDispatcher::new(iii.clone(), trig_registry.clone()),
    );
    let state = AppState::new(cfg, dispatcher);

    email::handlers::register_all(&iii, &state);

    let _new_mail_ref = iii.register_trigger_type(
        RegisterTriggerType::new(
            "email::new-mail",
            "Fires when IMAP IDLE pushes a new message to a configured (account, folder). \
         Payload: { account, folder, uid, message_id, from, subject, snippet, ts }. \
         If the IMAP server lacks the IDLE capability, the account fails at startup (E610).",
            email::triggers::new_mail::Handler {
                registry: trig_registry.clone(),
            },
        )
        .trigger_request_format::<email::triggers::new_mail::NewMailBindingConfig>(),
    );

    let supervised = state.start_idle().await;

    configuration::register_config_trigger(&iii, state.clone())
        .map_err(|e| anyhow::anyhow!("configuration trigger binding failed: {e}"))?;
    configuration::register_config_status(&iii, state.clone());

    tracing::info!(
        "email registered {} functions and 1 trigger type; {} IMAP connections supervised",
        email::handlers::REGISTERED_FN_COUNT,
        supervised,
    );

    wait_for_shutdown_signal().await?;
    tracing::info!("email shutting down");
    iii.shutdown_async().await;
    state.stop_idle().await;
    Ok(())
}

async fn wait_for_shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate())?;
        tokio::select! {
            r = tokio::signal::ctrl_c() => r,
            _ = sigterm.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}
