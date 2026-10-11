//! Per-session state on disk: `<dir>/<safe(sid)>.json` (atomic, 0600) and the
//! step log `<dir>/log/<safe(sid)>.jsonl`.
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::controller::Ctl;
use crate::mirror::{remove_if_exists, safe_id, write_private};
use crate::types::Msg;

/// Everything `assemble` needs to resume a session after a restart.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionState {
    /// The task (first raw message), never editable.
    pub raw0: Option<Msg>,
    /// Raw messages already ingested into the projection.
    pub covered: usize,
    /// Hash of the covered raw prefix; a mismatch means the history was rewritten.
    pub prefix_hash: String,
    /// Rollback ledger, sent right after the task.
    pub ledger: Option<Msg>,
    /// The projection: what the model sees after the task (and ledger).
    pub proj: Vec<Msg>,
    /// Mirror text as last written; `None` before the first write.
    pub written: Option<String>,
    pub ctl: Ctl,
    /// Calibration: provider prompt tokens / raw count, in [0.5, 3.0].
    pub ratio: f64,
    /// Raw count of the last request sent.
    pub raw_sent: u64,
    /// Projection index after the last applied edit.
    pub edit_mark: usize,
    /// Per-message hashes of the last outgoing list.
    pub sent_hashes: Vec<String>,
    /// `(turn_id, step)` last served (idempotent replays).
    pub last_step: Option<(String, u64)>,
    /// The outgoing request last served, resent on a replay.
    pub last_out: Option<CachedOut>,
    /// Filesystem root the mirror was last written under.
    pub root: Option<String>,
    /// The run was ended by the controller.
    pub stopped: bool,
    /// Effective budget of the last step, in tokens (the session's
    /// `context_budget_tokens`, else the worker default, lowered to the
    /// model's usable room); 0 before any step.
    pub budget: u64,
    pub stats: Stats,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            raw0: None,
            covered: 0,
            prefix_hash: String::new(),
            ledger: None,
            proj: Vec::new(),
            written: None,
            ctl: Ctl::default(),
            ratio: 1.0,
            raw_sent: 0,
            edit_mark: 0,
            sent_hashes: Vec::new(),
            last_step: None,
            last_out: None,
            root: None,
            stopped: false,
            budget: 0,
            stats: Stats::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CachedOut {
    pub messages: Vec<Msg>,
    pub system_prompt: String,
}

/// Cumulative per-session counters (`context-clm::status`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Stats {
    pub steps: u64,
    pub edits_applied: u64,
    pub edits_rejected: u64,
    pub rollbacks: u64,
    pub stops: u64,
    /// Highest calibrated context size seen, in tokens.
    pub peak_tc: u64,
    /// Thinking blocks stripped.
    pub stripped: u64,
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// `dir` is already resolved (`iii_worker_paths::resolve_path`).
    pub fn new(dir: PathBuf) -> Store {
        Store { dir }
    }

    fn state_path(&self, sid: &str) -> PathBuf {
        self.dir.join(format!("{}.json", safe_id(sid)))
    }

    fn log_path(&self, sid: &str) -> PathBuf {
        self.dir.join("log").join(format!("{}.jsonl", safe_id(sid)))
    }

    pub fn load(&self, sid: &str) -> io::Result<Option<SessionState>> {
        match fs::read(self.state_path(sid)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn save(&self, sid: &str, state: &SessionState) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        write_private(&self.state_path(sid), &serde_json::to_vec(state)?)
    }

    /// Delete the session's state and step log; missing files are fine.
    pub fn remove(&self, sid: &str) -> io::Result<()> {
        remove_if_exists(&self.state_path(sid))?;
        remove_if_exists(&self.log_path(sid))
    }

    /// Append one JSON line to the session's step log (0600, never through a symlink).
    pub fn append_log(&self, sid: &str, line: &Value) -> io::Result<()> {
        let path = self.log_path(sid);
        fs::create_dir_all(self.dir.join("log"))?;
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut bytes = serde_json::to_vec(line)?;
        bytes.push(b'\n');
        options.open(path)?.write_all(&bytes)
    }
}
