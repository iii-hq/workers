use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{
    register_worker, InitOptions, OtelConfig, RegisterTriggerInput, TriggerRequest, WorkerMetadata,
};
use serde_json::json;
use tokio::task::JoinHandle;
use tracing_subscriber::EnvFilter;

mod manifest;

use iii_auth::config::{resolve_store_backend, validate_config, AuthConfig, StoreBackend};
use iii_auth::store::{IiiStateAuthStore, InMemoryAuthStore};

#[derive(Parser, Debug)]
#[command(
    name = "iii-auth",
    about = "OAuth authority worker for iii RBAC, discovery, DCR, JWKS, and token validation."
)]
struct Cli {
    #[arg(long, env = "III_AUTH_CONFIG", default_value = "./config.yaml")]
    config: String,

    #[arg(long, env = "III_URL")]
    url: Option<String>,

    #[arg(long)]
    issuer: Option<String>,

    #[arg(long)]
    idp_mode: Option<String>,

    #[arg(long)]
    rotation_cron: Option<String>,

    #[arg(long)]
    manifest: bool,
}

const CONNECTION_READY_SETTLE: Duration = Duration::from_millis(50);
const SKILL_REGISTER_TIMEOUT: Duration = Duration::new(3 * 60, 0);
const SKILL_REGISTER_MAX_BACKOFF: Duration = Duration::new(60, 0);
const SKILL_REGISTER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    if cli.manifest {
        let m = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m)?);
        return Ok(());
    }

    let mut cfg = iii_auth::config::load_config(&cli.config)
        .with_context(|| format!("failed to load auth config from {}", cli.config))?;
    if let Some(issuer) = cli.issuer {
        cfg.issuer = issuer;
    }
    if let Some(idp_mode) = cli.idp_mode {
        cfg.idp_mode = idp_mode;
    }
    if let Some(rotation_cron) = cli.rotation_cron {
        cfg.rotation_cron = rotation_cron;
    }
    validate_config(&cfg).context("invalid auth config")?;
    let engine_url = cli.url.unwrap_or_else(|| cfg.engine_url.clone());
    let cfg = Arc::new(cfg);

    let iii = Arc::new(register_worker(
        &engine_url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "auth".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    wait_for_connection_ready(&iii, &cfg).await?;

    let store: Arc<dyn iii_auth::store::AuthStore> = match resolve_store_backend(&cfg) {
        StoreBackend::Memory => Arc::new(InMemoryAuthStore::new()),
        StoreBackend::IiiState => {
            let iii_for_store: Arc<dyn iii_auth::io::IIITrigger> = iii.clone();
            Arc::new(IiiStateAuthStore::new(iii_for_store, cfg.state_timeout_ms))
        }
    };

    let _refs = iii_auth::register_with_iii(&iii, store, cfg.clone())
        .await
        .context("auth register failed")?;

    register_triggers(&iii, &cfg).context("auth trigger registration failed")?;
    let skill_register_handle = spawn_skill_register(iii.clone(), cfg.clone());

    tracing::info!("auth ready");
    wait_for_shutdown().await?;
    skill_register_handle.abort();
    let _ = tokio::time::timeout(SKILL_REGISTER_SHUTDOWN_TIMEOUT, skill_register_handle).await;
    unregister_skill(&iii, &cfg).await;
    tracing::info!("auth shutting down");
    iii.shutdown_async().await;
    Ok(())
}

fn register_triggers(iii: &iii_sdk::III, cfg: &AuthConfig) -> Result<()> {
    let http_routes = [
        (
            "auth::server_metadata",
            "GET",
            ".well-known/oauth-authorization-server",
        ),
        (
            "auth::resource_metadata",
            "GET",
            ".well-known/oauth-protected-resource",
        ),
        ("auth::register", "POST", "register"),
        ("auth::jwks", "GET", ".well-known/jwks.json"),
        ("auth::token", "POST", "token"),
        ("auth::introspect", "POST", "introspect"),
        ("auth::revoke", "POST", "revoke"),
    ];
    for (function_id, method, api_path) in http_routes {
        iii.register_trigger(RegisterTriggerInput {
            trigger_type: "http".to_string(),
            function_id: function_id.to_string(),
            config: json!({ "api_path": api_path, "http_method": method }),
            metadata: None,
        })
        .with_context(|| format!("failed to register {method} {api_path} for {function_id}"))?;
    }
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "cron".to_string(),
        function_id: "auth::jwks_rotate".to_string(),
        config: json!({ "expression": cfg.rotation_cron }),
        metadata: None,
    })
    .context("failed to register JWKS rotation trigger")?;
    Ok(())
}

async fn register_skill_with_retry(iii: &iii_sdk::III, id: &str, body: &str, timeout_ms: u64) {
    let mut backoff = Duration::from_secs(5);
    let started = Instant::now();
    loop {
        let res = iii
            .trigger(TriggerRequest {
                function_id: "skills::register".into(),
                payload: json!({ "id": id, "skill": body }),
                action: None,
                timeout_ms: Some(timeout_ms),
            })
            .await;
        match res {
            Ok(_) => return,
            Err(e) => {
                if started.elapsed() > SKILL_REGISTER_TIMEOUT {
                    tracing::warn!(%id, error = %e, "skills handshake gave up");
                    return;
                }
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(SKILL_REGISTER_MAX_BACKOFF);
    }
}

fn spawn_skill_register(iii: Arc<iii_sdk::III>, cfg: Arc<AuthConfig>) -> JoinHandle<()> {
    tokio::spawn(async move {
        register_skill_with_retry(
            &iii,
            iii_auth::SKILL_ID,
            iii_auth::SKILL_MD,
            cfg.skills_register_timeout_ms,
        )
        .await;
        for (id, body) in iii_auth::SUB_SKILLS {
            register_skill_with_retry(&iii, id, body, cfg.skills_register_timeout_ms).await;
        }
    })
}

async fn unregister_skill(iii: &Arc<iii_sdk::III>, cfg: &Arc<AuthConfig>) {
    for (id, _) in iii_auth::SUB_SKILLS {
        let _ = iii
            .trigger(TriggerRequest {
                function_id: "skills::unregister".into(),
                payload: json!({ "id": id }),
                action: None,
                timeout_ms: Some(cfg.skills_unregister_timeout_ms),
            })
            .await;
    }
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "skills::unregister".into(),
            payload: json!({ "id": iii_auth::SKILL_ID }),
            action: None,
            timeout_ms: Some(cfg.skills_unregister_timeout_ms),
        })
        .await;
}

async fn wait_for_connection_ready(iii: &iii_sdk::III, cfg: &AuthConfig) -> Result<()> {
    let interval = Duration::from_millis(cfg.connection_ready_interval_ms);
    for attempt in 1..=cfg.connection_ready_attempts {
        let state = iii.get_connection_state();
        if state == iii_sdk::IIIConnectionState::Connected {
            tokio::time::sleep(CONNECTION_READY_SETTLE).await;
            return Ok(());
        }
        tracing::debug!(attempt, state = ?state, "iii engine connection not ready");
        tokio::time::sleep(interval).await;
    }
    anyhow::bail!(
        "timed out waiting for iii engine connection after {} attempts",
        cfg.connection_ready_attempts
    )
}

async fn wait_for_shutdown() -> Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).context("failed to install SIGTERM handler")?;
        tokio::select! {
            r = tokio::signal::ctrl_c() => r.context("failed to await SIGINT")?,
            _ = sigterm.recv() => {}
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .context("failed to await SIGINT")
    }
}
