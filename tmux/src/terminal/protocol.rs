//! Request and response shapes of `tmux::terminal::*`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use super::output_buffer::{OutputFrame, Replay};
use crate::tmux_cli::WindowArg;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct OpenRequest {
    /// Session id (`$5`) or exact name to attach to.
    pub session: String,
    /// Window id (`@3`) or index to select before attaching.
    #[serde(default)]
    pub window: Option<WindowArg>,
    /// Terminal width in columns (2..=500).
    pub cols: u16,
    /// Terminal height in rows (2..=500).
    pub rows: u16,
    /// Browser handler receiving output: `iii::tmux-ui::pty-output::console-<browser id>`.
    pub output_function_id: String,
    #[serde(rename = "_caller_worker_id", default)]
    #[schemars(skip)]
    pub caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct OpenResponse {
    /// Terminal handle for the other calls.
    pub terminal_id: String,
    /// Short-lived credential required by every other call. Never persist it.
    pub access_key: String,
    /// Session the client attached to.
    pub session_id: String,
    /// The tmux client's tty, as `list-clients` shows it.
    pub client_tty: Option<String>,
}

/// Identifies one terminal and proves the caller opened it.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TerminalRef {
    /// Terminal handle from `open`.
    pub terminal_id: String,
    /// Credential from `open`.
    pub access_key: String,
    #[serde(rename = "_caller_worker_id", default)]
    #[schemars(skip)]
    pub caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct WriteRequest {
    #[serde(flatten)]
    pub terminal: TerminalRef,
    /// Base64 keyboard input, at most 64 KiB decoded.
    pub data: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct WriteResponse {
    /// Bytes written to the tmux client.
    pub written: usize,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ResizeRequest {
    #[serde(flatten)]
    pub terminal: TerminalRef,
    /// Width in columns (2..=500).
    pub cols: u16,
    /// Height in rows (2..=500).
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ResizeResponse {
    /// Width applied.
    pub cols: u16,
    /// Height applied.
    pub rows: u16,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SwitchRequest {
    #[serde(flatten)]
    pub terminal: TerminalRef,
    /// Session id (`$5`) or exact name to move this client to.
    pub session: String,
    /// Window id (`@3`) or index to select there.
    #[serde(default)]
    pub window: Option<WindowArg>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SwitchResponse {
    /// Session the client now shows.
    pub session_id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ReplayRequest {
    #[serde(flatten)]
    pub terminal: TerminalRef,
    /// Last sequence the page applied; frames after it are returned.
    pub after_sequence: u64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct OkResponse {
    /// Always true.
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CloseResponse {
    /// False when the terminal was already gone.
    pub closed: bool,
}

/// Pushed to the page's output handler.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct OutputEvent {
    /// Terminal the bytes belong to.
    pub terminal_id: String,
    /// 1, 2, 3… per terminal; null on the final eof event.
    pub sequence: Option<u64>,
    /// Base64 output bytes.
    pub data: Option<String>,
    /// The tmux client exited.
    pub eof: bool,
    /// Its exit code, on eof.
    pub exit_code: Option<u32>,
    /// What went wrong, on eof.
    pub error: Option<String>,
}
