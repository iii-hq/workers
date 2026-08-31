use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::errors::Error;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions, RegisterFunction, RegisterTriggerType};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use storage::config::{redact_url, WorkerConfig};
use storage::configuration;
use storage::handlers::{
    delete_object, get_object, head_object, list_buckets, list_objects, presign_post, presign_url,
    put_object, AppState,
};
use storage::triggers::dispatcher::EngineDispatcher;
use storage::triggers::handler::{ObjectCreatedHandler, ObjectDeletedHandler};
use storage::triggers::registry::TriggerRegistry;
use tokio::sync::RwLock;

#[derive(Parser, Debug)]
#[command(name = "storage", about = "storage worker")]
struct Cli {
    /// Optional seed config.yaml used to populate `initial_value` on first register.
    #[arg(long)]
    config: Option<String>,
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    /// Print the registry publish manifest as JSON and exit. No engine connection.
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
        let m = storage::manifest::build_manifest();
        println!(
            "{}",
            serde_json::to_string_pretty(&m).expect("manifest serializes")
        );
        return Ok(());
    }

    tracing::info!(
        name = storage::worker_name(),
        seed_config = cli.config.as_deref().unwrap_or("(none)"),
        url = %redact_url(&cli.url),
        "starting"
    );

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: storage::worker_name().to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..Default::default()
        },
    ));

    let seed = match &cli.config {
        Some(path) => match WorkerConfig::from_file(path) {
            Ok(cfg) => {
                tracing::info!(path = %path, "loaded seed config for initial registration");
                Some(cfg)
            }
            Err(e) => {
                tracing::warn!(
                    path = %path,
                    error = %e,
                    "failed to load seed config; relying on existing configuration entry"
                );
                None
            }
        },
        None => None,
    };

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering storage configuration schema")?;

    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading storage configuration")?;

    let registry = Arc::new(TriggerRegistry::new());
    let dispatcher = Arc::new(EngineDispatcher::new(
        iii.as_ref().clone(),
        registry.clone(),
    ));

    let state = AppState::new(HashMap::new());
    let wired_buckets = Arc::new(RwLock::new(HashSet::new()));
    let runtime =
        configuration::StorageRuntime::new(dispatcher.clone(), &state, wired_buckets.clone());
    runtime
        .apply_config(&state, cfg)
        .await
        .map_err(anyhow::Error::msg)
        .context("applying initial storage configuration")?;

    // Register the storage::* RPC functions inline.
    {
        let st = state.clone();
        iii.register_function(
            "storage::putObject",
            RegisterFunction::new_async(move |req: put_object::PutReq| {
                let st = st.clone();
                async move { put_object::handle(&st, req).await.map_err(Error::from) }
            })
            .description(
                "Write a small object inline as base64 (10 MiB hard limit). This buffers and inflates the payload; use presignPost or a presignUrl PUT for files and large objects.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "storage::getObject",
            RegisterFunction::new_async(move |req: get_object::GetReq| {
                let st = st.clone();
                async move { get_object::handle(&st, req).await.map_err(Error::from) }
            })
            .description("Read a small object inline as base64 (10 MiB hard limit). This buffers and inflates the payload; use a GET URL from presignUrl for files and large objects."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "storage::presignPost",
            RegisterFunction::new_async(move |req: presign_post::PresignPostReq| {
                let st = st.clone();
                async move { presign_post::handle(&st, req).await.map_err(Error::from) }
            })
            .description(
                "Issue a short-lived multipart/form-data POST for direct browser upload without buffering the file in the engine or worker RPC path.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "storage::deleteObject",
            RegisterFunction::new_async(move |req: delete_object::DeleteReq| {
                let st = st.clone();
                async move { delete_object::handle(&st, req).await.map_err(Error::from) }
            })
            .description("Delete an object. No-op when the object does not exist."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "storage::listBuckets",
            RegisterFunction::new_async(move |req: list_buckets::ListBucketsReq| {
                let st = st.clone();
                async move { list_buckets::handle(&st, req).await.map_err(Error::from) }
            })
            .description("List configured worker-facing storage buckets and their providers."),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "storage::listObjects",
            RegisterFunction::new_async(move |req: list_objects::ListObjectsReq| {
                let st = st.clone();
                async move { list_objects::handle(&st, req).await.map_err(Error::from) }
            })
            .description(
                "List objects and common prefixes in a bucket with provider-native pagination.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "storage::presignUrl",
            RegisterFunction::new_async(move |req: presign_url::PresignReq| {
                let st = st.clone();
                async move { presign_url::handle(&st, req).await.map_err(Error::from) }
            })
            .description(
                "Issue a short-lived URL the browser can hit directly to PUT or GET an object.",
            ),
        );
    }
    {
        let st = state.clone();
        iii.register_function(
            "storage::headObject",
            RegisterFunction::new_async(move |req: head_object::HeadReq| {
                let st = st.clone();
                async move { head_object::handle(&st, req).await.map_err(Error::from) }
            })
            .description(
                "Fetch object metadata (size, ETag, content-type, last-modified) without downloading the body.",
            ),
        );
    }

    let _ = iii.register_trigger_type(RegisterTriggerType::new(
        "storage::object-created",
        "Fires when an object is written to a configured bucket whose notifications source is wired up.",
        ObjectCreatedHandler {
            registry: registry.clone(),
            wired_buckets: wired_buckets.clone(),
            reconfigure_gate: state.reconfigure_gate.clone(),
        },
    ));
    let _ = iii.register_trigger_type(RegisterTriggerType::new(
        "storage::object-deleted",
        "Fires when an object is removed from a configured bucket whose notifications source is wired up.",
        ObjectDeletedHandler {
            registry: registry.clone(),
            wired_buckets: wired_buckets.clone(),
            reconfigure_gate: state.reconfigure_gate.clone(),
        },
    ));

    configuration::register_config_trigger(&iii, state.clone(), runtime.clone())
        .context("registering configuration change trigger")?;

    storage::ui::register(&iii);

    tracing::info!("storage registered 8 functions and 2 trigger types, waiting for invocations");
    wait_for_shutdown_signal().await?;
    tracing::info!("storage shutting down");
    runtime.shutdown().await;
    iii.shutdown_async().await;
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
