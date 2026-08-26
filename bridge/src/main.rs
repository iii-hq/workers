//! `iii-bridge` binary entry.
//!
//! Boot: parse CLI, connect to the LOCAL engine, register the `bridge`
//! configuration schema, fetch the authoritative value, start the bridge via
//! `boot::start`, wire the `configuration:updated` reload trigger, then sleep
//! until Ctrl+C. The optional `--config` YAML file is a **seed only**: it is
//! used to populate the configuration entry the first time, after which the
//! configuration worker is the source of truth. The REMOTE engine URL comes
//! from the configuration value. A legacy `III_URL` fallback remains when
//! that value is absent; supervised deployments must set `config.url` so the
//! remote target stays separate from the local/control `III_URL`.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};

use iii_bridge::config::BridgeConfig;
use iii_bridge::configuration;

#[derive(Parser, Debug)]
#[command(name = "bridge", about = "Bridge worker for iii.")]
struct Cli {
    /// Optional seed config.yaml. This seeds the configuration entry only when
    /// nothing is stored yet; thereafter the configuration worker is the
    /// authoritative source and `--config` is ignored for the stored value.
    #[arg(long)]
    config: Option<String>,
    /// Local/control engine URL used to register this worker.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    #[arg(long)]
    manifest: bool,
}

fn worker_metadata() -> WorkerMetadata {
    WorkerMetadata {
        runtime: "rust".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: "bridge".to_string(),
        os: std::env::consts::OS.to_string(),
        pid: Some(std::process::id()),
        telemetry: None,
        ..WorkerMetadata::default()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    use tracing_subscriber::prelude::*;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();
    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&iii_bridge::manifest::build_manifest()).unwrap()
        );
        return Ok(());
    }

    // Parse the optional --config seed (best-effort). It only seeds the
    // configuration entry on first boot; the authoritative value comes from the
    // configuration worker below.
    let seed: Option<BridgeConfig> = match cli.config.as_deref() {
        Some(path) => match std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_yaml::from_str::<BridgeConfig>(&s).map_err(|e| e.to_string()))
        {
            Ok(c) => {
                tracing::info!(path = %path, "loaded seed config");
                Some(c.normalized())
            }
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "failed to load --config seed; continuing without seed");
                None
            }
        },
        None => None,
    };

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(worker_metadata()),
            ..InitOptions::default()
        },
    ));

    // Register the schema (seeding initial_value only when nothing is stored),
    // then load the authoritative config the bridge actually connects with.
    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering bridge configuration schema")?;
    let config = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading bridge configuration")?;

    tracing::info!(remote = %config.effective_url(), "iii-bridge ready");
    let boot = iii_bridge::boot::start(iii.clone(), config).await?;

    // Subscribe to configuration:updated so the remote client, forward table,
    // and expose table reload live on a config change (see configuration).
    configuration::register_config_trigger(&iii, &boot)
        .await
        .map_err(anyhow::Error::msg)
        .context("binding configuration trigger")?;

    tokio::signal::ctrl_c().await?;
    tracing::info!("iii-bridge shutting down");
    boot.shutdown().await;
    iii.shutdown_async().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    const CHILD_MODE_ENV: &str = "BRIDGE_CLI_URL_TEST_CHILD";
    const EXPLICIT_URL_ENV: &str = "BRIDGE_CLI_URL_TEST_EXPLICIT";
    const OUTPUT_PREFIX: &str = "bridge-cli-url=";

    fn parse_url_in_subprocess(explicit_url: Option<&str>, env_url: Option<&str>) -> String {
        let mut command = Command::new(std::env::current_exe().expect("resolve test executable"));
        command
            .args(["--exact", "tests::print_cli_url_for_parent", "--nocapture"])
            .env(CHILD_MODE_ENV, "1")
            .env_remove(EXPLICIT_URL_ENV)
            .env_remove("III_URL");

        if let Some(url) = explicit_url {
            command.env(EXPLICIT_URL_ENV, url);
        }
        if let Some(url) = env_url {
            command.env("III_URL", url);
        }

        let output = command.output().expect("run CLI parser subprocess");
        assert!(
            output.status.success(),
            "CLI parser subprocess failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("CLI parser output is UTF-8")
            .lines()
            .find_map(|line| line.strip_prefix(OUTPUT_PREFIX))
            .map(str::to_owned)
            .expect("CLI parser subprocess printed the selected URL")
    }

    #[test]
    fn explicit_url_overrides_iii_url() {
        let url =
            parse_url_in_subprocess(Some("ws://127.0.0.1:49311"), Some("ws://127.0.0.1:49312"));

        assert_eq!(url, "ws://127.0.0.1:49311");
    }

    #[test]
    fn iii_url_is_used_without_explicit_url() {
        let url = parse_url_in_subprocess(None, Some("ws://127.0.0.1:49312"));

        assert_eq!(url, "ws://127.0.0.1:49312");
    }

    #[test]
    fn default_url_is_used_without_overrides() {
        let url = parse_url_in_subprocess(None, None);

        assert_eq!(url, "ws://127.0.0.1:49134");
    }

    #[test]
    fn print_cli_url_for_parent() {
        if std::env::var(CHILD_MODE_ENV).ok().as_deref() != Some("1") {
            return;
        }

        let mut args = vec!["bridge".to_string()];
        if let Ok(url) = std::env::var(EXPLICIT_URL_ENV) {
            args.extend(["--url".to_string(), url]);
        }
        let cli = Cli::parse_from(args);

        println!("{OUTPUT_PREFIX}{}", cli.url);
    }
}
