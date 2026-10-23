use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};

use tmux::collector::{Collector, ConsoleClients};
use tmux::config::WorkerConfig;
use tmux::events::{self, Monitor};
use tmux::functions::{self, Ctx};
use tmux::terminal::{self, TerminalManager};
use tmux::{binary, configuration, manifest, ui};

const ABOUT: &str = "tmux as an iii worker: sessions and windows, agent / compose / port status per pane, and a Console page with a live tmux terminal.";

#[derive(Parser, Debug)]
#[command(name = "tmux", about = ABOUT)]
struct Cli {
    /// YAML seed applied only when the `tmux` configuration entry is first created.
    #[arg(long)]
    config: Option<String>,
    /// iii engine WebSocket address.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    /// Print the registry manifest as JSON and exit without connecting.
    #[arg(long)]
    manifest: bool,
}

async fn wait_for_shutdown_signal() -> Result<()> {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        _ = sigterm.recv() => {}
    }
    Ok(())
}

fn load_seed(path: &str) -> Option<WorkerConfig> {
    let contents = std::fs::read_to_string(path)
        .map_err(|error| tracing::warn!(path, %error, "ignoring unreadable seed config"))
        .ok()?;
    let value: serde_json::Value = serde_yaml::from_str(&contents)
        .map_err(|error| tracing::warn!(path, %error, "ignoring invalid seed YAML"))
        .ok()?;
    WorkerConfig::from_json(&value)
        .map_err(|error| tracing::warn!(path, %error, "ignoring invalid seed config"))
        .ok()
}

#[tokio::main]
async fn main() -> Result<()> {
    // This binary shares its name with tmux. If tmux resolution ever probes
    // a copy of it, answer like a program that is not tmux and stop there.
    if std::env::var_os(binary::PROBE_ENV).is_some() {
        std::process::exit(2);
    }

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
            serde_json::to_string_pretty(&manifest::build_manifest())?
        );
        return Ok(());
    }

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "tmux".to_string(),
                os: std::env::consts::OS.to_string(),
                description: Some(ABOUT.to_string()),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    let seed = cli.config.as_deref().and_then(load_seed);
    configuration::register(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering tmux configuration")?;
    let config = match configuration::fetch(&iii).await {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(%error, "stored tmux configuration unusable; starting with defaults");
            WorkerConfig::default()
        }
    }
    .into_shared();
    iii_console_ui::register_configuration_identity(&iii, "tmux", configuration::config_id());

    let console = ConsoleClients::default();
    let collector = Arc::new(Collector::new(config.clone(), console.clone()));
    let monitor = Monitor::new((*iii).clone(), collector.clone());
    let poke = {
        let monitor = monitor.clone();
        Arc::new(move || monitor.poke()) as Arc<dyn Fn() + Send + Sync>
    };
    let engine_observes = {
        let iii = iii.clone();
        Arc::new(move || iii.engine_supports_observe())
            as Arc<dyn Fn() -> Option<bool> + Send + Sync>
    };
    let terminals = TerminalManager::new(
        Arc::new((*iii).clone()),
        collector.clone(),
        console,
        poke,
        engine_observes,
    );

    functions::register_all(
        &iii,
        Arc::new(Ctx {
            collector: collector.clone(),
            monitor: monitor.clone(),
            transcripts: Default::default(),
        }),
    );
    terminal::register(&iii, terminals.clone());
    events::register_changed_trigger(&iii, monitor.clone());
    let reload =
        configuration::bind_reload(&iii, config).context("binding tmux configuration reload")?;
    reload.run().await;
    ui::register(&iii);

    monitor.spawn();
    terminals.spawn_reaper();

    let info = collector.info().await;
    match (&info.path, &info.version) {
        (Some(path), Some(version)) => {
            tracing::info!(%path, %version, server_running = info.server_running, "tmux worker ready")
        }
        _ => tracing::warn!(error = ?info.error, "tmux worker ready, but tmux is unavailable"),
    }

    wait_for_shutdown_signal().await?;
    let detached = terminals.close_all().await;
    tracing::info!(detached, "tmux worker shutting down");
    iii.shutdown_async().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::Cli;

    #[test]
    fn url_accepts_the_release_engine_environment() {
        let command = Cli::command();
        let url = command
            .get_arguments()
            .find(|argument| argument.get_id() == "url")
            .expect("url argument exists");
        assert_eq!(url.get_env(), Some(std::ffi::OsStr::new("III_URL")));
    }

    #[test]
    fn there_is_no_version_flag_that_could_pass_for_tmux() {
        // `tmux -V` is how the real tmux is recognised; this binary must not answer it.
        assert!(Cli::try_parse_from(["tmux", "-V"]).is_err());
    }
}
