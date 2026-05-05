use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use context_compaction::{CompactionDefaults, CompactionError, SummariseFn};
use harness_types::AgentMessage;
use iii_sdk::{register_worker, InitOptions};
use serde_json::Value;

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

#[derive(Debug, Default, PartialEq)]
struct ConfigOverrides {
    threshold_pct: Option<f32>,
    fallback_context_window: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct RuntimeConfig {
    threshold_pct: f32,
    fallback_context_window: u64,
}

impl RuntimeConfig {
    fn load(config_path: Option<&Path>) -> Self {
        let defaults = CompactionDefaults::default();
        let mut cfg = Self {
            threshold_pct: defaults.threshold_pct,
            fallback_context_window: defaults.fallback_context_window,
        };

        if let Some(path) = config_path {
            match load_config_overrides(path) {
                Ok(overrides) => cfg.apply(overrides),
                Err(e) => log::warn!(
                    "failed to load context-compaction config from {}: {e}; using defaults",
                    path.display()
                ),
            }
        }

        if let Ok(threshold_pct) = std::env::var("CONTEXT_COMPACTION_THRESHOLD_PCT") {
            if let Ok(threshold_pct) = threshold_pct.trim().parse() {
                cfg.threshold_pct = threshold_pct;
            }
        }
        if let Ok(context_window) = std::env::var("CONTEXT_COMPACTION_FALLBACK_CONTEXT_WINDOW") {
            if let Ok(context_window) = context_window.trim().parse() {
                cfg.fallback_context_window = context_window;
            }
        }

        cfg
    }

    fn apply(&mut self, overrides: ConfigOverrides) {
        if let Some(threshold_pct) = overrides.threshold_pct {
            self.threshold_pct = threshold_pct;
        }
        if let Some(fallback_context_window) = overrides.fallback_context_window {
            self.fallback_context_window = fallback_context_window;
        }
    }

    fn compaction_defaults(self) -> CompactionDefaults {
        CompactionDefaults {
            threshold_pct: self.threshold_pct,
            fallback_context_window: self.fallback_context_window,
        }
    }
}

fn config_path_arg() -> Option<PathBuf> {
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == std::ffi::OsStr::new("--config") {
            return args.next().map(PathBuf::from);
        }
        if let Some(s) = arg.to_str() {
            if let Some(path) = s.strip_prefix("--config=") {
                return Some(PathBuf::from(path));
            }
        }
    }
    None
}

fn load_config_overrides(path: &Path) -> Result<ConfigOverrides> {
    let raw = std::fs::read_to_string(path)?;
    Ok(parse_config_overrides(&raw))
}

fn parse_config_overrides(raw: &str) -> ConfigOverrides {
    if let Some(overrides) = parse_json_config_overrides(raw) {
        return overrides;
    }
    parse_yaml_config_overrides(raw)
}

fn parse_json_config_overrides(raw: &str) -> Option<ConfigOverrides> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let root = value.as_object()?;
    let object = root
        .get("config")
        .and_then(Value::as_object)
        .unwrap_or(root);
    let threshold_pct = object
        .get("threshold_pct")
        .and_then(Value::as_f64)
        .map(|v| v as f32);
    let fallback_context_window = object
        .get("fallback_context_window")
        .or_else(|| object.get("context_window"))
        .and_then(Value::as_u64);
    Some(ConfigOverrides {
        threshold_pct,
        fallback_context_window,
    })
}

fn parse_yaml_config_overrides(raw: &str) -> ConfigOverrides {
    let mut overrides = ConfigOverrides::default();
    let mut in_config_block = false;
    let mut config_indent = None;

    for line in raw.lines() {
        let indent = line.chars().take_while(|ch| *ch == ' ').count();
        let line = strip_yaml_comment(line).trim_end();
        let line = line.trim_start();
        if line.is_empty() || line == "---" {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };

        if indent == 0 {
            let key = key.trim();
            if key == "config" && parse_yaml_scalar(value).is_none() {
                in_config_block = true;
                config_indent = None;
                continue;
            }
            in_config_block = false;
            parse_config_key(&mut overrides, key, value);
            continue;
        }

        if !in_config_block {
            continue;
        }

        let expected_indent = *config_indent.get_or_insert(indent);
        if indent != expected_indent {
            continue;
        }
        parse_config_key(&mut overrides, key.trim(), value);
    }

    overrides
}

fn parse_config_key(overrides: &mut ConfigOverrides, key: &str, value: &str) {
    match key {
        "threshold_pct" => {
            overrides.threshold_pct = parse_yaml_scalar(value).and_then(|v| v.parse().ok());
        }
        "fallback_context_window" | "context_window" => {
            overrides.fallback_context_window =
                parse_yaml_scalar(value).and_then(|v| v.parse().ok());
        }
        _ => {}
    }
}

fn strip_yaml_comment(line: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;

    for (idx, ch) in line.char_indices() {
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '#' if !in_single && !in_double => return &line[..idx],
            _ => {}
        }
    }

    line
}

fn parse_yaml_scalar(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value == "null" || value == "~" {
        return None;
    }

    let unquoted = if value.len() >= 2 {
        let bytes = value.as_bytes();
        let quote = bytes[0];
        if (quote == b'\'' || quote == b'"') && bytes[value.len() - 1] == quote {
            &value[1..value.len() - 1]
        } else {
            value
        }
    } else {
        value
    };

    let unquoted = unquoted.trim();
    if unquoted.is_empty() {
        None
    } else {
        Some(unquoted.to_string())
    }
}

/// Default summariser: produces a placeholder so the proactive compactor
/// can run without a model wired up. Production deployments swap in an
/// LLM-backed summariser at startup.
struct NoopSummariser;

#[async_trait]
impl SummariseFn for NoopSummariser {
    async fn summarise(
        &self,
        _messages: Vec<AgentMessage>,
        _instructions: Option<String>,
    ) -> Result<String, CompactionError> {
        Ok("(compacted; no summariser configured)".into())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let engine_url = std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
    let cfg = RuntimeConfig::load(config_path_arg().as_deref());
    let iii = register_worker(&engine_url, InitOptions::default());

    let _handles = context_compaction::register_with_iii_config(
        &iii,
        Arc::new(NoopSummariser),
        cfg.compaction_defaults(),
    )
    .context("context-compaction register failed")?;
    log::info!(
        "context-compaction registered (context_compaction::watcher, context_compaction::compactor); threshold_pct={}; fallback_context_window={}",
        cfg.threshold_pct,
        cfg.fallback_context_window
    );

    tokio::signal::ctrl_c().await.ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_yaml_config_overrides_threshold_and_window() {
        let overrides = parse_config_overrides(
            r"
threshold_pct: 0.75
fallback_context_window: 128000
",
        );

        assert_eq!(overrides.threshold_pct, Some(0.75));
        assert_eq!(overrides.fallback_context_window, Some(128_000));
    }

    #[test]
    fn parse_json_config_accepts_context_window_alias() {
        let overrides = parse_config_overrides(
            r#"{ "config": { "threshold_pct": 0.7, "context_window": 1 } }"#,
        );

        assert_eq!(overrides.threshold_pct, Some(0.7));
        assert_eq!(overrides.fallback_context_window, Some(1));
    }
}
