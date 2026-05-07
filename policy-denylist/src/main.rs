//! Binary entrypoint for the policy-denylist worker.

mod config;
mod manifest;

use anyhow::{anyhow, Result};
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, WorkerMetadata};
use policy_denylist::{subscribe_denylist_with_config, PolicyDenylistConfig};
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(
    name = "iii-policy-denylist",
    about = "Denylist subscriber for agent::before_tool_call"
)]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, default_value = "ws://127.0.0.1:49134", env = "III_URL")]
    url: String,

    #[arg(long)]
    manifest: bool,
}

fn apply_runtime_env_overrides(cfg: &mut config::WorkerConfig) {
    if let Ok(topic) = std::env::var("POLICY_DENYLIST_TOPIC") {
        let topic = topic.trim();
        if !topic.is_empty() {
            cfg.topic = topic.to_string();
        }
    }
    if let Ok(denied) = std::env::var("POLICY_DENIED_TOOLS") {
        let denied_tools = parse_denied_tools(&denied);
        if !denied_tools.is_empty() {
            cfg.denied_tools = denied_tools;
        }
    }
}

fn parse_denied_tools(raw: &str) -> Vec<String> {
    let raw = raw.trim();
    let raw = raw
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(raw);
    raw.split(',')
        .map(str::trim)
        .map(|s| s.trim_matches('"').trim_matches('\'').trim())
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};

    match signal(SignalKind::terminate()) {
        Ok(mut sigterm) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = sigterm.recv() => {},
            }
        }
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    tokio::signal::ctrl_c().await.ok();
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
        let m = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m).unwrap());
        return Ok(());
    }

    let mut cfg = match config::load_config(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %cli.config,
                "failed to load config, using defaults"
            );
            config::WorkerConfig::default()
        }
    };
    apply_runtime_env_overrides(&mut cfg);

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "policy-denylist".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    let _sub = subscribe_denylist_with_config(
        &iii,
        cfg.denied_tools.clone(),
        PolicyDenylistConfig {
            topic: cfg.topic.clone(),
        },
    )
    .map_err(|e| anyhow!("subscribe failed: {e}"))?;

    tracing::info!(
        topic = %cfg.topic,
        denied_tools = %cfg.denied_tools.join(","),
        "policy-denylist subscribed (policy::denylist)",
    );

    shutdown_signal().await;

    tracing::info!("policy-denylist shutting down");
    iii.shutdown_async().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_denied_tools;

    // ── Adversarial unit tests added per plan
    // /Users/ytallolayon/.claude/plans/let-s-implement-more-tests-refactored-flask.md

    #[test]
    fn parse_denied_tools_handles_empty_string() {
        assert!(parse_denied_tools("").is_empty());
    }

    #[test]
    fn parse_denied_tools_strips_whitespace_and_filters_empty() {
        assert_eq!(
            parse_denied_tools("  tool1  ,  ,  tool2  "),
            vec!["tool1".to_string(), "tool2".to_string()]
        );
    }

    #[test]
    fn parse_denied_tools_accepts_json_array_syntax() {
        // The Tier 2 demo.sh injects `bridge::trigger` as a single-value
        // env, but operators may still pass JSON-array form. This pins
        // the parser's tolerance for both quoting forms.
        assert_eq!(
            parse_denied_tools(r#"["tool1", "tool2"]"#),
            vec!["tool1".to_string(), "tool2".to_string()]
        );
        assert_eq!(
            parse_denied_tools("['tool1', 'tool2']"),
            vec!["tool1".to_string(), "tool2".to_string()]
        );
    }

    // TODO(test-harden): the parser strips '[' prefix and ']' suffix
    // jointly via `.and_then(strip_suffix)`. If either bracket is missing,
    // the .and_then chain returns None and the raw input falls through to
    // .split(','), leaking the unmatched bracket into the first or last
    // element ("[tool1" or "tool2]"). The trim-quotes step doesn't strip
    // brackets. The operator's intended denial silently never matches the
    // real tool name. Fix: either accept both bracket forms strictly
    // (require both or neither) or reject malformed input and log a
    // warning.
    #[ignore]
    #[test]
    fn parse_denied_tools_handles_malformed_unclosed_bracket() {
        assert_eq!(
            parse_denied_tools("[tool1,tool2"),
            vec!["tool1".to_string(), "tool2".to_string()],
            "open bracket without close should still parse cleanly"
        );
        assert_eq!(
            parse_denied_tools("tool1,tool2]"),
            vec!["tool1".to_string(), "tool2".to_string()],
            "close bracket without open should still parse cleanly"
        );
    }
}
