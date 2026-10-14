use std::collections::HashSet;
use std::sync::Arc;

use arc_swap::ArcSwap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub type SharedConfig = Arc<ArcSwap<WorkerConfig>>;

pub const CONFIG_ID: &str = "tmux";
pub const CONFIG_NAME: &str = "tmux";
pub const CONFIG_DESCRIPTION: &str =
    "tmux binary and server socket, polling cadence, console terminal limits, and the agent CLIs recognised in panes.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WorkerConfig {
    /// tmux executable name (looked up on PATH, never this worker's own binary) or absolute path.
    pub tmux_binary: String,
    /// tmux server socket name (`tmux -L`). Unset uses the default server.
    pub socket_name: Option<String>,
    /// tmux server socket path (`tmux -S`). Wins over `socket_name`.
    pub socket_path: Option<String>,
    /// How often the live feed re-reads sessions, windows and agent state, in milliseconds.
    pub poll_interval_ms: u64,
    /// How often the process table and listening ports are re-scanned, in milliseconds.
    pub ports_interval_ms: u64,
    /// An agent without `working_patterns` counts as working while its pane changed within this window, in milliseconds.
    pub busy_window_ms: u64,
    /// Maximum time for one tmux command, in milliseconds.
    pub command_timeout_ms: u64,
    /// Maximum concurrent console terminals (each one is an attached tmux client).
    pub max_terminals: usize,
    /// A console terminal with no input or resize heartbeat for this long is detached, in milliseconds.
    pub terminal_idle_timeout_ms: u64,
    /// Agent CLIs recognised in pane process trees, first match wins.
    pub agents: Vec<AgentMatcher>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentMatcher {
    /// Stable id reported in pane status, e.g. `claude`.
    pub id: String,
    /// Display name, e.g. `Claude Code`.
    pub label: String,
    /// Process names (or argv[0]/argv[1] basenames) that identify the agent.
    pub process: Vec<String>,
    /// Case-insensitive regexes over the pane's bottom lines that mean the agent is working.
    #[serde(default)]
    pub working_patterns: Vec<String>,
    /// Case-insensitive regexes over the pane's bottom lines that mean the agent is waiting for you.
    #[serde(default)]
    pub waiting_patterns: Vec<String>,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            tmux_binary: "tmux".to_string(),
            socket_name: None,
            socket_path: None,
            poll_interval_ms: 1_000,
            ports_interval_ms: 3_000,
            busy_window_ms: 2_000,
            command_timeout_ms: 5_000,
            max_terminals: 8,
            terminal_idle_timeout_ms: 30_000,
            agents: default_agents(),
        }
    }
}

fn matcher(
    id: &str,
    label: &str,
    process: &[&str],
    working: &[&str],
    waiting: &[&str],
) -> AgentMatcher {
    let owned = |items: &[&str]| items.iter().map(|item| item.to_string()).collect();
    AgentMatcher {
        id: id.to_string(),
        label: label.to_string(),
        process: owned(process),
        working_patterns: owned(working),
        waiting_patterns: owned(waiting),
    }
}

/// Claude Code shows `esc to interrupt` only while a turn runs, and numbered
/// `❯ 1. Yes` choices for permission and plan prompts. In a narrow pane (a phone
/// attached to the session shrinks its windows) the status line loses its
/// `esc to interrupt` tail, so the running clock of the spinner line
/// (`✻ Harmonizing… (16m 46s`) and Codex's `Working (93s` count as well.
pub fn default_agents() -> Vec<AgentMatcher> {
    vec![
        matcher(
            "claude",
            "Claude Code",
            &["claude"],
            &[r"esc to interrupt", r"…\s*\(\d+[hms]\b"],
            &[
                r"❯\s*1\.\s*Yes",
                r"Do you want to proceed\?",
                r"Enter to select",
            ],
        ),
        matcher(
            "codex",
            "Codex",
            &["codex"],
            &[r"esc to interrupt", r"\bWorking\s*\(\d+"],
            &[
                r"Would you like to (run|make|apply)",
                r"Yes, proceed",
                r"Allow command\?",
            ],
        ),
        matcher(
            "opencode",
            "OpenCode",
            &["opencode"],
            &[r"esc (to )?interrupt"],
            &[r"Permission required", r"Allow once"],
        ),
        matcher(
            "gemini",
            "Gemini CLI",
            &["gemini"],
            &[r"esc to cancel"],
            &[
                r"Allow execution",
                r"Waiting for user confirmation",
                r"Apply this change\?",
            ],
        ),
    ]
}

