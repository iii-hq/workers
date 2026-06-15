//! Build the `codex exec --json` argv from a run request, mirroring the TS
//! SDK's exec.ts. Derived settings and the caller's `codex_config` pass-through
//! serialize through `--config key=value` with TOML-literal values.

use serde_json::{Map, Value};

use crate::functions::types::RunRequest;

/// Resolved per-turn settings after merging payload over config defaults.
pub struct ResolvedOptions {
    pub model: String,
    pub cwd: String,
    pub sandbox_mode: String,
    pub approval_policy: String,
    pub reasoning_effort: String,
    pub skip_git_repo_check: bool,
    pub base_url: String,
    /// developer_instructions + any caller codex_config, already merged.
    pub config: Map<String, Value>,
    pub images: Vec<String>,
    pub additional_directories: Vec<String>,
}

/// Serialize a JSON value as a TOML literal for `--config key=value`.
pub fn toml_value(v: &Value) -> String {
    match v {
        Value::String(s) => toml::Value::String(s.clone()).to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => {
            // Arrays/objects: TOML inline via the toml crate; fall back to a
            // quoted JSON string if it cannot be represented.
            match json_to_toml(other) {
                Some(tv) => tv.to_string(),
                None => toml::Value::String(other.to_string()).to_string(),
            }
        }
    }
}

fn json_to_toml(v: &Value) -> Option<toml::Value> {
    match v {
        Value::String(s) => Some(toml::Value::String(s.clone())),
        Value::Bool(b) => Some(toml::Value::Boolean(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(toml::Value::Integer(i))
            } else {
                n.as_f64().map(toml::Value::Float)
            }
        }
        Value::Array(a) => {
            let items: Option<Vec<toml::Value>> = a.iter().map(json_to_toml).collect();
            items.map(toml::Value::Array)
        }
        Value::Object(o) => {
            let mut t = toml::map::Map::new();
            for (k, val) in o {
                t.insert(k.clone(), json_to_toml(val)?);
            }
            Some(toml::Value::Table(t))
        }
        Value::Null => None,
    }
}

/// Build argv for `codex exec`. `output_schema_path` is a temp file path when a
/// schema was supplied. `resume_thread` continues an existing thread.
pub fn build_args(
    opts: &ResolvedOptions,
    resume_thread: Option<&str>,
    output_schema_path: Option<&str>,
) -> Vec<String> {
    let mut a: Vec<String> = vec!["exec".into(), "--json".into()];

    // --config overrides first (base_url + the merged config map).
    if !opts.base_url.is_empty() {
        a.push("--config".into());
        a.push(format!(
            "openai_base_url={}",
            toml_value(&Value::String(opts.base_url.clone()))
        ));
    }
    if !opts.reasoning_effort.is_empty() {
        a.push("--config".into());
        a.push(format!(
            "model_reasoning_effort={}",
            toml_value(&Value::String(opts.reasoning_effort.clone()))
        ));
    }
    if !opts.approval_policy.is_empty() {
        a.push("--config".into());
        a.push(format!(
            "approval_policy={}",
            toml_value(&Value::String(opts.approval_policy.clone()))
        ));
    }
    for (k, v) in &opts.config {
        a.push("--config".into());
        a.push(format!("{k}={}", toml_value(v)));
    }

    if !opts.model.is_empty() {
        a.push("--model".into());
        a.push(opts.model.clone());
    }
    if !opts.sandbox_mode.is_empty() {
        a.push("--sandbox".into());
        a.push(opts.sandbox_mode.clone());
    }
    if !opts.cwd.is_empty() {
        a.push("--cd".into());
        a.push(opts.cwd.clone());
    }
    if opts.skip_git_repo_check {
        a.push("--skip-git-repo-check".into());
    }
    if let Some(path) = output_schema_path {
        a.push("--output-schema".into());
        a.push(path.to_string());
    }
    for dir in &opts.additional_directories {
        a.push("--add-dir".into());
        a.push(dir.clone());
    }
    for img in &opts.images {
        a.push("--image".into());
        a.push(img.clone());
    }
    if let Some(thread) = resume_thread {
        a.push("resume".into());
        a.push(thread.to_string());
    }
    a
}

/// Merge payload over config defaults into ResolvedOptions, injecting the iii
/// developer_instructions block unless the caller supplied one.
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

    let mut config: Map<String, Value> = req
        .codex_config
        .clone()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    // developer_instructions: caller wins; otherwise inject the iii block.
    if let Some(prompt) = iii_prompt {
        config
            .entry("developer_instructions".to_string())
            .or_insert_with(|| Value::String(prompt.to_string()));
    }

    // An empty string is treated as unset: a caller must not be able to wipe
    // the operator's configured sandbox / approval defaults with "".
    let or_default = |v: &Option<String>, dflt: &str| {
        v.clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| dflt.to_string())
    };

    ResolvedOptions {
        model,
        cwd,
        sandbox_mode: or_default(&req.sandbox_mode, &d.sandbox_mode),
        approval_policy: or_default(&req.approval_policy, &d.approval_policy),
        reasoning_effort: or_default(&req.reasoning_effort, &d.reasoning_effort),
        skip_git_repo_check: req.skip_git_repo_check.unwrap_or(d.skip_git_repo_check),
        base_url: cfg.base_url.clone(),
        config,
        images: req.images.clone().unwrap_or_default(),
        additional_directories: req.additional_directories.clone().unwrap_or_default(),
    }
}
