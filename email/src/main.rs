use anyhow::Result;
use clap::Parser;
use iii_sdk::{
    register_worker, IIIError, InitOptions, RegisterTriggerInput, RegisterTriggerType,
    WorkerMetadata,
};
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(
    name = "email",
    about = "Email worker for iii agents (SMTP + IMAP IDLE)"
)]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

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

    // rustls 0.23 requires explicit CryptoProvider install before any
    // ClientConfig::builder() call. Idempotent — safe to call once at startup.
    let _ = rustls::crypto::ring::default_provider().install_default();

    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&email::manifest::build_manifest())?
        );
        return Ok(());
    }

    let cfg = match email::config::WorkerConfig::from_file(&cli.config) {
        Ok(c) => {
            tracing::info!(
                accounts = c.accounts.len(),
                "loaded config from {}",
                cli.config
            );
            c
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "config load failed, using defaults");
            email::config::WorkerConfig::default()
        }
    };
    let cfg = Arc::new(cfg);

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
    email::provider::imap::connection::install_iii_handle(iii.clone());

    let imap_pool = Arc::new(email::provider::imap::ImapPool::new(cfg.clone()));
    let trig_registry = Arc::new(email::triggers::registry::TriggerRegistry::new());
    let dispatcher: Arc<dyn email::triggers::dispatcher::EventDispatcher> = Arc::new(
        email::triggers::dispatcher::EngineDispatcher::new(iii.clone(), trig_registry.clone()),
    );

    email::handlers::register_all(&iii, &cfg, &imap_pool);

    let _new_mail_ref = iii.register_trigger_type(RegisterTriggerType::new(
        "email::new-mail",
        "Fires when IMAP IDLE pushes a new message to a configured (account, folder). \
         Payload: { account, folder, uid, message_id, from, subject, snippet, ts }. \
         If the IMAP server lacks the IDLE capability, the account fails at startup (E610).",
        email::triggers::new_mail::Handler {
            registry: trig_registry.clone(),
        },
    ));

    // Spawn one persistent IMAP+IDLE connection per (account, folder).
    // E610 here is fatal — we refuse to silently degrade to polling.
    let mut idle_handles = Vec::new();
    for (acct_name, acct_cfg) in cfg.accounts.iter() {
        if acct_cfg.provider != email::config::Provider::Imap {
            continue;
        }
        let Some(imap) = acct_cfg.imap.clone() else {
            tracing::warn!(account = %acct_name, "provider=imap but imap config missing, skipping");
            continue;
        };
        for folder in imap.folders.iter().cloned() {
            let name = acct_name.clone();
            let cfg_clone = cfg.clone();
            let dispatcher = dispatcher.clone();
            let h = tokio::spawn(async move {
                email::provider::imap::connection::run_until_shutdown(
                    name, folder, cfg_clone, dispatcher,
                )
                .await
            });
            idle_handles.push(h);
        }
    }

    tracing::info!(
        "email registered {} functions and 1 trigger type; {} IMAP connections supervised",
        email::handlers::REGISTERED_FN_COUNT,
        idle_handles.len(),
    );

    wait_for_shutdown_signal().await?;
    tracing::info!("email shutting down");
    iii.shutdown_async().await;
    for h in idle_handles {
        h.abort();
    }
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

#[allow(dead_code)]
fn _suppress_unused(_: IIIError, _: RegisterTriggerInput) {}
