use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::accounts::ClaudeLaunchInput;
use super::{register_fn, spec, Ctx, FunctionSpec, KilledOutput};
use crate::tmux_cli::{
    find_window, format_literal, resolve_cwd, unescape_separators, validate_command, validate_name,
    window_target, TmuxError, WindowArg, WindowRef, HOLD, SEP,
};

pub const CREATE: &str = "tmux::windows::create";
pub const SELECT: &str = "tmux::windows::select";
pub const RENAME: &str = "tmux::windows::rename";
pub const KILL: &str = "tmux::windows::kill";
pub const MOVE: &str = "tmux::windows::move";

const CREATE_DESC: &str = "Create a window in a tmux session. Optional name, working directory (default: the directory the session was created in), a shell command to run instead of the default shell, and whether to make it the current window (default true). With remain_on_exit, its pane stays once its program ends, dead, for tmux::panes::respawn to restart it.";
const SELECT_DESC: &str = "Make a window the current window of its session. Like tmux itself, every client attached to that session (your terminal included) switches to it.";
const RENAME_DESC: &str = "Rename a tmux window.";
const KILL_DESC: &str = "Kill a tmux window and every program running in its panes.";
const MOVE_DESC: &str = "Move a window to another tmux session, after its last window or just before or after one of its windows (`before` / `after`, which need tmux 3.2 or later and also reorder a window inside its own session). The window keeps its id, panes and programs, and the destination session's current window stays as it was. When it was the last window of its session, tmux closes that session.";

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct CreateWindowInput {
    /// Session id (`$5`) or exact name.
    pub session: String,
    /// Window name. No `:` or `.`.
    #[serde(default)]
    pub name: Option<String>,
    /// Working directory, absolute or `~/…`. Defaults to the directory the session was created in
    /// (the home folder on tmux older than 3.2, which can't tell).
    #[serde(default)]
    pub cwd: Option<String>,
    /// Shell command to run instead of the default shell.
    #[serde(default)]
    pub command: Option<String>,
    /// Make it the session's current window (default true).
    #[serde(default)]
    pub select: Option<bool>,
    /// Keep its pane once its program ends, shown dead, so tmux::panes::respawn can restart it (default false: tmux closes it). On tmux before 3.0, which has no pane options, every pane of the window is kept.
    #[serde(default)]
    pub remain_on_exit: bool,
    /// Start Claude Code on an account instead of `command` (not both): the worker builds the command (returned as `runs`) and marks the pane with its account.
    #[serde(default)]
    pub claude: Option<ClaudeLaunchInput>,
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
    /// With `claude`: the exact command the pane runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs: Option<String>,
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveWindowInput {
    /// Session id (`$5`) or exact name of the session the window is in.
    pub session: String,
    /// Window id (`@3`) or index.
    pub window: WindowArg,
    /// Session id (`$5`) or exact name of the session to move it to. Its own session reorders it, with `before` or `after`.
    pub target_session: String,
    /// Put it just before this window of `target_session` (id or index). Needs tmux 3.2 or later.
    #[serde(default)]
    pub before: Option<WindowArg>,
    /// Put it just after this window of `target_session` (id or index). Needs tmux 3.2 or later. Without `before` or `after` it goes after the session's last window.
    #[serde(default)]
    pub after: Option<WindowArg>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MovedWindow {
    /// Id of the session the window is in now.
    pub session_id: String,
    /// Window id, the same as before the move.
    pub window_id: String,
    /// Its index in that session now.
    pub window_index: u32,
    /// It was the last window of its session, so tmux closed that session.
    pub source_session_closed: bool,
}

/// Which side of another window a moved window lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Before,
    After,
}

impl Side {
    /// `move-window`'s flag. Both need tmux 3.2: `-b` came then, and `-a`
    /// (from 2.1) only then started to mean "after the window -t names";
    /// before, it puts the window after the session's current one whatever
    /// -t says, and inside one session it ends tmux 2.6's and 3.0a's server
    /// (seen in containers; 3.1c survives it).
    fn flag(self) -> &'static str {
        match self {
            Self::Before => "-b",
            Self::After => "-a",
        }
    }

    /// The one sentence for a tmux before 3.2, `version` when known.
    fn too_old(self, version: &str) -> String {
        let word = match self {
            Self::Before => "before",
            Self::After => "after",
        };
        if version.is_empty() {
            format!("Moving a window {word} another needs tmux 3.2 or later.")
        } else {
            format!(
                "Moving a window {word} another needs tmux 3.2 or later, and this computer has tmux {version}."
            )
        }
    }
}

const INDEX_TAKEN: &str = "Another window took that place just now; move it again.";

