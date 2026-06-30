//! Build the `grok` argv from a run request. The worker drives the Grok CLI in
//! headless print mode (`grok -p <prompt> --output-format streaming-json`) so
//! the turn streams machine-readable events on stdout. Mirrors the documented
//! Grok Build CLI flags at <https://docs.x.ai>.

use crate::functions::types::RunRequest;

/// Resolved per-turn settings after merging payload over config defaults.
pub struct ResolvedOptions {
    pub model: String,
    pub cwd: String,
    /// Auto-approve tool/command execution in headless runs.
    pub always_approve: bool,
    /// Extra directories the agent may read/write.
    pub additional_directories: Vec<String>,
    /// developer/system instructions prepended to the prompt (the iii block).
    pub instructions: Option<String>,
}

/// Build argv for a headless Grok turn. `prompt` is the user message for this
/// turn; `resume_session` continues an existing Grok session by id.
pub fn build_args(
    prompt: &str,
    opts: &ResolvedOptions,
    resume_session: Option<&str>,
) -> Vec<String> {
    // Grok has no developer-instructions flag, so the iii context block is
    // prepended to the prompt on the first turn only. A resumed session already
    // carries it in its history.
    let prompt = match &opts.instructions {
        Some(ctx) if !ctx.is_empty() && resume_session.is_none() => {
            format!("{ctx}\n\n{prompt}")
        }
        _ => prompt.to_string(),
    };
    let mut a: Vec<String> = vec![
        "--print".into(),
        prompt,
        "--output-format".into(),
        "streaming-json".into(),
        // Headless: never paint the TUI alt-screen or self-update mid-run.
        "--no-alt-screen".into(),
        "--no-auto-update".into(),
    ];

    if !opts.model.is_empty() {
        a.push("--model".into());
        a.push(opts.model.clone());
    }
    if !opts.cwd.is_empty() {
        a.push("--cwd".into());
        a.push(opts.cwd.clone());
    }
    if opts.always_approve {
        a.push("--always-approve".into());
    }
    for dir in &opts.additional_directories {
        a.push("--add-dir".into());
        a.push(dir.clone());
    }
    if let Some(session) = resume_session {
        a.push("--session".into());
        a.push(session.to_string());
    }
    a
}

/// Merge payload over config defaults into ResolvedOptions, injecting the iii
/// developer instructions block unless the caller supplied one.
pub fn resolve(
    req: &RunRequest,
    cfg: &crate::config::Config,
    prior_model: Option<&str>,
    prior_cwd: Option<&str>,
    iii_prompt: Option<&str>,
) -> ResolvedOptions {
    let d = &cfg.defaults;
    let model = req
        .model
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| prior_model.filter(|s| !s.is_empty()).map(str::to_string))
        .unwrap_or_else(|| d.model.clone());
    let cwd = req
        .cwd
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| prior_cwd.filter(|s| !s.is_empty()).map(str::to_string))
        .unwrap_or_else(|| d.cwd.clone());

    ResolvedOptions {
        model,
        cwd,
        always_approve: req.always_approve.unwrap_or(d.always_approve),
        additional_directories: req.additional_directories.clone().unwrap_or_default(),
        instructions: iii_prompt.map(str::to_string),
    }
}
