use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{register_fn, spec, Ctx, FunctionSpec, KilledOutput};
use crate::tmux_cli::{resolve_cwd, validate_command, validate_name, SEP};

pub const CREATE: &str = "tmux::sessions::create";
pub const RENAME: &str = "tmux::sessions::rename";
pub const KILL: &str = "tmux::sessions::kill";

const CREATE_DESC: &str = "Create a detached tmux session (starting the tmux server if needed). Optional name, working directory (default $HOME), first window name and a shell command to run instead of the default shell.";
const RENAME_DESC: &str = "Rename a tmux session.";
const KILL_DESC: &str = "Kill a tmux session and every program running in its windows.";

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct CreateSessionInput {
    /// Session name; tmux picks a number when omitted. No `:` or `.`.
    #[serde(default)]
    pub name: Option<String>,
    /// Working directory, absolute or `~/…`. Defaults to $HOME.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Shell command to run in the first window instead of the default shell.
    #[serde(default)]
    pub command: Option<String>,
    /// Name of the first window.
    #[serde(default)]
    pub window_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CreatedSession {
    /// New session id, e.g. `$7`.
    pub session_id: String,
    /// Session name.
    pub session: String,
    /// First window id.
    pub window_id: String,
    /// First window index.
    pub window_index: u32,
    /// First pane id.
    pub pane_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RenameSessionInput {
    /// Session id (`$5`) or exact name.
    pub session: String,
    /// New name. No `:` or `.`.
    pub name: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RenamedSession {
    /// Session id.
    pub session_id: String,
    /// New name.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SessionInput {
    /// Session id (`$5`) or exact name.
    pub session: String,
}

/// `#{session_id}␟#{session_name}␟#{window_id}␟#{window_index}␟#{pane_id}`
pub fn created_format() -> String {
    [
        "#{session_id}",
        "#{session_name}",
        "#{window_id}",
        "#{window_index}",
        "#{pane_id}",
    ]
    .join(&SEP.to_string())
}

pub fn parse_created(output: &str) -> Result<CreatedSession, String> {
    let line = output.lines().next().unwrap_or_default();
    let fields: Vec<&str> = line.split(SEP).collect();
    if fields.len() != 5 {
        return Err(format!("unexpected tmux output: {line:?}"));
    }
    Ok(CreatedSession {
        session_id: fields[0].to_string(),
        session: fields[1].to_string(),
        window_id: fields[2].to_string(),
        window_index: fields[3].parse().unwrap_or_default(),
        pane_id: fields[4].to_string(),
    })
}

pub async fn create(ctx: &Ctx, input: CreateSessionInput) -> Result<CreatedSession, String> {
    let tmux = ctx.tmux().await?;
    let mut args: Vec<String> = vec![
        "new-session".into(),
        "-d".into(),
        "-P".into(),
        "-F".into(),
        created_format(),
    ];
    if let Some(name) = input.name.as_deref() {
        validate_name("session", name)?;
        args.extend(["-s".into(), name.to_string()]);
    }
    if let Some(window_name) = input.window_name.as_deref() {
        validate_name("window", window_name)?;
        args.extend(["-n".into(), window_name.to_string()]);
    }
    let cwd = match input.cwd.as_deref() {
        Some(cwd) => resolve_cwd(cwd)?,
        None => resolve_cwd("~").unwrap_or_else(|_| "/".to_string()),
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

pub async fn rename(ctx: &Ctx, input: RenameSessionInput) -> Result<RenamedSession, String> {
    validate_name("session", &input.name)?;
    let tmux = ctx.tmux().await?;
    let session_id = tmux.resolve_session(&input.session).await?;
    tmux.run(["rename-session", "-t", &session_id, "--", &input.name])
        .await
        .map_err(|error| error.to_string())?;
    ctx.monitor.poke();
    Ok(RenamedSession {
        session_id,
        name: input.name,
    })
}

pub async fn kill(ctx: &Ctx, input: SessionInput) -> Result<KilledOutput, String> {
    let tmux = ctx.tmux().await?;
    let session_id = tmux.resolve_session(&input.session).await?;
    tmux.run(["kill-session", "-t", &session_id])
        .await
        .map_err(|error| error.to_string())?;
    ctx.monitor.poke();
    Ok(KilledOutput { killed: true })
}

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<CreateSessionInput, CreatedSession>(CREATE, CREATE_DESC),
        spec::<RenameSessionInput, RenamedSession>(RENAME, RENAME_DESC),
        spec::<SessionInput, KilledOutput>(KILL, KILL_DESC),
    ]
}

pub fn register(iii: &IIIClient, ctx: &Arc<Ctx>) {
    register_fn!(iii, ctx, CREATE, CREATE_DESC, CreateSessionInput, create);
    register_fn!(iii, ctx, RENAME, RENAME_DESC, RenameSessionInput, rename);
    register_fn!(iii, ctx, KILL, KILL_DESC, SessionInput, kill);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn created_output_parses() {
        let line = ["$3", "my sess", "@7", "0", "%9"].join(&SEP.to_string());
        let created = parse_created(&format!("{line}\n")).unwrap();
        assert_eq!(created.session_id, "$3");
        assert_eq!(created.session, "my sess");
        assert_eq!(
            (created.window_id.as_str(), created.window_index),
            ("@7", 0)
        );
        assert_eq!(created.pane_id, "%9");
        assert!(parse_created("nonsense").is_err());
    }
}
