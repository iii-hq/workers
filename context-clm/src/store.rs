//! Per-session state on disk: `<dir>/<safe(sid)>.json` (atomic, 0600) and the
//! step log `<dir>/log/<safe(sid)>.jsonl`.
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

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
    /// The step in `last_step` ingested only context maintenance (every call
    /// of the reply it answered touched the mirror); its response and its
    /// replays carry `applied.maintenance_step`.
    pub maintenance: bool,
    /// `usable` that context-manager last reported for `last_usable_model`
    /// (0 = none yet).
    pub last_usable: u64,
    /// The model (`provider/id`) `last_usable` and the `last_cm_*` counts
    /// were learned from; under another model they are unknown.
    pub last_usable_model: String,
    /// context-manager's count of the last request CLM sent, less its request
    /// overhead (0 = none yet).
    pub last_cm_tokens: u64,
    /// CLM's raw ruler count of that same request.
    pub last_cm_raw: u64,
    /// The tool schemas' share of `last_cm_tokens` (`breakdown.tools_tokens`).
    pub last_cm_tools: u64,
    /// `request_overhead_tokens` of the step in `last_out`; a replay with another value is recomputed.
    pub last_overhead: u64,
    /// The limit the system section states, fixed on the session's first CLM
    /// step so the section stays byte-stable (0 = not set yet).
    pub section_limit: u64,
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
            maintenance: false,
            last_usable: 0,
            last_usable_model: String::new(),
            last_cm_tokens: 0,
            last_cm_raw: 0,
            last_cm_tools: 0,
            last_overhead: 0,
            section_limit: 0,
            budget: 0,
            stats: Stats::default(),
        }
    }
}

/// Both fields are required: a partial cache must fail the load (the state is
/// set aside and rebuilt), never be replayed as an empty request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

    fn corrupt_path(&self, sid: &str) -> PathBuf {
        self.dir.join(format!("{}.json.corrupt", safe_id(sid)))
    }

    /// Set an unreadable state file aside as `<safe>.json.corrupt`, replacing
    /// an older one; `remove` deletes it with the session.
    pub fn quarantine(&self, sid: &str) -> io::Result<()> {
        fs::rename(self.state_path(sid), self.corrupt_path(sid))
    }

    fn log_path(&self, sid: &str) -> PathBuf {
        self.dir.join("log").join(format!("{}.jsonl", safe_id(sid)))
    }

    /// Whether the session has a state file, readable or not.
    pub fn exists(&self, sid: &str) -> bool {
        self.state_path(sid).exists()
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
        create_private_dir(&self.dir)?;
        write_private(&self.state_path(sid), &serde_json::to_vec(state)?)
    }

    /// Delete the session's state (and a copy set aside) and step log; missing files are fine.
    pub fn remove(&self, sid: &str) -> io::Result<()> {
        remove_if_exists(&self.state_path(sid))?;
        remove_if_exists(&self.corrupt_path(sid))?;
        remove_if_exists(&self.log_path(sid))
    }

    /// Append one JSON line to the session's step log (0600, never through a symlink).
    pub fn append_log(&self, sid: &str, line: &Value) -> io::Result<()> {
        let path = self.log_path(sid);
        create_private_dir(&self.dir.join("log"))?;
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

/// Create a folder (and missing parents) 0700 on unix: state and step logs
/// hold transcript text. An existing folder keeps its mode.
fn create_private_dir(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}
