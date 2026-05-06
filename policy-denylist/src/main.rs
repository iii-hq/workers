use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use iii_sdk::{register_worker, InitOptions};
use serde_json::Value;

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

#[derive(Debug, Default, PartialEq, Eq)]
struct ConfigOverrides {
    topic: Option<String>,
    denied_tools: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeConfig {
    topic: String,
    denied_tools: Vec<String>,
}

impl RuntimeConfig {
    fn load(config_path: Option<&Path>) -> Self {
        let mut cfg = Self {
            topic: policy_denylist::DEFAULT_TOPIC.to_string(),
            denied_tools: policy_denylist::default_denied_tools(),
        };

        if let Some(path) = config_path {
            match load_config_overrides(path) {
                Ok(overrides) => cfg.apply(overrides),
                Err(e) => log::warn!(
                    "failed to load policy-denylist config from {}: {e}; using defaults",
                    path.display()
                ),
            }
        }

        if let Ok(topic) = std::env::var("POLICY_DENYLIST_TOPIC") {
            if !topic.trim().is_empty() {
                cfg.topic = topic;
            }
        }
        if let Ok(denied) = std::env::var("POLICY_DENIED_TOOLS") {
            let denied_tools = parse_denied_tools(&denied);
            if !denied_tools.is_empty() {
                cfg.denied_tools = denied_tools;
            }
        }

        cfg
    }

    fn apply(&mut self, overrides: ConfigOverrides) {
        if let Some(topic) = overrides.topic {
            self.topic = topic;
        }
        if let Some(denied_tools) = overrides.denied_tools {
            self.denied_tools = denied_tools;
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

    let topic = object
        .get("topic")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let denied_tools = object
        .get("denied_tools")
        .and_then(parse_json_denied_tools)
        .filter(|v| !v.is_empty());

    Some(ConfigOverrides {
        topic,
        denied_tools,
    })
}

fn parse_json_denied_tools(value: &Value) -> Option<Vec<String>> {
    if let Some(list) = value.as_array() {
        let out: Vec<String> = list
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        return Some(out);
    }
    value.as_str().map(parse_denied_tools)
}

fn parse_yaml_config_overrides(raw: &str) -> ConfigOverrides {
    let mut overrides = ConfigOverrides::default();
    let mut in_config_block = false;
    let mut config_indent = None;
    let mut denied_tools_list: Vec<String> = Vec::new();
    let mut reading_denied_tools = false;
    let mut denied_tools_indent = 0;

    for raw_line in raw.lines() {
        let indent = raw_line.chars().take_while(|ch| *ch == ' ').count();
        let line = strip_yaml_comment(raw_line).trim_end();
        let line = line.trim_start();
        if line.is_empty() || line == "---" {
            continue;
        }

        if reading_denied_tools && indent >= denied_tools_indent {
            if let Some(item) = line.strip_prefix("- ") {
                if let Some(value) = parse_yaml_scalar(item) {
                    denied_tools_list.push(value);
                }
                continue;
            }
        }
        reading_denied_tools = false;

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
            parse_config_key(
                &mut overrides,
                key,
                value,
                &mut reading_denied_tools,
                &mut denied_tools_indent,
                indent + 2,
            );
            continue;
        }

        if !in_config_block {
            continue;
        }

        let expected_indent = *config_indent.get_or_insert(indent);
        if indent != expected_indent {
            continue;
        }
        parse_config_key(
            &mut overrides,
            key.trim(),
            value,
            &mut reading_denied_tools,
            &mut denied_tools_indent,
            indent + 2,
        );
    }

    if !denied_tools_list.is_empty() {
        overrides.denied_tools = Some(denied_tools_list);
    }
    overrides
}

fn parse_config_key(
    overrides: &mut ConfigOverrides,
    key: &str,
    value: &str,
    reading_denied_tools: &mut bool,
    denied_tools_indent: &mut usize,
    list_indent: usize,
) {
    match key {
        "topic" => {
            if let Some(value) = parse_yaml_scalar(value) {
                overrides.topic = Some(value);
            }
        }
        "denied_tools" => {
            if let Some(value) = parse_yaml_scalar(value) {
                let denied_tools = parse_denied_tools(&value);
                if !denied_tools.is_empty() {
                    overrides.denied_tools = Some(denied_tools);
                }
            } else {
                *reading_denied_tools = true;
                *denied_tools_indent = list_indent;
            }
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

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let engine_url = std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
    let cfg = RuntimeConfig::load(config_path_arg().as_deref());

    let iii = register_worker(&engine_url, InitOptions::default());
    let _sub = policy_denylist::subscribe_denylist_with_config(
        &iii,
        cfg.denied_tools.clone(),
        policy_denylist::PolicyDenylistConfig {
            topic: cfg.topic.clone(),
        },
    )
    .map_err(|e| anyhow!("subscribe failed: {e}"))?;
    log::info!(
        "policy-denylist registered (policy::denylist on {}); denied=[{}]",
        cfg.topic,
        cfg.denied_tools.join(", ")
    );

    tokio::signal::ctrl_c().await.ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_yaml_config_overrides_topic_and_denied_tools() {
        let overrides = parse_config_overrides(
            r"
topic: agent::custom_before_tool_call
denied_tools:
  - bash:rm -rf
  - sudo
",
        );

        assert_eq!(
            overrides.topic.as_deref(),
            Some("agent::custom_before_tool_call")
        );
        assert_eq!(
            overrides.denied_tools,
            Some(vec!["bash:rm -rf".to_string(), "sudo".to_string()])
        );
    }

    #[test]
    fn parse_json_config_overrides_wrapped_config() {
        let overrides = parse_config_overrides(
            r#"{
                "config": {
                    "topic": "agent::json_before_tool_call",
                    "denied_tools": ["bash:rm -rf", "sudo"]
                }
            }"#,
        );

        assert_eq!(
            overrides.topic.as_deref(),
            Some("agent::json_before_tool_call")
        );
        assert_eq!(
            overrides.denied_tools,
            Some(vec!["bash:rm -rf".to_string(), "sudo".to_string()])
        );
    }
}
