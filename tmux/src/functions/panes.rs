use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{register_fn, spec, Ctx, FunctionSpec};
use crate::tmux_cli::{is_pane_id, Tmux, SEP};

pub const SELECT: &str = "tmux::panes::select";
pub const CAPTURE: &str = "tmux::panes::capture";

const SELECT_DESC: &str = "Make a pane the active pane of its window, optionally zooming the window on it (zoom: true, like tmux's prefix z) or unzooming it (zoom: false); without zoom the window keeps its zoom state. Like tmux itself, every client looking at that window follows, your terminal included. It does not change the session's current window (tmux::windows::select does).";
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

fn pane_arg(raw: &str) -> Result<&str, String> {
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
    let line = output.lines().next().unwrap_or_default();
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

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<SelectPaneInput, SelectedPane>(SELECT, SELECT_DESC),
        spec::<PaneInput, PaneCapture>(CAPTURE, CAPTURE_DESC),
    ]
}

pub fn register(iii: &IIIClient, ctx: &Arc<Ctx>) {
    register_fn!(iii, ctx, SELECT, SELECT_DESC, SelectPaneInput, select);
    register_fn!(iii, ctx, CAPTURE, CAPTURE_DESC, PaneInput, capture);
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
        assert!(parse_selected("").is_err());
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
        // A screen line that happens to look like a header is still text.
        let tricky = parse_capture("%4", &format!("{header}\n{header}\n")).unwrap();
        assert_eq!(tricky.text, header);
        assert!(parse_capture("%4", "garbage").is_err());
    }
}
