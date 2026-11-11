use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::accounts::ClaudeLaunchInput;
use super::{register_fn, spec, Ctx, FunctionSpec, KilledOutput};
use crate::tmux_cli::{
    format_literal, resolve_cwd, unescape_separators, validate_command, validate_name, HOLD, SEP,
};

pub const CREATE: &str = "tmux::sessions::create";
pub const RENAME: &str = "tmux::sessions::rename";
pub const KILL: &str = "tmux::sessions::kill";

const CREATE_DESC: &str = "Create a detached tmux session (starting the tmux server if needed). Optional name, working directory (default $HOME), first window name and a shell command to run instead of the default shell. With remain_on_exit, the pane stays once its program ends, dead, for tmux::panes::respawn to restart it.";
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
    /// Keep the pane once its program ends, shown dead, so tmux::panes::respawn can restart it (default false: tmux closes it). On tmux before 3.0, which has no pane options, every pane of its window is kept.
    #[serde(default)]
    pub remain_on_exit: bool,
    /// Start Claude Code on an account in the first window instead of `command` (not both): the worker builds the command (returned as `runs`) and marks the pane with its account.
    #[serde(default)]
    pub claude: Option<ClaudeLaunchInput>,
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
    /// With `claude`: the exact command the pane runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs: Option<String>,
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
    let line = unescape_separators(output.lines().next().unwrap_or_default());
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
        runs: None,
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
        args.extend(["-s".into(), tmux.name_arg(name)]);
    }
    if let Some(window_name) = input.window_name.as_deref() {
        validate_name("window", window_name)?;
        args.extend(["-n".into(), tmux.new_window_name_arg(window_name)]);
    }
    let cwd = match input.cwd.as_deref() {
        Some(cwd) => resolve_cwd(cwd)?,
        None => resolve_cwd("~").unwrap_or_else(|_| "/".to_string()),
    };
    args.extend(["-c".into(), format_literal(&cwd)]);
    let launch = super::accounts::plan(ctx, input.claude.as_ref(), input.command.as_deref())?;
    if launch.is_some() {
        args.extend(["--".into(), HOLD.to_string()]);
    } else if let Some(command) = input.command.as_deref() {
        validate_command(command)?;
        let first = if input.remain_on_exit { HOLD } else { command };
        args.extend(["--".into(), first.to_string()]);
    }
    let output = tmux.run(&args).await.map_err(|error| error.sentence())?;
    ctx.monitor.poke();
    let mut created = parse_created(&output)?;
    if let Some(launch) = launch {
        super::accounts::start_claude(ctx, &tmux, &created.pane_id, &launch, input.remain_on_exit)
            .await?;
        created.runs = Some(launch.command);
    } else if input.remain_on_exit {
        tmux.keep_when_done(&created.pane_id, input.command.as_deref())
            .await?;
    }
    Ok(created)
}

pub async fn rename(ctx: &Ctx, input: RenameSessionInput) -> Result<RenamedSession, String> {
    validate_name("session", &input.name)?;
    let tmux = ctx.tmux().await?;
    let session_id = tmux.resolve_session(&input.session).await?;
    tmux.run([
        "rename-session",
        "-t",
        &session_id,
        "--",
        &tmux.name_arg(&input.name),
    ])
    .await
    .map_err(|error| error.sentence())?;
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
        .map_err(|error| error.sentence())?;
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
        let escaped = parse_created("$3\\037my sess\\037@7\\0370\\037%9\n").unwrap();
        assert_eq!(
            (escaped.session.as_str(), escaped.pane_id.as_str()),
            ("my sess", "%9")
        );
        assert!(parse_created("nonsense").is_err());
    }
}