/// The new index (first line), then the id of every session left.
pub fn parse_moved(output: &str) -> Result<(u32, Vec<String>), String> {
    let mut lines = output.lines();
    let first = lines.next().unwrap_or_default();
    let index = first
        .trim()
        .parse()
        .map_err(|_| format!("unexpected tmux output: {first:?}"))?;
    let sessions = lines
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    Ok((index, sessions))
}

/// tmux's refusal of a move, in one sentence where tmux's own words don't
/// say what to do.
fn move_error(error: TmuxError, side: Option<Side>, version: &str) -> String {
    let message = error.to_string();
    match side {
        // A tmux this check couldn't date (`next-3.2`, `master`) and that
        // doesn't know the flag.
        Some(side)
            if ["unknown flag", "unknown option", "usage:"]
                .iter()
                .any(|words| message.contains(words)) =>
        {
            side.too_old(version)
        }
        _ if message.contains("sessions are grouped") => {
            "Those two sessions are grouped, so they already share their windows.".to_string()
        }
        // A computed index (tmux before 3.2) another client took first.
        _ if message.contains("index in use") => INDEX_TAKEN.to_string(),
        _ => error.sentence(),
    }
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
        runs: None,
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
        args.extend(["-n".into(), tmux.new_window_name_arg(name)]);
    }
    // Without -c, tmux would use the cwd of the process asking (this
    // worker's); the session's own directory is what a caller expects.
    let cwd = match input.cwd.as_deref() {
        Some(cwd) => format_literal(&resolve_cwd(cwd)?),
        None => "#{session_path}".to_string(),
    };
    args.extend(["-c".into(), cwd]);
    let launch =
        super::accounts::plan(ctx, input.claude.as_ref(), input.command.as_deref()).await?;
    if launch.is_some() {
        args.extend(["--".into(), HOLD.to_string()]);
    } else if let Some(command) = input.command.as_deref() {
        validate_command(command)?;
        let first = if input.remain_on_exit { HOLD } else { command };
        args.extend(["--".into(), first.to_string()]);
    }
    if let Some((launch, gate)) = launch {
        let (output, runs) = super::accounts::create_with_claude(
            &tmux,
            args,
            launch,
            gate,
            input.remain_on_exit,
            |output| parse_created(output).map(|created| created.pane_id),
        )
        .await?;
        ctx.monitor.poke();
        let mut created = parse_created(&output)?;
        created.runs = Some(runs);
        return Ok(created);
    }
    let output = tmux.run(&args).await.map_err(|error| error.sentence())?;
    ctx.monitor.poke();
    let created = parse_created(&output)?;
    if input.remain_on_exit {
        tmux.keep_when_done(&created.pane_id, input.command.as_deref())
            .await?;
    }
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
    .map_err(|error| error.sentence())?;
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
        &tmux.name_arg(&input.name),
    ])
    .await
    .map_err(|error| error.sentence())?;
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
    .map_err(|error| error.sentence())?;
    ctx.monitor.poke();
    Ok(KilledOutput { killed: true })
}

