//! `computer::shell` — run a shell command inside the desktop guest.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ShellInput {
    pub session_id: String,
    /// Command line to run in the guest.
    pub command: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ShellOutput {
    pub stdout: String,
    pub stderr: String,
    /// Process exit code; a non-zero code is returned, not raised.
    pub exit_code: i64,
}
