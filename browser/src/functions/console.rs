//! `browser::console::read` — filtered read over the console ring buffer.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::session::ConsoleEntry;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ConsoleReadInput {
    pub session_id: String,
    /// Regex applied to entry text. Use it: dumping an unfiltered console
    /// wastes the caller's context.
    #[serde(default)]
    pub pattern: Option<String>,
    /// Only entries at this level: `log`, `info`, `warning`, `error`,
    /// `debug`, `exception`. `error` also matches `exception`.
    #[serde(default)]
    pub level: Option<String>,
    /// Only entries with `seq` greater than this; resume from the cursor
    /// returned as `last_seq`.
    #[serde(default)]
    pub since_seq: Option<u64>,
    /// Maximum entries returned, newest kept (default 100).
    #[serde(default)]
    pub limit: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ConsoleReadOutput {
    pub entries: Vec<ConsoleEntry>,
    /// Cursor for the next `since_seq`.
    pub last_seq: u64,
    /// Entries evicted from the ring buffer since session start.
    pub dropped: u64,
}

pub const DEFAULT_LIMIT: usize = 100;

pub fn level_matches(want: &str, entry_level: &str) -> bool {
    if want == entry_level {
        return true;
    }
    want == "error" && entry_level == "exception"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_includes_exceptions() {
        assert!(level_matches("error", "error"));
        assert!(level_matches("error", "exception"));
        assert!(!level_matches("warning", "error"));
    }
}
