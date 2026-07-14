//! `computer::files::read` / `write` — read and write text files in the
//! desktop guest.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FilesReadInput {
    pub session_id: String,
    /// Absolute path in the guest filesystem.
    pub path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FilesReadOutput {
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FilesWriteInput {
    pub session_id: String,
    /// Absolute path in the guest filesystem.
    pub path: String,
    /// Text to write, replacing any existing content.
    pub content: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FilesWriteOutput {
    pub ok: bool,
}
