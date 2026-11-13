//! `mcp-server` binary entry. Connects to the iii engine (normally through
//! rbac-proxy), refuses to start unless that connection is proxied or
//! `III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC` is set, registers `mcp-server::handler`
//! and `mcp-server::observed` once the http worker is known to listen on loopback
//! (or `III_MCP_DANGEROUSLY_ALLOW_PUBLIC_HTTP` is set), serves
//! `POST`/`GET`/`DELETE <api_path>`, ends idle MCP sessions, then sleeps on
//! Ctrl+C.
//!
//! All real logic lives under [`mcp-server`]; tests under `tests/` reuse the
//! same modules to drive the bridge in-process.

use anyhow::Result;
use clap::Parser;
use iii_mcp_server::bus::Bus;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

use iii_mcp_server::engine::SdkEngine;
use iii_mcp_server::http_guard;
use iii_mcp_server::rbac_guard::{self, Decision};
use iii_mcp_server::sessions::Sessions;
use iii_mcp_server::{config, functions, manifest};

use crate::config::McpConfig;

#[derive(Parser, Debug)]
#[command(
    name = "mcp-server",
    about = "MCP 2025-06-18 Streamable HTTP bridge for the iii engine."
)]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    /// Engine URL; normally rbac-proxy's. Falls back to III_MCP_ENGINE_URL,
    /// then III_URL.
    #[arg(long)]
    url: Option<String>,

    #[arg(long)]
    manifest: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let url = config::engine_url(
        cli.url.clone(),
        std::env::var("III_MCP_ENGINE_URL").ok(),
        std::env::var("III_URL").ok(),
    );

    if cli.manifest {
        let m = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m).unwrap());
        return Ok(());
    }

    let cfg = match config::load_config(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            McpConfig::default()
        }
    };
    let cfg = Arc::new(cfg);

    iii_mcp_server::disable_trace_payloads();
    let iii = register_worker(
        &url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "mcp-server".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    // RBAC first: nothing is registered or served until the connection is
    // known to be behind rbac-proxy (or the escape hatch is set).
    let probe = Bus::call(
        iii.as_ref(),
        rbac_guard::SESSION_PROBE,
        serde_json::json!({}),
        Some(10_000),
    )
    .await
    .map_err(|e| e.to_string());
    let dangerous =
        rbac_guard::parse_dangerous(std::env::var(rbac_guard::DANGEROUS_ENV).ok().as_deref());
    let mode = match rbac_guard::decide(&rbac_guard::classify_probe(probe), dangerous, &url) {
        Decision::Refuse(message) => {
            eprintln!("{message}");
            iii.shutdown_async().await;
            std::process::exit(1);
        }
        Decision::Start {
            mode,
            warnings,
            notes,
        } => {
            for w in warnings {
                tracing::warn!("{w}");
                eprintln!("{w}");
            }
            for n in notes {
                tracing::info!("{n}");
            }
            mode
        }
    };
    tracing::info!(rbac = ?mode, "rbac check passed");

    // Then where the endpoint is reachable from: the http worker's bind.
    let status = Bus::call(
        iii.as_ref(),
        "http::status",
        serde_json::json!({}),
        Some(10_000),
    )
    .await
    .map_err(|e| e.to_string());
    let public = http_guard::parse_flag(std::env::var(http_guard::PUBLIC_HTTP_ENV).ok().as_deref());
    match http_guard::decide(status, public, &cfg.api_path) {
        http_guard::Bind::Refuse(message) => {
            eprintln!("{message}");
            iii.shutdown_async().await;
            std::process::exit(1);
        }
        http_guard::Bind::Serve { warnings } => {
            for w in warnings {
                tracing::warn!("{w}");
                eprintln!("{w}");
            }
        }
    }

    let sessions = Arc::new(Sessions::new(cfg.session_limits()));
    let ctx = functions::Ctx::new(
        SdkEngine {
            client: iii.clone(),
        },
        cfg.clone(),
        sessions.clone(),
        Arc::new(mode),
    );
    functions::register_all(&iii, &ctx);

    // The routes live as long as the connection; dropping the handles keeps
    // them. Report whether they became active without blocking startup.
    let routes = functions::observe_routes(&iii, &cfg.api_path);
    let api_path = cfg.api_path.clone();
    tokio::spawn(async move {
        for route in routes {
            match tokio::time::timeout(Duration::from_secs(30), route.ready()).await {
                Ok(Ok(())) => {
                    tracing::info!(api_path = %api_path, observer_id = %route.id(), "http::on-request route active")
                }
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, api_path = %api_path, "http::on-request route rejected")
                }
                Err(_) => {
                    tracing::warn!(api_path = %api_path, "http::on-request route still pending after 30s; is the http worker running?")
                }
            }
        }
    });

    // Idle sessions lose their observers; check every minute.
    // Idle-session sweep: once a minute, end the sessions idle longer than
    // session_idle_ttl_secs (clients do not always send DELETE).
    let sweep = sessions.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            let n = sweep.end_idle().await;
            if n > 0 {
                tracing::info!(sessions = n, "ended idle MCP sessions");
            }
        }
    });

    tracing::info!("mcp-server ready, waiting for invocations");
    tokio::signal::ctrl_c().await?;
    tracing::info!("mcp-server shutting down");
    sessions.close_all().await;
    iii.shutdown_async().await;
    Ok(())
}
