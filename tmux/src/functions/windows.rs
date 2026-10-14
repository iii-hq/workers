use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{register_fn, spec, Ctx, FunctionSpec, KilledOutput};
use crate::tmux_cli::{
    resolve_cwd, unescape_separators, validate_command, validate_name, window_target, WindowArg,
    WindowRef, SEP,
};

pub const CREATE: &str = "tmux::windows::create";
pub const SELECT: &str = "tmux::windows::select";
pub const RENAME: &str = "tmux::windows::rename";
pub const KILL: &str = "tmux::windows::kill";

const CREATE_DESC: &str = "Create a window in a tmux session. Optional name, working directory (default: the directory the session was created in), a shell command to run instead of the default shell, and whether to make it the current window (default true).";
const SELECT_DESC: &str = "Make a window the current window of its session. Like tmux itself, every client attached to that session (your terminal included) switches to it.";
const RENAME_DESC: &str = "Rename a tmux window.";
const KILL_DESC: &str = "Kill a tmux window and every program running in its panes.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateWindowInput {
    /// Session id (`$5`) or exact name.
    pub session: String,
    /// Window name. No `:` or `.`.
    #[serde(default)]
    pub name: Option<String>,
    /// Working directory, absolute or `~/…`. Defaults to the directory the session was created in.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Shell command to run instead of the default shell.
    #[serde(default)]
    pub command: Option<String>,
    /// Make it the session's current window (default true).
    #[serde(default)]
    pub select: Option<bool>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CreatedWindow {
    /// Session id.
    pub session_id: String,
    /// New window id, e.g. `@9`.
    pub window_id: String,
    /// New window index.
    pub window_index: u32,
    /// Its first pane id.
    pub pane_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WindowInput {
    /// Session id (`$5`) or exact name.
    pub session: String,
    /// Window id (`@3`) or index.
    pub window: WindowArg,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SelectedWindow {
    /// Session id.
    pub session_id: String,
    /// Window id now current.
    pub window_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RenameWindowInput {
    /// Session id (`$5`) or exact name.
    pub session: String,
    /// Window id (`@3`) or index.
    pub window: WindowArg,
    /// New name. No `:` or `.`.
    pub name: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RenamedWindow {
    /// Window id.
    pub window_id: String,
    /// New name.
    pub name: String,
}

pub fn parse_created(output: &str) -> Result<CreatedWindow, String> {
    let line = unescape_separators(output.lines().next().unwrap_or_default());
    let fields: Vec<&str> = line.split(SEP).collect();
    if fields.len() != 4 {
        return Err(format!("unexpected tmux output: {line:?}"));
    }
    Ok(CreatedWindow {
        session_id: fields[0].to_string(),
        window_id: fields[1].to_string(),
        window_index: fields[2].parse().unwrap_or_default(),
        pane_id: fields[3].to_string(),
    })
}

/// The session and window id a [`WindowInput`]-like pair names.
async fn locate(
    ctx: &Ctx,
    session: &str,
    window: &WindowArg,
) -> Result<(crate::tmux_cli::Tmux, String, String), String> {
    let tmux = ctx.tmux().await?;
    let session_id = tmux.resolve_session(session).await?;
    let window_id = tmux
        .resolve_window(&session_id, &WindowRef::parse(window)?)
        .await?;
    Ok((tmux, session_id, window_id))
}

pub async fn create(ctx: &Ctx, input: CreateWindowInput) -> Result<CreatedWindow, String> {
    let tmux = ctx.tmux().await?;
    let session_id = tmux.resolve_session(&input.session).await?;
    let format = [
        "#{session_id}",
        "#{window_id}",
        "#{window_index}",
        "#{pane_id}",
    ]
    .join(&SEP.to_string());
    let mut args: Vec<String> = vec!["new-window".into(), "-P".into(), "-F".into(), format];
    if input.select == Some(false) {
        args.push("-d".into());
    }
    // `$5:` is "the next free index in that session".
    args.extend(["-t".into(), format!("{session_id}:")]);
    if let Some(name) = input.name.as_deref() {
        validate_name("window", name)?;
        args.extend(["-n".into(), name.to_string()]);
    }
    // Without -c, tmux would use the cwd of the process asking (this
    // worker's); the session's own directory is what a caller expects.
    let cwd = match input.cwd.as_deref() {
        Some(cwd) => resolve_cwd(cwd)?,
        None => "#{session_path}".to_string(),
    };
    args.extend(["-c".into(), cwd]);
    if let Some(command) = input.command.as_deref() {
        validate_command(command)?;
        args.extend(["--".into(), command.to_string()]);
    }
    let output = tmux.run(&args).await.map_err(|error| error.to_string())?;
    let created = parse_created(&output)?;
    ctx.monitor.poke();
    Ok(created)
}

pub async fn select(ctx: &Ctx, input: WindowInput) -> Result<SelectedWindow, String> {
    let (tmux, session_id, window_id) = locate(ctx, &input.session, &input.window).await?;
    tmux.run([
        "select-window",
        "-t",
        &window_target(&session_id, &WindowRef::Id(window_id.clone())),
    ])
    .await
    .map_err(|error| error.to_string())?;
    ctx.monitor.poke();
    Ok(SelectedWindow {
        session_id,
        window_id,
    })
}

pub async fn rename(ctx: &Ctx, input: RenameWindowInput) -> Result<RenamedWindow, String> {
    validate_name("window", &input.name)?;
    let (tmux, session_id, window_id) = locate(ctx, &input.session, &input.window).await?;
    tmux.run([
        "rename-window",
        "-t",
        &window_target(&session_id, &WindowRef::Id(window_id.clone())),
        "--",
        &input.name,
    ])
    .await
    .map_err(|error| error.to_string())?;
    ctx.monitor.poke();
    Ok(RenamedWindow {
        window_id,
        name: input.name,
    })
}

pub async fn kill(ctx: &Ctx, input: WindowInput) -> Result<KilledOutput, String> {
    let (tmux, session_id, window_id) = locate(ctx, &input.session, &input.window).await?;
    tmux.run([
        "kill-window",
        "-t",
        &window_target(&session_id, &WindowRef::Id(window_id)),
    ])
    .await
    .map_err(|error| error.to_string())?;
    ctx.monitor.poke();
    Ok(KilledOutput { killed: true })
}

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<CreateWindowInput, CreatedWindow>(CREATE, CREATE_DESC),
        spec::<WindowInput, SelectedWindow>(SELECT, SELECT_DESC),
        spec::<RenameWindowInput, RenamedWindow>(RENAME, RENAME_DESC),
        spec::<WindowInput, KilledOutput>(KILL, KILL_DESC),
    ]
}

pub fn register(iii: &IIIClient, ctx: &Arc<Ctx>) {
    register_fn!(iii, ctx, CREATE, CREATE_DESC, CreateWindowInput, create);
    register_fn!(iii, ctx, SELECT, SELECT_DESC, WindowInput, select);
    register_fn!(iii, ctx, RENAME, RENAME_DESC, RenameWindowInput, rename);
    register_fn!(iii, ctx, KILL, KILL_DESC, WindowInput, kill);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn created_output_parses() {
        let line = ["$3", "@8", "2", "%11"].join(&SEP.to_string());
        let created = parse_created(&line).unwrap();
        assert_eq!(
            (
                created.session_id.as_str(),
                created.window_id.as_str(),
                created.window_index,
                created.pane_id.as_str()
            ),
            ("$3", "@8", 2, "%11")
        );
        let escaped = parse_created("$3\\037@8\\0372\\037%11").unwrap();
        assert_eq!(escaped.window_index, 2);
        assert!(parse_created("").is_err());
    }
}