impl WorkerConfig {
    pub fn from_json(value: &serde_json::Value) -> Result<Self, String> {
        let inner = value.get("tmux").unwrap_or(value);
        let config: Self = serde_json::from_value(inner.clone())
            .map_err(|error| format!("invalid tmux config: {error}"))?;
        config.validate()?;
        Ok(config)
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("WorkerConfig serializes")
    }

    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(WorkerConfig))
            .expect("WorkerConfig schema serializes")
    }

    pub fn into_shared(self) -> SharedConfig {
        Arc::new(ArcSwap::from_pointee(self))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.tmux_binary.trim().is_empty() {
            return Err("tmux_binary must not be empty".to_string());
        }
        for (key, value) in [
            ("socket_name", &self.socket_name),
            ("socket_path", &self.socket_path),
        ] {
            if value
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
            {
                return Err(format!("{key} must be omitted or non-empty"));
            }
        }
        if !(250..=60_000).contains(&self.poll_interval_ms) {
            return Err("poll_interval_ms must be between 250 and 60000".to_string());
        }
        if !(1_000..=300_000).contains(&self.ports_interval_ms) {
            return Err("ports_interval_ms must be between 1000 and 300000".to_string());
        }
        if !(250..=60_000).contains(&self.busy_window_ms) {
            return Err("busy_window_ms must be between 250 and 60000".to_string());
        }
        if !(500..=60_000).contains(&self.command_timeout_ms) {
            return Err("command_timeout_ms must be between 500 and 60000".to_string());
        }
        if !(1..=64).contains(&self.max_terminals) {
            return Err("max_terminals must be between 1 and 64".to_string());
        }
        if !(5_000..=3_600_000).contains(&self.terminal_idle_timeout_ms) {
            return Err("terminal_idle_timeout_ms must be between 5000 and 3600000".to_string());
        }
        let mut ids = HashSet::new();
        for agent in &self.agents {
            if agent.id.trim().is_empty() || agent.label.trim().is_empty() {
                return Err("every agent needs a non-empty id and label".to_string());
            }
            if !ids.insert(agent.id.as_str()) {
                return Err(format!("agent id '{}' is listed twice", agent.id));
            }
            if agent.process.iter().all(|name| name.trim().is_empty()) {
                return Err(format!(
                    "agent '{}' needs at least one process name",
                    agent.id
                ));
            }
            for pattern in agent.working_patterns.iter().chain(&agent.waiting_patterns) {
                crate::agents::compile_pattern(pattern)
                    .map_err(|error| format!("agent '{}': {error}", agent.id))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        WorkerConfig::default().validate().unwrap();
    }

    #[test]
    fn wrapped_configuration_round_trips() {
        let value = serde_json::json!({"tmux": {"poll_interval_ms": 2000}});
        let config = WorkerConfig::from_json(&value).unwrap();
        assert_eq!(config.poll_interval_ms, 2000);
        assert_eq!(config.tmux_binary, "tmux");
        assert_eq!(config.agents.len(), default_agents().len());
    }

    #[test]
    fn rejects_bad_patterns_and_duplicate_agents() {
        let mut config = WorkerConfig::default();
        config.agents[0]
            .waiting_patterns
            .push("(unclosed".to_string());
        assert!(config.validate().unwrap_err().contains("claude"));

        let mut config = WorkerConfig::default();
        config.agents.push(config.agents[0].clone());
        assert!(config.validate().unwrap_err().contains("twice"));
    }

    #[test]
    fn rejects_out_of_range_intervals() {
        let config = WorkerConfig {
            poll_interval_ms: 10,
            ..WorkerConfig::default()
        };
        assert!(config.validate().is_err());
    }
}
