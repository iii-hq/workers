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
