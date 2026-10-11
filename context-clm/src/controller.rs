//! Budget controller state.
use serde::{Deserialize, Serialize};

/// Per-session controller state, persisted with the session.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ctl {
    /// Tier fractions already nudged.
    pub nudged: Vec<f64>,
    /// The final turn was granted.
    pub finalized: bool,
    /// Rollbacks so far.
    pub n_retry: u32,
    /// Consecutive rollbacks without an applied edit.
    pub consec: u32,
    /// Messages rolled back so far.
    pub n_rolled: usize,
    /// Commands whose steps were rolled back (for the ledger).
    pub cmds: Vec<String>,
}
