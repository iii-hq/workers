use anyhow::Result;
use clap::Parser;
use editor::{bus::Bus, config, configuration, events, functions, manifest, observe, ui};
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(
    name = "editor",
    about = "Editor surface for code — diffs, git marks, fuzzy find, conflict-safe saves"
)]
struct Cli {
    /// Optional one-time seed for the configuration worker on first
    /// registration. The configuration worker is the source of truth after
    /// that, so this never overwrites a stored value.
    #[arg(long)]
    config: Option<String>,

    /// Engine WebSocket. `III_URL` is what the worker manager injects when the
    /// worker runs managed — inside a sandbox the engine is on the VM's
    /// gateway, never on the VM's own loopback, so the default is only right
    /// for a worker started by hand on the host.
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

    // A seed that will not parse is a warning, not a failure: the stored
    // value (or the built-in default) still applies.
    let seed = cli
        .config
        .as_deref()
        .and_then(|path| match config::WorkerConfig::from_file(path) {
            Ok(cfg) => Some(cfg),
            Err(e) => {
                tracing::warn!(error = %e, path, "failed to read config seed; ignoring it");
                None
            }
        });

    let iii = register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "editor".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    // The configuration worker is where the authoritative limits live, and a
    // loaded engine can answer its registration late. Dying on that timeout is
    // worse than waiting: the source watcher restarts the worker straight back
    // into the same race, so a slow boot becomes a crash loop that never
    // converges. So: retry.
    //
    // And if the retries run out, still do not exit. Serve the seed — the
    // operator's own numbers, from `--config` — and keep asking for the
    // authoritative value in the background, because a worker that is up on
    // known limits beats one that is not up at all. What is never done is
    // guessing: the log says exactly which numbers are in force, and the
    // built-in defaults are used only when nothing was seeded either.
    const CONFIG_BOOT_ATTEMPTS: u32 = 5;
    let mut loaded = None;
    let mut last_err = None;
    for attempt in 1..=CONFIG_BOOT_ATTEMPTS {
        match configuration::register_config(&iii, seed.as_ref()).await {
            Ok(()) => match configuration::fetch_config(&iii).await {
                Ok(cfg) => {
                    loaded = Some(cfg);
                    break;
                }
                Err(e) => last_err = Some(format!("loading editor configuration: {e}")),
            },
            Err(e) => last_err = Some(format!("registering editor configuration: {e}")),
        }
        if attempt < CONFIG_BOOT_ATTEMPTS {
            let backoff = std::time::Duration::from_millis(400 * u64::from(attempt));
            tracing::warn!(
                attempt,
                backoff_ms = backoff.as_millis() as u64,
                error = last_err.as_deref().unwrap_or("unknown"),
                "editor configuration not ready; retrying"
            );
            tokio::time::sleep(backoff).await;
        }
    }
    let fell_back = loaded.is_none();
    let cfg = match loaded {
        Some(cfg) => cfg,
        None => {
            let (cfg, source) = configuration::fallback_config(seed.as_ref());
            tracing::warn!(
                attempts = CONFIG_BOOT_ATTEMPTS,
                error = last_err.as_deref().unwrap_or("unknown"),
                "the configuration worker never answered, so the editor is serving {source}; \
                 these limits are a stopgap and may be stale until it does"
            );
            cfg
        }
    };
    let git_timeout_ms = Arc::new(std::sync::atomic::AtomicU64::new(cfg.git_timeout_ms));
    let cfg = configuration::cell(cfg);

    // A fallback has to be temporary. The hot-reload trigger below only fires
    // once the configuration worker holds an `editor` entry, and the likeliest
    // reason the boot failed is that it was not up to take the registration —
    // in which case no reload event can ever arrive. This keeps asking until it
    // can, then publishes through the same path a reload would.
    if fell_back {
        configuration::spawn_boot_recovery(
            iii.clone(),
            cfg.clone(),
            git_timeout_ms.clone(),
            seed.clone(),
        );
    }

    // The bus carries the engine URL because `shell::fs::read` answers with a
    // channel reference that has to be dialled separately.
    let bus = Arc::new(Bus::new(
        iii.clone(),
        cli.url.clone(),
        git_timeout_ms.clone(),
    ));

    // Custom trigger types go up before the functions that emit on them, so a
    // handler can never fire against a half-built subscriber set.
    let changed = events::register_changed_trigger(&iii);

    functions::register_all(&iii, &cfg, &bus);
    ui::register(&iii);

    // The observer is what makes the workspace see edits made by anything, not
    // just by callers of this worker.
    observe::bind(&iii, &cfg, &bus, changed);

    // Bound last, so the handler closes over fully-built state. The bind retries
    // internally, for the same reason the boot handshake does; a bind that never
    // succeeds is fatal rather than a warning, because a worker whose limits can
    // never be changed is worse than one that did not start.
    configuration::register_config_trigger(&iii, cfg.clone(), git_timeout_ms)
        .await
        .map_err(|e| anyhow::anyhow!("binding the configuration trigger: {e}"))?;

    tracing::info!("editor ready, waiting for invocations");
    tokio::signal::ctrl_c().await?;
    tracing::info!("editor shutting down");
    iii.shutdown_async().await;
    Ok(())
}
