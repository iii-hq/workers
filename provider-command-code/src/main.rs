use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use provider_command_code::register::register_provider;

#[derive(Parser, Debug)]
#[command(
    name = "provider-command-code",
    about = "Command Code dual-protocol provider worker behind llm-router."
)]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    #[arg(long)]
    manifest: bool,
}

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
    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&provider_command_code::manifest::build_manifest())?
        );
        return Ok(());
    }
    if let Ok(contents) = std::fs::read_to_string(&cli.config) {
        if has_config_keys(&contents) {
            tracing::warn!(
                path = %cli.config,
                "provider-command-code takes no file-based config; configure its llm-router provider slice instead"
            );
        }
    }
    let iii = register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "provider-command-code".into(),
                os: std::env::consts::OS.into(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    register_provider(iii.clone()).await?;
    tracing::info!(url = %cli.url, "provider-command-code registered");
    tokio::signal::ctrl_c().await?;
    iii.shutdown_async().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::has_config_keys;

    #[test]
    fn only_real_yaml_keys_trigger_the_warning() {
        assert!(!has_config_keys("# comments only\n{}\n"));
        assert!(has_config_keys("api_url: https://example.test\n"));
    }
}
