//! `provider-openai` binary entry.
//!
//! The worker keeps no operator settings of its own: credentials, `api_url`,
//! and `max_tokens` arrive per request from llm-router's resolve step.
//! `--config` is still accepted per the binary-worker CLI contract (the
//! engine passes it when an operator sets a config block); keys found there
//! are warned about instead of silently dropped.

use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use provider_openai::register::register_provider;

#[derive(Parser, Debug)]
#[command(
    name = "provider-openai",
    about = "OpenAI Responses provider worker behind llm-router."
)]
struct Cli {
    /// Accepted for the standard worker CLI contract; provider config comes
    /// from llm-router's resolve step, not from a file.
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, env = "III_WS_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,

    #[arg(long)]
    manifest: bool,
}

/// True when the YAML contents carry anything beyond comments, blank lines,
/// or a bare empty mapping (`{}`).
fn has_config_keys(contents: &str) -> bool {
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .any(|line| line != "{}")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // Registry publish pipeline: print the manifest JSON and exit.
    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&provider_openai::manifest::build_manifest())?
        );
        return Ok(());
    }

    if let Ok(contents) = std::fs::read_to_string(&cli.config) {
        if has_config_keys(&contents) {
            tracing::warn!(
                path = %cli.config,
                "provider-openai takes no file-based config; configure the provider \
                 via the engine's `llm-router` configuration entry — ignoring this file's keys"
            );
        }
    }

    let iii = register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "provider-openai".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );

    register_provider(iii.clone()).await?;
    tracing::info!(url = %cli.url, "provider-openai registered");

    tokio::signal::ctrl_c().await?;
    iii.shutdown_async().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::has_config_keys;

    #[test]
    fn empty_and_comment_only_contents_have_no_keys() {
        assert!(!has_config_keys(""));
        assert!(!has_config_keys("\n\n"));
        assert!(!has_config_keys("# only a comment\n  # indented comment\n"));
        assert!(!has_config_keys("{}\n"));
        assert!(!has_config_keys("# comment\n{}\n"));
    }

    #[test]
    fn real_keys_are_detected() {
        assert!(has_config_keys("api_url: https://example.com\n"));
        assert!(has_config_keys("# comment\nmax_tokens: 8192\n"));
    }
}
