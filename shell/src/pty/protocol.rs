use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::output_buffer::OutputFrame;
use super::session::SessionStatus;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenRequest {
    #[serde(default)]
    pub request_id: Option<String>,
    pub cwd: String,
    pub cols: u16,
    pub rows: u16,
    pub output_function_id: String,
    /// Program to run instead of the user's login shell — an agent CLI, a
    /// REPL, a TUI. Resolved on the worker's PATH. A caller that can open a
    /// login shell can already run any program by typing it, so this adds
    /// reach, not privilege; it is what lets a session BE one program with
    /// no shell around it.
    #[serde(default)]
    pub program: Option<String>,
    /// argv for `program`. Ignored without `program` (a login shell takes no
    /// arguments here).
    #[serde(default)]
    pub args: Option<Vec<String>>,
    /// Extra environment for this session, on top of what the worker
    /// forwards. Deny-only, like `shell::exec`'s per-call env: every key is
    /// allowed except the exec-hijacking keys (`PATH`, `LD_*`, `DYLD_*`,
    /// `BASH_ENV`, ...), which fail the call.
    #[serde(default)]
    pub env: Option<BTreeMap<String, String>>,
    #[serde(rename = "_caller_worker_id", default)]
    #[schemars(skip)]
    pub caller_worker_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct OpenResponse {
    pub session_id: String,
    pub access_key: String,
    pub reconnect_token: String,
    pub pid: Option<u32>,
    pub cwd: String,
    /// The program the session runs; absent for a login shell.
    pub program: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteRequest {
    pub session_id: String,
    pub access_key: String,
    pub data: String,
    #[serde(rename = "_caller_worker_id", default)]
    #[schemars(skip)]
    pub caller_worker_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WriteResponse {
    pub written: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResizeRequest {
    pub session_id: String,
    pub access_key: String,
    pub cols: u16,
    pub rows: u16,
    #[serde(rename = "_caller_worker_id", default)]
    #[schemars(skip)]
    pub caller_worker_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ResizeResponse {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CloseRequest {
    pub session_id: String,
    pub access_key: String,
    #[serde(rename = "_caller_worker_id", default)]
    #[schemars(skip)]
    pub caller_worker_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CloseResponse {
    pub closed: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AttachRequest {
    #[serde(default)]
    pub request_id: Option<String>,
    pub session_id: String,
    pub reconnect_token: String,
    pub output_function_id: String,
    pub cols: u16,
    pub rows: u16,
    pub after_sequence: u64,
    #[serde(rename = "_caller_worker_id", default)]
    #[schemars(skip)]
    pub caller_worker_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct AttachResponse {
    pub access_key: String,
    pub reconnect_token: String,
    pub frames: Vec<OutputFrame>,
    pub truncated: bool,
    pub next_sequence: u64,
    pub cwd: String,
    pub status: SessionStatus,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DetachRequest {
    pub session_id: String,
    pub access_key: String,
    #[serde(rename = "_caller_worker_id", default)]
    #[schemars(skip)]
    pub caller_worker_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DetachResponse {
    pub status: SessionStatus,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SessionsRequest {}

/// One live session, without credentials: enough to tell a terminal that
/// shows nothing apart from one that was never fed. A page that counts the
/// frames it applied compares its own count against `sequence` — equal
/// means the frames arrived and the browser is at fault, far behind means
/// delivery is.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SessionSummary {
    pub session_id: String,
    pub cwd: String,
    pub program: Option<String>,
    pub pid: Option<u32>,
    pub status: SessionStatus,
    /// Sequence number of the last frame the session produced.
    pub sequence: u64,
    /// Frames still replayable from the ring buffer.
    pub frames: usize,
    pub frame_bytes: usize,
    /// Whether the buffer has dropped frames a full replay would need.
    pub truncated: bool,
    /// Where output is being delivered; absent while detached.
    pub output_function_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SessionsResponse {
    pub sessions: Vec<SessionSummary>,
}
