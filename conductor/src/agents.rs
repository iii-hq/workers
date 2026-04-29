use std::path::Path;

use crate::git::{run_cmd, CmdResult};
use crate::types::{AgentKind, AgentSpec};

fn default_bin(kind: AgentKind) -> Option<&'static str> {
    match kind {
        AgentKind::Claude => Some("claude"),
        AgentKind::Codex => Some("codex"),
        AgentKind::Gemini => Some("gemini"),
        AgentKind::Aider => Some("aider"),
        AgentKind::Cursor => Some("cursor-agent"),
        AgentKind::Amp => Some("amp"),
        AgentKind::Opencode => Some("opencode"),
        AgentKind::Qwen => Some("qwen"),
        AgentKind::Remote => None,
    }
}

fn build_args(spec: &AgentSpec) -> Vec<String> {
    if let Some(args) = &spec.args {
        if !args.is_empty() {
            return args.clone();
        }
    }
    let prompt = spec.prompt.clone().unwrap_or_default();
    match spec.kind {
        AgentKind::Claude => {
            let mut a = vec!["--print".to_string()];
            if spec.worktree {
                a.push("--worktree".to_string());
            }
            if !prompt.is_empty() {
                a.push(prompt);
            }
            a
        }
        AgentKind::Codex => {
            if prompt.is_empty() {
                vec!["exec".to_string()]
            } else {
                vec!["exec".to_string(), prompt]
            }
        }
        AgentKind::Gemini => {
            if prompt.is_empty() {
                Vec::new()
            } else {
                vec!["--prompt".to_string(), prompt]
            }
        }
        _ => {
            if prompt.is_empty() {
                Vec::new()
            } else {
                vec![prompt]
            }
        }
    }
}

pub async fn run_local_agent(spec: &AgentSpec, cwd: &Path, timeout_ms: Option<u64>) -> CmdResult {
    if spec.kind == AgentKind::Remote {
        return CmdResult {
            ok: false,
            code: None,
            stdout: String::new(),
            stderr: "remote agent must be invoked through iii.trigger".to_string(),
        };
    }
    let bin = match spec
        .bin
        .clone()
        .or_else(|| default_bin(spec.kind).map(String::from))
    {
        Some(b) => b,
        None => {
            return CmdResult {
                ok: false,
                code: None,
                stdout: String::new(),
                stderr: format!("no binary configured for agent kind {:?}", spec.kind),
            };
        }
    };
    let args = build_args(spec);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_cmd(cwd, &bin, &arg_refs, timeout_ms).await
}
