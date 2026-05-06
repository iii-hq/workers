use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use iii_sdk::{register_worker, InitOptions};
use serde_json::Value;

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

#[derive(Debug, Default, PartialEq, Eq)]
struct ConfigOverrides {
    max_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuntimeConfig {
    max_bytes: u64,
}

impl RuntimeConfig {
    fn load(config_path: Option<&Path>) -> Self {
        let mut cfg = Self {
            max_bytes: document_extract::DEFAULT_MAX_BYTES,
        };

        if let Some(path) = config_path {
            match load_config_overrides(path) {
                Ok(overrides) => cfg.apply(overrides),
                Err(e) => log::warn!(
                    "failed to load document-extract config from {}: {e}; using defaults",
                    path.display()
                ),
            }
        }

        if let Ok(max_bytes) = std::env::var("DOCUMENT_EXTRACT_MAX_BYTES") {
            if let Ok(max_bytes) = max_bytes.trim().parse() {
                cfg.max_bytes = max_bytes;
            }
        }

        cfg
    }

    fn apply(&mut self, overrides: ConfigOverrides) {
        if let Some(max_bytes) = overrides.max_bytes {
            self.max_bytes = max_bytes;
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
    let max_bytes = object.get("max_bytes").and_then(Value::as_u64);
    Some(ConfigOverrides { max_bytes })
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
    if key == "max_bytes" {
        overrides.max_bytes = parse_yaml_scalar(value).and_then(|v| v.parse().ok());
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

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let engine_url = std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
    let cfg = RuntimeConfig::load(config_path_arg().as_deref());
    let iii = Arc::new(register_worker(&engine_url, InitOptions::default()));

    let _refs = document_extract::register_with_iii_with_config(
        &iii,
        document_extract::DocumentExtractConfig {
            max_bytes: cfg.max_bytes,
        },
    );
    log::info!(
        "document-extract registered (document::extract); max_bytes={}",
        cfg.max_bytes
    );

    tokio::signal::ctrl_c().await.ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_yaml_config_overrides_max_bytes() {
        let overrides = parse_config_overrides("max_bytes: 1024\n");
        assert_eq!(overrides.max_bytes, Some(1024));
    }

    #[test]
    fn parse_json_config_overrides_wrapped_config() {
        let overrides = parse_config_overrides(r#"{ "config": { "max_bytes": 2048 } }"#);
        assert_eq!(overrides.max_bytes, Some(2048));
    }
}
