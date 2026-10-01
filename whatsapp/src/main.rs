use std::io::Read;
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use clap::Parser;
use iii_sdk::{InitOptions, register_worker, runtime::WorkerMetadata};
use serde_json::json;
use whatsapp::{
    config::{self, Config},
    configuration, service,
    sessions::SessionManager,
    ui,
};

#[derive(Parser)]
#[command(name = "whatsapp", version, about)]
struct Cli {
    /// Optional YAML seed; existing central configuration remains authoritative.
    #[arg(long)]
    config: Option<std::path::PathBuf>,
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    /// Use only local configuration (isolated development / interface capture).
    #[arg(long)]
    local_config: bool,
    /// Print package information without connections or storage access.
    #[arg(long, conflicts_with = "catalog")]
    manifest: bool,
    /// Print function schemas without connections or storage access.
    #[arg(long)]
    catalog: bool,
    /// Import a saved events::poll page into an existing session's local inbox, offline.
    #[arg(long, requires_all = ["local_config", "import_session"], conflicts_with_all = ["manifest", "catalog"])]
    import_events: Option<std::path::PathBuf>,
    /// Existing session receiving the offline event import.
    #[arg(long, requires = "import_events")]
    import_session: Option<String>,
}

fn main() -> Result<()> {
    // The SDK records function inputs/outputs in telemetry by default, including
    // pairing codes. Disable payload capture before creating any worker threads;
    // keep timing/error spans available without account or message contents.
    // SAFETY: this is the process entrypoint, before Tokio, logging, or the SDK
    // starts threads that could read or write the process environment.
    unsafe { std::env::set_var("III_DISABLE_TRACE_PAYLOADS", "1") };
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run_cli())
}

async fn run_cli() -> Result<()> {
    whatsapp::logging::init();
    let cli = Cli::parse();
    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "name": "whatsapp", "version": env!("CARGO_PKG_VERSION"),
                "description": env!("CARGO_PKG_DESCRIPTION"), "default_config": Config::default(),
                "native_client": {"repository":"https://github.com/oxidezap/whatsapp-rust", "revision":"dd0763c4635b353c4ed52def39d030fb2f375f9a"},
                "runtime": "native binary; no Node, browser or sandbox required",
                "trigger_types": [service::EVENTS],
                "console_ui": {"scope":"whatsapp", "assets":[ui::PAGE_PATH, ui::STYLES_PATH]}
            }))?
        );
        return Ok(());
    }
    if cli.catalog {
        println!(
            "{}",
            serde_json::to_string_pretty(&whatsapp::registry().catalog())?
        );
        return Ok(());
    }
    let seed_result = cli
        .config
        .as_ref()
        .map(|path| -> Result<Config> {
            Config::from_yaml(&std::fs::read_to_string(path).context("reading config seed")?)
        })
        .transpose();
    let seed = match seed_result {
        Ok(seed) => seed,
        Err(error) if cli.local_config => return Err(error),
        Err(_) => {
            tracing::warn!(
                "Unable to read the optional configuration seed; using central configuration or defaults"
            );
            None
        }
    };
    if let Some(path) = cli.import_events {
        const MAX_IMPORT_BYTES: u64 = 64 * 1024 * 1024;
        let mut input = Vec::new();
        std::fs::File::open(path)
            .context("opening event import")?
            .take(MAX_IMPORT_BYTES + 1)
            .read_to_end(&mut input)
            .context("reading event import")?;
        ensure!(
            input.len() as u64 <= MAX_IMPORT_BYTES,
            "event import exceeds 64 MiB"
        );
        let page: serde_json::Value =
            serde_json::from_slice(&input).context("invalid event import JSON")?;
        let session_id = cli.import_session.context("missing import session")?;
        let imported =
            whatsapp::sessions::import_events(&seed.unwrap_or_default(), &session_id, &page)?;
        println!("{}", json!({"imported": imported}));
        return Ok(());
    }
    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                name: "whatsapp".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    let result = run(cli.local_config, seed, iii.clone(), cli.url).await;
    tokio::task::spawn_blocking(move || iii.shutdown()).await?;
    result
}

async fn run(
    local: bool,
    seed: Option<Config>,
    iii: Arc<iii_sdk::IIIClient>,
    engine_url: String,
) -> Result<()> {
    let config: Config = if local {
        seed.unwrap_or_default()
    } else {
        let spec = iii_config_client::EntrySpec {
            id: config::config_id(),
            form_id: config::DEFAULT_CONFIG_ID,
            name: "WhatsApp",
            description: "WhatsApp linked devices, official Cloud API, storage and operating limits",
            schema: Config::json_schema(),
            default_value: serde_json::to_value(Config::default())?,
        };
        iii_config_client::ensure(&iii, &spec, seed.map(serde_json::to_value).transpose()?)
            .await
            .map_err(anyhow::Error::msg)?;
        Config::from_json(
            &iii_config_client::fetch(&iii, spec.id)
                .await
                .map_err(anyhow::Error::msg)?
                .context("missing whatsapp configuration")?,
        )?
    };
    config.validate()?;
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let manager = SessionManager::open(config)?;
    Arc::new(whatsapp::registry()).install(&iii, manager.clone());
    let delivery = service::register(iii.clone(), manager.clone());
    let cloud_webhooks = whatsapp::cloud::webhook::register(&iii, manager.clone(), engine_url)?;
    iii_console_ui::register_configuration_identity(&iii, "whatsapp", config::config_id());
    ui::register(&iii);
    configuration::register(&iii, &manager, local).await?;
    tracing::info!("WhatsApp functions registered; sessions start only on explicit request");
    #[cfg(unix)]
    let signal = tokio::select! { result = tokio::signal::ctrl_c() => result, _ = terminate.recv() => Ok(()) };
    #[cfg(not(unix))]
    let signal = tokio::signal::ctrl_c().await;
    for route in cloud_webhooks {
        route.unregister();
    }
    manager.shutdown().await;
    delivery.abort();
    let _ = delivery.await;
    signal.context("waiting for shutdown signal")
}
