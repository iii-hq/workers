//! `browser::downloads::list` / `browser::download` / `browser::download::remove`
//! — the files a session downloaded, the way a browser's downloads panel
//! shows them. Files land guid-named in the session's download dir; `download`
//! returns one file's bytes for saving or attaching to the chat.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::session::DownloadRecord;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DownloadsListInput {
    pub session_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DownloadsListOutput {
    /// Newest first.
    pub downloads: Vec<DownloadRecord>,
}

/// Upper bound on a download returned inline: the bytes travel base64 over
/// the bus, so anything larger stays on disk and the caller is told where.
pub const MAX_DOWNLOAD_BYTES: u64 = 25 * 1024 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DownloadInput {
    pub session_id: String,
    /// The download's CDP guid, from `browser::downloads::list`.
    pub guid: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DownloadOutput {
    pub ok: bool,
    /// The file, base64.
    pub data: String,
    pub file_name: String,
    pub size_bytes: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DownloadRemoveInput {
    pub session_id: String,
    /// The download to forget, and delete from disk.
    pub guid: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DownloadRemoveOutput {
    pub ok: bool,
}