/// `move-window` keeps the window's id, panes and programs. Its `-t $2:`
/// would take the first free index, which can be a gap between two windows,
/// so a plain move goes after the last window: the window shows last, where
/// a caller that "moved it to that session" looks for it. From tmux 3.2 that
/// is `-a` after the last window's id, decided by tmux in the same step;
/// before, `-a` means something else (see [`Side::flag`]), so the index after
/// the last one is named, which another client can take first.
pub async fn move_window(ctx: &Ctx, input: MoveWindowInput) -> Result<MovedWindow, String> {
    let next_to = match (&input.before, &input.after) {
        (Some(_), Some(_)) => return Err("Give before or after, not both.".to_string()),
        (Some(window), None) => Some((Side::Before, WindowRef::parse(window)?)),
        (None, Some(window)) => Some((Side::After, WindowRef::parse(window)?)),
        (None, None) => None,
    };
    let window = WindowRef::parse(&input.window)?;
    let tmux = ctx.tmux().await?;
    let modern = tmux.at_least(3, 2);
    if let Some((side, _)) = &next_to {
        if !modern {
            return Err(side.too_old(&tmux.version));
        }
    }

    let source_id = tmux.resolve_session(&input.session).await?;
    let source_windows = tmux.windows_of(&source_id).await?;
    let moving = find_window(&source_windows, &source_id, &window)?.clone();
    let target_id = tmux.resolve_session(&input.target_session).await?;
    let same_session = source_id == target_id;
    let target_windows = if same_session {
        source_windows
    } else {
        tmux.windows_of(&target_id).await?
    };

    let mut args: Vec<String> = vec!["move-window".into()];
    let target = match &next_to {
        None if same_session => return Err("The window is already in that session.".to_string()),
        None => {
            args.push("-d".into());
            match target_windows.iter().max_by_key(|w| w.index) {
                Some(last) if modern => {
                    args.push("-a".into());
                    format!("{target_id}:{}", last.id)
                }
                Some(last) => format!("{target_id}:{}", last.index.saturating_add(1)),
                None => format!("{target_id}:"),
            }
        }
        Some((side, reference)) => {
            let neighbour = find_window(&target_windows, &target_id, reference)?;
            if neighbour.id == moving.id {
                return Err("A window can't be moved before or after itself.".to_string());
            }
            // -d keeps the destination's current window. Inside one session
            // the moved window leaves its old place, and tmux makes another
            // window current when that was the current one, unless the move
            // selects it in its new place: so no -d for the current window.
            if !(same_session && moving.active) {
                args.push("-d".into());
            }
            args.push(side.flag().into());
            format!("{target_id}:{}", neighbour.id)
        }
    };
    args.extend([
        "-s".into(),
        format!("{source_id}:{}", moving.id),
        "-t".into(),
        target,
        // Asked in the same tmux command, so nothing can change in between.
        ";".into(),
        "display-message".into(),
        "-p".into(),
        "-t".into(),
        format!("{target_id}:{}", moving.id),
        "#{window_index}".into(),
        ";".into(),
        "list-sessions".into(),
        "-F".into(),
        "#{session_id}".into(),
    ]);
    let output = tmux.run(&args).await.map_err(|error| {
        move_error(
            error,
            next_to.as_ref().map(|(side, _)| *side),
            &tmux.version,
        )
    })?;
    ctx.monitor.poke();
    let (window_index, sessions) = parse_moved(&output)?;
    Ok(MovedWindow {
        session_id: target_id,
        window_id: moving.id,
        window_index,
        source_session_closed: !same_session && !sessions.contains(&source_id),
    })
}

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<CreateWindowInput, CreatedWindow>(CREATE, CREATE_DESC),
        spec::<WindowInput, SelectedWindow>(SELECT, SELECT_DESC),
        spec::<RenameWindowInput, RenamedWindow>(RENAME, RENAME_DESC),
        spec::<WindowInput, KilledOutput>(KILL, KILL_DESC),
        spec::<MoveWindowInput, MovedWindow>(MOVE, MOVE_DESC),
    ]
}

pub fn register(iii: &IIIClient, ctx: &Arc<Ctx>) {
    register_fn!(iii, ctx, CREATE, CREATE_DESC, CreateWindowInput, create);
    register_fn!(iii, ctx, SELECT, SELECT_DESC, WindowInput, select);
    register_fn!(iii, ctx, RENAME, RENAME_DESC, RenameWindowInput, rename);
    register_fn!(iii, ctx, KILL, KILL_DESC, WindowInput, kill);
    register_fn!(iii, ctx, MOVE, MOVE_DESC, MoveWindowInput, move_window);
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

    #[test]
    fn moved_output_parses() {
        let (index, sessions) = parse_moved("4\n$0\n$2\n").unwrap();
        assert_eq!(index, 4);
        assert_eq!(sessions, vec!["$0", "$2"]);
        assert!(parse_moved("").is_err());
        assert!(parse_moved("can't find window\n").is_err());
    }

    #[test]
    fn a_tmux_without_the_flag_gets_one_sentence() {
        assert_eq!(
            Side::Before.too_old("3.1c"),
            "Moving a window before another needs tmux 3.2 or later, and this computer has tmux 3.1c."
        );
        assert_eq!(
            Side::After.too_old("3.0a"),
            "Moving a window after another needs tmux 3.2 or later, and this computer has tmux 3.0a."
        );
        assert_eq!(
            Side::After.too_old(""),
            "Moving a window after another needs tmux 3.2 or later."
        );
        // As tmux 2.6 to 3.1c and 3.6 refuse a flag they don't have.
        for refusal in [
            "tmux: unknown option -- b\nusage: move-window [-dkr] [-s src-window] [-t dst-window]",
            "command move-window: unknown flag -b",
        ] {
            assert_eq!(
                move_error(
                    TmuxError::Failed(refusal.into()),
                    Some(Side::Before),
                    "next-3.2"
                ),
                "Moving a window before another needs tmux 3.2 or later, and this computer has tmux next-3.2."
            );
        }
        assert_eq!(
            move_error(TmuxError::Failed("index in use: 3".into()), None, "3.1c"),
            INDEX_TAKEN
        );
        assert_eq!(
            move_error(
                TmuxError::Failed("can't find window: @9".into()),
                None,
                "3.6"
            ),
            "There is no window @9."
        );
        assert_eq!(
            move_error(TmuxError::Failed("lost server".into()), None, "3.6"),
            "lost server",
            "anything else is tmux's own words"
        );
        assert_eq!(
            move_error(
                TmuxError::Failed("sessions are grouped".into()),
                Some(Side::After),
                "3.6"
            ),
            "Those two sessions are grouped, so they already share their windows."
        );
    }
}
