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
    about = "Denylist subscriber for agent::before_function_call"
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
    if let Ok(denied) = std::env::var("POLICY_DENIED_FUNCTIONS") {
        let denied_functions = parse_denied_functions(&denied);
        if !denied_functions.is_empty() {
            cfg.denied_functions = denied_functions;
        }
    } else if let Ok(denied) = std::env::var("POLICY_DENIED_TOOLS") {
        let denied_functions = parse_denied_functions(&denied);
        if !denied_functions.is_empty() {
            cfg.denied_functions = denied_functions;
        }
    }
}

fn parse_denied_functions(raw: &str) -> Vec<String> {
    let raw = raw.trim();
    // Brackets must be balanced. A single unmatched bracket previously
    // leaked into the first/last token (e.g. `[tool1,tool2` parsed as
    // `["[tool1", "tool2"]`), silently breaking the operator's intended
    // denial. Strip both, or strip neither.
    let stripped = match (raw.strip_prefix('['), raw.strip_suffix(']')) {
        (Some(inner), Some(_)) => inner.strip_suffix(']').unwrap_or(inner),
        (None, None) => raw,
        _ => {
            tracing::warn!(
                input = %raw,
                "POLICY_DENIED_FUNCTIONS has unmatched bracket; ignoring brackets and parsing as comma-separated"
            );
            raw.trim_matches(|c| c == '[' || c == ']')
        }
    };
    stripped
        .split(',')
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
        cfg.denied_functions.clone(),
        PolicyDenylistConfig {
            topic: cfg.topic.clone(),
        },
    )
    .map_err(|e| anyhow!("subscribe failed: {e}"))?;

    tracing::info!(
        topic = %cfg.topic,
        denied_functions = %cfg.denied_functions.join(","),
        "policy-denylist subscribed (policy::denylist)",
    );

    shutdown_signal().await;

    tracing::info!("policy-denylist shutting down");
    iii.shutdown_async().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_denied_functions;

    // ── Adversarial unit tests added per plan
    // /Users/ytallolayon/.claude/plans/let-s-implement-more-tests-refactored-flask.md

    #[test]
    fn parse_denied_functions_handles_empty_string() {
        assert!(parse_denied_functions("").is_empty());
    }

    #[test]
    fn parse_denied_functions_strips_whitespace_and_filters_empty() {
        assert_eq!(
            parse_denied_functions("  tool1  ,  ,  tool2  "),
            vec!["tool1".to_string(), "tool2".to_string()]
        );
    }

    #[test]
    fn parse_denied_functions_accepts_json_array_syntax() {
        // The Tier 2 demo.sh injects `bridge::trigger` as a single-value
        // env, but operators may still pass JSON-array form. This pins
        // the parser's tolerance for both quoting forms.
        assert_eq!(
            parse_denied_functions(r#"["tool1", "tool2"]"#),
            vec!["tool1".to_string(), "tool2".to_string()]
        );
        assert_eq!(
            parse_denied_functions("['tool1', 'tool2']"),
            vec!["tool1".to_string(), "tool2".to_string()]
        );
    }

    /// Malformed bracket inputs must not leak the bracket character into
    /// the parsed tool name. A leaked bracket would silently break the
    /// operator's intended denial: `bridge::trigger` would never match
    /// `[bridge::trigger`. The parser strips bracket pairs symmetrically
    /// and falls back to a bracket-tolerant strip + warning when one side
    /// is missing.
    #[test]
    fn parse_denied_functions_handles_malformed_unclosed_bracket() {
        assert_eq!(
            parse_denied_functions("[tool1,tool2"),
            vec!["tool1".to_string(), "tool2".to_string()],
            "open bracket without close must not leak '[' into the first token"
        );
        assert_eq!(
            parse_denied_functions("tool1,tool2]"),
            vec!["tool1".to_string(), "tool2".to_string()],
            "close bracket without open must not leak ']' into the last token"
        );
    }
}
