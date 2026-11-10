use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{register_fn, spec, Ctx, FunctionSpec, KilledOutput};
use crate::tmux_cli::{
    find_window, resolve_cwd, unescape_separators, validate_command, validate_name,
    version_at_least, window_target, TmuxError, WindowArg, WindowRef, SEP,
};

pub const CREATE: &str = "tmux::windows::create";
pub const SELECT: &str = "tmux::windows::select";
pub const RENAME: &str = "tmux::windows::rename";
pub const KILL: &str = "tmux::windows::kill";
pub const MOVE: &str = "tmux::windows::move";

const CREATE_DESC: &str = "Create a window in a tmux session. Optional name, working directory (default: the directory the session was created in), a shell command to run instead of the default shell, and whether to make it the current window (default true).";
const SELECT_DESC: &str = "Make a window the current window of its session. Like tmux itself, every client attached to that session (your terminal included) switches to it.";
const RENAME_DESC: &str = "Rename a tmux window.";
const KILL_DESC: &str = "Kill a tmux window and every program running in its panes.";
const MOVE_DESC: &str = "Move a window to another tmux session, after its last window or just before or after one of its windows (`before` / `after`, which also reorder a window inside its own session). The window keeps its id, panes and programs, and the destination session's current window stays as it was. When it was the last window of its session, tmux closes that session.";

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
    /// Put it just after this window of `target_session` (id or index). Without `before` or `after` it goes after the session's last window.
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
    /// `move-window`'s flag, and the first tmux that has it (tmux's CHANGES:
    /// `-a` came in 2.1, `-b` in 3.2).
    fn flag(self) -> (&'static str, u32, u32) {
        match self {
            Self::Before => ("-b", 3, 2),
            Self::After => ("-a", 2, 1),
        }
    }

    /// The one sentence for a tmux without the flag, `version` when known.
    fn too_old(self, version: Option<&str>) -> String {
        let (_, major, minor) = self.flag();
        let word = match self {
            Self::Before => "before",
            Self::After => "after",
        };
        match version {
            Some(version) => format!(
                "Moving a window {word} another needs tmux {major}.{minor} or later, and this computer has tmux {version}."
            ),
            None => format!("Moving a window {word} another needs tmux {major}.{minor} or later."),
        }
    }
}

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
fn move_error(error: TmuxError, side: Option<Side>, version: Option<&str>) -> String {
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
        _ => message,
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

/// `move-window` keeps the window's id, panes and programs. Its `-t $2:`
/// would take the first free index, which can be a gap between two windows,
/// so a plain move names the index after the last window: the window shows
/// last, where a caller that "moved it to that session" looks for it.
pub async fn move_window(ctx: &Ctx, input: MoveWindowInput) -> Result<MovedWindow, String> {
    let next_to = match (&input.before, &input.after) {
        (Some(_), Some(_)) => return Err("Give before or after, not both.".to_string()),
        (Some(window), None) => Some((Side::Before, WindowRef::parse(window)?)),
        (None, Some(window)) => Some((Side::After, WindowRef::parse(window)?)),
        (None, None) => None,
    };
    let window = WindowRef::parse(&input.window)?;
    let version = ctx.collector.tmux_version().await;
    if let Some((side, _)) = &next_to {
        let (_, major, minor) = side.flag();
        let known = version
            .as_deref()
            .and_then(|version| version_at_least(version, major, minor));
        if known == Some(false) {
            return Err(side.too_old(version.as_deref()));
        }
    }

    let tmux = ctx.tmux().await?;
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
            match target_windows.iter().map(|w| w.index).max() {
                Some(last) => format!("{target_id}:{}", last.saturating_add(1)),
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
            args.push(side.flag().0.into());
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
            version.as_deref(),
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
            Side::Before.too_old(Some("3.1c")),
            "Moving a window before another needs tmux 3.2 or later, and this computer has tmux 3.1c."
        );
        assert_eq!(
            Side::After.too_old(None),
            "Moving a window after another needs tmux 2.1 or later."
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
                    Some("next-3.2")
                ),
                "Moving a window before another needs tmux 3.2 or later, and this computer has tmux next-3.2."
            );
        }
        assert_eq!(
            move_error(TmuxError::Failed("index in use: 3".into()), None, None),
            "index in use: 3",
            "anything else is tmux's own words"
        );
        assert_eq!(
            move_error(
                TmuxError::Failed("sessions are grouped".into()),
                Some(Side::After),
                Some("3.6")
            ),
            "Those two sessions are grouped, so they already share their windows."
        );
    }
}
