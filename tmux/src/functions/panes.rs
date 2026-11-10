use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{register_fn, spec, Ctx, FunctionSpec};
use crate::tmux_cli::{
    format_literal, is_pane_id, resolve_cwd, unescape_separators, validate_command, Tmux,
    TmuxError, SEP,
};

pub const SELECT: &str = "tmux::panes::select";
pub const CAPTURE: &str = "tmux::panes::capture";
pub const SPLIT: &str = "tmux::panes::split";
pub const KILL: &str = "tmux::panes::kill";
pub const RESPAWN: &str = "tmux::panes::respawn";

const SELECT_DESC: &str = "Make a pane the active pane of its window. zoom sets the window's zoom, it never toggles: true zooms the window on this pane (or keeps it zoomed there), false unzooms it, and without zoom the window keeps its zoom state, moved to this pane. A window with one pane can't zoom. Like tmux itself, every client looking at that window follows, your terminal included. It does not change the session's current window (tmux::windows::select does).";
const SPLIT_DESC: &str = "Split a pane in two: the new pane goes to its right or below it, starts in that pane's current directory (or cwd) and runs the default shell (or command). It becomes the window's active pane unless select is false. Like tmux, splitting a zoomed window unzooms it.";
const KILL_DESC: &str = "Kill a pane and the program running in it. Killing a window's last pane closes the window, and its session's last window closes the session; the answer says which closed.";
const RESPAWN_DESC: &str = "Start a pane's program again in the same pane: the command it was started with (or command), in its starting directory (or cwd). A pane whose program is still running is refused unless kill is true, which ends that program first.";
const CAPTURE_DESC: &str = "The visible screen of one pane as text with its colour and style escapes (capture-pane -e), plus the pane's size, cursor position and whether a full-screen program has the alternate screen. Read-only.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SelectPaneInput {
    /// Pane id, e.g. `%5`.
    pub pane: String,
    /// true zooms the window on this pane, false unzooms it, absent keeps the window's zoom.
    #[serde(default)]
    pub zoom: Option<bool>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SelectedPane {
    /// Session id the pane was resolved in.
    pub session_id: String,
    /// Window id holding the pane.
    pub window_id: String,
    /// Pane id now active in its window.
    pub pane_id: String,
    /// The window is zoomed on the active pane.
    pub zoomed: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PaneInput {
    /// Pane id, e.g. `%5`.
    pub pane: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PaneCapture {
    /// Pane id.
    pub pane_id: String,
    /// Width in columns.
    pub width: u32,
    /// Height in rows.
    pub height: u32,
    /// Cursor column, from 0.
    pub cursor_x: u32,
    /// Cursor row, from 0.
    pub cursor_y: u32,
    /// The pane shows its alternate screen (a full-screen program).
    pub alternate: bool,
    /// The visible screen, one line per row separated by `\n`, with SGR escapes.
    pub text: String,
}

/// Where a split puts the new pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SplitDirection {
    /// To the right of the pane (tmux `split-window -h`).
    Right,
    /// Below the pane (tmux `split-window -v`).
    Below,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SplitPaneInput {
    /// Pane id to split, e.g. `%5`.
    pub pane: String,
    /// Where the new pane goes: `right` or `below`.
    pub direction: SplitDirection,
    /// Working directory, absolute or `~/…`. Defaults to the split pane's current directory.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Shell command to run instead of the default shell.
    #[serde(default)]
    pub command: Option<String>,
    /// Make the new pane the window's active pane (default true).
    #[serde(default)]
    pub select: Option<bool>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct NewPane {
    /// Session id.
    pub session_id: String,
    /// Window id holding both panes.
    pub window_id: String,
    /// The new pane's id.
    pub pane_id: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct KilledPane {
    /// Always true.
    pub killed: bool,
    /// It was its window's last pane, so the window closed.
    pub window_closed: bool,
    /// That window was its session's last, so the session closed.
    pub session_closed: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RespawnPaneInput {
    /// Pane id, e.g. `%5`.
    pub pane: String,
    /// Shell command to run instead of the one the pane was started with.
    #[serde(default)]
    pub command: Option<String>,
    /// Working directory, absolute or `~/…`. Defaults to the pane's starting directory.
    #[serde(default)]
    pub cwd: Option<String>,
    /// End the pane's program first when it is still running (default false).
    #[serde(default)]
    pub kill: Option<bool>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RespawnedPane {
    /// Pane id, the same as before.
    pub pane_id: String,
}

const STILL_RUNNING: &str = "The pane's program is still running; pass kill to restart it.";
const NO_ROOM: &str = "The pane is too small to split.";

pub(super) fn pane_arg(raw: &str) -> Result<&str, String> {
    let pane = raw.trim();
    if is_pane_id(pane) {
        Ok(pane)
    } else {
        Err(format!("pane must be an id like %5, got '{raw}'"))
    }
}

fn field<T: std::str::FromStr + Default>(value: &str) -> T {
    value.trim().parse().unwrap_or_default()
}

const SELECTED_FORMAT: [&str; 4] = [
    "#{session_id}",
    "#{window_id}",
    "#{pane_id}",
    "#{window_zoomed_flag}",
];

pub fn parse_selected(output: &str) -> Result<SelectedPane, String> {
    let line = unescape_separators(output.lines().next().unwrap_or_default());
    let fields: Vec<&str> = line.split(SEP).collect();
    if fields.len() != 4 {
        return Err(format!("unexpected tmux output: {line:?}"));
    }
    Ok(SelectedPane {
        session_id: fields[0].to_string(),
        window_id: fields[1].to_string(),
        pane_id: fields[2].to_string(),
        zoomed: fields[3].trim() == "1",
    })
}

const CAPTURE_FORMAT: [&str; 5] = [
    "#{pane_width}",
    "#{pane_height}",
    "#{cursor_x}",
    "#{cursor_y}",
    "#{alternate_on}",
];

/// `display-message` first (one line of numbers), then `capture-pane`, so
/// whatever the screen holds can never be mistaken for the header.
pub fn parse_capture(pane_id: &str, output: &str) -> Result<PaneCapture, String> {
    let (header, text) = output.split_once('\n').unwrap_or((output, ""));
    let header = unescape_separators(header);
    let fields: Vec<&str> = header.split(SEP).collect();
    if fields.len() != 5 {
        return Err(format!("unexpected tmux output: {header:?}"));
    }
    Ok(PaneCapture {
        pane_id: pane_id.to_string(),
        width: field(fields[0]),
        height: field(fields[1]),
        cursor_x: field(fields[2]),
        cursor_y: field(fields[3]),
        alternate: fields[4].trim() == "1",
        text: text.strip_suffix('\n').unwrap_or(text).to_string(),
    })
}

async fn describe(tmux: &Tmux, pane: &str) -> Result<SelectedPane, String> {
    let format = SELECTED_FORMAT.join(&SEP.to_string());
    let output = tmux
        .run(["display-message", "-p", "-t", pane, format.as_str()])
        .await
        .map_err(|error| error.to_string())?;
    parse_selected(&output)
}

pub async fn select(ctx: &Ctx, input: SelectPaneInput) -> Result<SelectedPane, String> {
    let pane = pane_arg(&input.pane)?;
    let tmux = ctx.tmux().await?;
    // Fails with tmux's own "can't find pane" before anything changes.
    let before = describe(&tmux, pane).await?;
    let run = |args: Vec<&str>| {
        let tmux = tmux.clone();
        let args: Vec<String> = args.into_iter().map(str::to_string).collect();
        async move {
            tmux.run(&args)
                .await
                .map(drop)
                .map_err(|error| error.to_string())
        }
    };
    match input.zoom {
        // `-Z` moves an existing zoom to the new pane instead of dropping it.
        None => run(vec!["select-pane", "-Z", "-t", pane]).await?,
        Some(true) if before.zoomed => run(vec!["select-pane", "-Z", "-t", pane]).await?,
        Some(true) => {
            run(vec!["select-pane", "-t", pane]).await?;
            run(vec!["resize-pane", "-Z", "-t", pane]).await?
        }
        Some(false) => {
            run(vec!["select-pane", "-t", pane]).await?;
            if describe(&tmux, pane).await?.zoomed {
                run(vec!["resize-pane", "-Z", "-t", pane]).await?;
            }
        }
    }
    let after = describe(&tmux, pane).await?;
    ctx.monitor.poke();
    Ok(after)
}

pub async fn capture(ctx: &Ctx, input: PaneInput) -> Result<PaneCapture, String> {
    let pane = pane_arg(&input.pane)?;
    let tmux = ctx.tmux().await?;
    let format = CAPTURE_FORMAT.join(&SEP.to_string());
    let output = tmux
        .run([
            "display-message",
            "-p",
            "-t",
            pane,
            format.as_str(),
            ";",
            "capture-pane",
            "-p",
            "-e",
            "-t",
            pane,
        ])
        .await
        .map_err(|error| error.to_string())?;
    parse_capture(pane, &output)
}

pub fn parse_new_pane(output: &str) -> Result<NewPane, String> {
    let line = unescape_separators(output.lines().next().unwrap_or_default());
    let fields: Vec<&str> = line.split(SEP).collect();
    if fields.len() != 3 {
        return Err(format!("unexpected tmux output: {line:?}"));
    }
    Ok(NewPane {
        session_id: fields[0].to_string(),
        window_id: fields[1].to_string(),
        pane_id: fields[2].to_string(),
    })
}

/// `#{window_panes}␟#{session_windows}` as read just before a kill.
pub fn parse_kill_counts(output: &str) -> Result<(u32, u32), String> {
    let line = unescape_separators(output.lines().next().unwrap_or_default());
    let fields: Vec<&str> = line.split(SEP).collect();
    match fields.as_slice() {
        [panes, windows] => match (panes.trim().parse(), windows.trim().parse()) {
            (Ok(panes), Ok(windows)) => Ok((panes, windows)),
            _ => Err(format!("unexpected tmux output: {line:?}")),
        },
        _ => Err(format!("unexpected tmux output: {line:?}")),
    }
}

/// tmux refusing a split for want of room, in one sentence.
fn split_error(error: TmuxError) -> String {
    let message = error.to_string();
    if message.contains("no space for new pane") || message.contains("pane too small") {
        NO_ROOM.to_string()
    } else {
        message
    }
}

/// tmux expands `-c` in the session's context, so `#{pane_current_path}`
/// there would be the current window's active pane, not this one: the
/// pane's own directory is read first and passed as text.
pub async fn split(ctx: &Ctx, input: SplitPaneInput) -> Result<NewPane, String> {
    let pane = pane_arg(&input.pane)?;
    let cwd = input.cwd.as_deref().map(resolve_cwd).transpose()?;
    if let Some(command) = input.command.as_deref() {
        validate_command(command)?;
    }
    let tmux = ctx.tmux().await?;
    let start = match cwd {
        Some(cwd) => format_literal(&cwd),
        None => {
            // Also tmux's own "can't find pane" before anything changes.
            let here = tmux
                .run(["display-message", "-p", "-t", pane, "#{pane_current_path}"])
                .await
                .map_err(|error| error.to_string())?;
            match here.trim_end_matches('\n') {
                "" => "#{session_path}".to_string(),
                here => format_literal(here),
            }
        }
    };
    let format = ["#{session_id}", "#{window_id}", "#{pane_id}"].join(&SEP.to_string());
    let direction = match input.direction {
        SplitDirection::Right => "-h",
        SplitDirection::Below => "-v",
    };
    let mut args: Vec<String> = vec![
        "split-window".into(),
        direction.into(),
        "-P".into(),
        "-F".into(),
        format,
    ];
    if input.select == Some(false) {
        args.push("-d".into());
    }
    args.extend(["-t".into(), pane.into(), "-c".into(), start]);
    if let Some(command) = input.command.as_deref() {
        args.extend(["--".into(), command.to_string()]);
    }
    let output = tmux.run(&args).await.map_err(split_error)?;
    ctx.monitor.poke();
    parse_new_pane(&output)
}

pub async fn kill(ctx: &Ctx, input: PaneInput) -> Result<KilledPane, String> {
    let pane = pane_arg(&input.pane)?;
    let tmux = ctx.tmux().await?;
    let counts = ["#{window_panes}", "#{session_windows}"].join(&SEP.to_string());
    // Counted in the same tmux command as the kill, so nothing changes in
    // between; a missing pane stops it at tmux's "can't find pane".
    let output = tmux
        .run([
            "display-message",
            "-p",
            "-t",
            pane,
            counts.as_str(),
            ";",
            "kill-pane",
            "-t",
            pane,
        ])
        .await
        .map_err(|error| error.to_string())?;
    ctx.monitor.poke();
    let (panes, windows) = parse_kill_counts(&output)?;
    let window_closed = panes <= 1;
    Ok(KilledPane {
        killed: true,
        window_closed,
        session_closed: window_closed && windows <= 1,
    })
}

pub async fn respawn(ctx: &Ctx, input: RespawnPaneInput) -> Result<RespawnedPane, String> {
    let pane = pane_arg(&input.pane)?;
    let cwd = input.cwd.as_deref().map(resolve_cwd).transpose()?;
    if let Some(command) = input.command.as_deref() {
        validate_command(command)?;
    }
    let kill = input.kill.unwrap_or(false);
    let tmux = ctx.tmux().await?;
    let dead = tmux
        .run(["display-message", "-p", "-t", pane, "#{pane_dead}"])
        .await
        .map_err(|error| error.to_string())?;
    if !kill && dead.trim() != "1" {
        return Err(STILL_RUNNING.to_string());
    }
    let mut args: Vec<String> = vec!["respawn-pane".into()];
    if kill {
        args.push("-k".into());
    }
    args.extend(["-t".into(), pane.into()]);
    if let Some(cwd) = cwd {
        args.extend(["-c".into(), format_literal(&cwd)]);
    }
    if let Some(command) = input.command.as_deref() {
        args.extend(["--".into(), command.to_string()]);
    }
    tmux.run(&args).await.map_err(|error| {
        let message = error.to_string();
        if message.contains("still active") {
            STILL_RUNNING.to_string()
        } else {
            message
        }
    })?;
    ctx.monitor.poke();
    Ok(RespawnedPane {
        pane_id: pane.to_string(),
    })
}

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<SelectPaneInput, SelectedPane>(SELECT, SELECT_DESC),
        spec::<PaneInput, PaneCapture>(CAPTURE, CAPTURE_DESC),
        spec::<SplitPaneInput, NewPane>(SPLIT, SPLIT_DESC),
        spec::<PaneInput, KilledPane>(KILL, KILL_DESC),
        spec::<RespawnPaneInput, RespawnedPane>(RESPAWN, RESPAWN_DESC),
    ]
}

pub fn register(iii: &IIIClient, ctx: &Arc<Ctx>) {
    register_fn!(iii, ctx, SELECT, SELECT_DESC, SelectPaneInput, select);
    register_fn!(iii, ctx, CAPTURE, CAPTURE_DESC, PaneInput, capture);
    register_fn!(iii, ctx, SPLIT, SPLIT_DESC, SplitPaneInput, split);
    register_fn!(iii, ctx, KILL, KILL_DESC, PaneInput, kill);
    register_fn!(iii, ctx, RESPAWN, RESPAWN_DESC, RespawnPaneInput, respawn);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_ids_are_required() {
        assert_eq!(pane_arg(" %12 ").unwrap(), "%12");
        assert!(pane_arg("12").is_err());
        assert!(pane_arg("@3").is_err());
        assert!(pane_arg("%1 ; kill-server").is_err());
    }

    #[test]
    fn selected_output_parses() {
        let line = ["$3", "@8", "%11", "1"].join(&SEP.to_string());
        let selected = parse_selected(&format!("{line}\n")).unwrap();
        assert_eq!(
            (
                selected.session_id.as_str(),
                selected.window_id.as_str(),
                selected.pane_id.as_str(),
                selected.zoomed
            ),
            ("$3", "@8", "%11", true)
        );
        assert!(parse_selected("$3\\037@8\\037%11\\0370\n").is_ok());
        assert!(parse_selected("").is_err());
    }

    #[test]
    fn split_and_kill_outputs_parse() {
        let line = ["$3", "@8", "%12"].join(&SEP.to_string());
        let pane = parse_new_pane(&format!("{line}\n")).unwrap();
        assert_eq!(
            (
                pane.session_id.as_str(),
                pane.window_id.as_str(),
                pane.pane_id.as_str()
            ),
            ("$3", "@8", "%12")
        );
        assert!(parse_new_pane("$3\\037@8\\037%12\n").is_ok());
        assert!(parse_new_pane("").is_err());
        assert_eq!(
            parse_kill_counts(&format!("2{SEP}1\n")).unwrap(),
            (2, 1),
            "the kill itself prints nothing"
        );
        assert_eq!(parse_kill_counts("1\\0373\n").unwrap(), (1, 3));
        assert!(parse_kill_counts("x\n").is_err());
        assert!(parse_kill_counts(&format!("a{SEP}1")).is_err());
    }

    #[test]
    fn directions_are_right_and_below() {
        let parse =
            |value: &str| serde_json::from_value::<SplitDirection>(serde_json::json!(value));
        assert_eq!(parse("right").unwrap(), SplitDirection::Right);
        assert_eq!(parse("below").unwrap(), SplitDirection::Below);
        assert!(parse("left").is_err());
        assert!(parse("Right").is_err());
    }

    #[test]
    fn no_room_to_split_is_one_sentence() {
        // tmux 3.x, then older ones.
        for refusal in [
            "no space for new pane",
            "create pane failed: pane too small",
        ] {
            assert_eq!(split_error(TmuxError::Failed(refusal.into())), NO_ROOM);
        }
        assert_eq!(
            split_error(TmuxError::Failed("can't find pane: %9".into())),
            "can't find pane: %9"
        );
    }

    #[test]
    fn capture_keeps_the_screen_as_tmux_printed_it() {
        let header = ["80", "24", "3", "1", "0"].join(&SEP.to_string());
        let screen = "\x1b[1mhello\x1b[0m\n$ ";
        let capture = parse_capture("%4", &format!("{header}\n{screen}\n")).unwrap();
        assert_eq!(
            (
                capture.width,
                capture.height,
                capture.cursor_x,
                capture.cursor_y
            ),
            (80, 24, 3, 1)
        );
        assert!(!capture.alternate);
        assert_eq!(capture.text, screen);
        let escaped = header.replace(SEP, "\\037");
        let capture = parse_capture("%4", &format!("{escaped}\n{screen}\n")).unwrap();
        assert_eq!((capture.width, capture.height), (80, 24));
        assert_eq!(capture.text, screen);
        // A screen line that happens to look like a header is still text.
        let tricky = parse_capture("%4", &format!("{header}\n{header}\n")).unwrap();
        assert_eq!(tricky.text, header);
        assert!(parse_capture("%4", "garbage").is_err());
    }
}
