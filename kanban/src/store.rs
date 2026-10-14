//! The board file: `<data_path>/board.json`, one JSON document holding every
//! ticket (soft-deleted rows included) with its comments and activity inline.
//! Reads are lock-free; every mutation runs read → mutate → atomic write
//! under one mutex so concurrent function calls never interleave on disk.
//!
//! The file carries `version`. Version 1 (tickets only) loads as-is and is
//! written back as [`BOARD_VERSION`], after a one-time copy of the original to
//! `board.v1.json` (an older worker would drop the projects, releases and
//! subtask links on its next write); a file written by a newer worker is
//! refused on read, so no mutation ever overwrites it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::KanbanConfig;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Change {
    pub field: String,
    pub from: Value,
    pub to: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Comment {
    pub id: String,
    pub ticket_id: String,
    /// Set for a reply; replies stay one level deep.
    #[serde(default)]
    pub parent_id: Option<String>,
    pub body: String,
    /// Agent profile id, or `user`.
    pub author: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Activity {
    pub id: String,
    pub ticket_id: String,
    pub seq: u64,
    /// ticket.created | ticket.updated | ticket.moved | ticket.deleted | ticket.restored | comment.created
    #[serde(rename = "type")]
    pub kind: String,
    pub actor: String,
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<Vec<Change>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Ticket {
    /// Internal uuid.
    pub id: String,
    /// Human-readable id, e.g. KAN-1.
    pub key: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Column id.
    pub status: String,
    pub priority: String,
    /// Agent profile id.
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    /// Uuid of the parent ticket. One level deep: a parent has no parent.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Uuid of the project (several tickets, independent of the release).
    #[serde(default)]
    pub project_id: Option<String>,
    /// Uuid of the release.
    #[serde(default)]
    pub release_id: Option<String>,
    /// Position inside its column.
    #[serde(default)]
    pub order: u64,
    pub created_at: String,
    pub updated_at: String,
    /// Set when soft-deleted; the row stays on disk.
    #[serde(default)]
    pub deleted_at: Option<String>,
    #[serde(default = "default_actor")]
    pub created_by: String,
    #[serde(default)]
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub activity: Vec<Activity>,
}

pub fn default_actor() -> String {
    "user".to_string()
}

/// A named group of tickets (a product area), independent of releases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Project {
    pub id: String,
    /// Unique among projects (trimmed, ASCII case ignored).
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Agent profile id, or `user`.
    #[serde(default)]
    pub lead: Option<String>,
    /// blue | purple | teal | green | amber | rose.
    #[serde(default)]
    pub color: Option<String>,
    /// The label this project was converted from, if any.
    #[serde(default)]
    pub source_label: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Set when archived; an archived project refuses new tickets.
    #[serde(default)]
    pub archived_at: Option<String>,
}

/// A deliverable that tickets are filed under. Its status is derived from the
/// tickets; only `released_at` is stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Release {
    pub id: String,
    /// Unique among releases (trimmed, ASCII case ignored).
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// `YYYY-MM-DD`.
    #[serde(default)]
    pub target_at: Option<String>,
    #[serde(default)]
    pub released_at: Option<String>,
    /// The label this release was converted from, if any.
    #[serde(default)]
    pub source_label: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// The newest board file version this worker reads and writes.
pub const BOARD_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Board {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default = "one_u64")]
    pub next_number: u64,
    #[serde(default)]
    pub tickets: IndexMap<String, Ticket>,
    #[serde(default)]
    pub projects: IndexMap<String, Project>,
    #[serde(default)]
    pub releases: IndexMap<String, Release>,
}

fn one() -> u32 {
    1
}

fn one_u64() -> u64 {
    1
}

impl Default for Board {
    fn default() -> Self {
        Self {
            version: BOARD_VERSION,
            next_number: 1,
            tickets: IndexMap::new(),
            projects: IndexMap::new(),
            releases: IndexMap::new(),
        }
    }
}

impl Board {
    pub fn empty() -> Self {
        Self::default()
    }
}

/// A missing file is an empty board; anything else that fails is an error
/// the caller surfaces instead of silently starting a new board over a
/// corrupt one.
pub async fn read_board(file: &Path) -> Result<Board, String> {
    read_stored(file).await.map(|(board, _)| board)
}

/// [`read_board`] plus the version the file was stored with (the current one
/// for a missing file).
async fn read_stored(file: &Path) -> Result<(Board, u32), String> {
    let raw = match tokio::fs::read_to_string(file).await {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Board::empty(), BOARD_VERSION));
        }
        Err(error) => {
            return Err(format!(
                "cannot read board file {}: {error}",
                file.display()
            ));
        }
    };
    // Probe the version first: a newer file may not fit this worker's model,
    // and "version 3" is a better error than whatever field failed to parse.
    #[derive(Deserialize)]
    struct Probe {
        version: Option<u32>,
    }
    let probe: Probe = serde_json::from_str(&raw)
        .map_err(|error| format!("board file {} is not valid JSON: {error}", file.display()))?;
    if let Some(version) = probe.version.filter(|version| *version > BOARD_VERSION) {
        return Err(format!(
            "board file {} is version {version}, newer than the version {BOARD_VERSION} this worker understands; update the kanban worker (the file was left untouched)",
            file.display()
        ));
    }
    let mut board: Board = serde_json::from_str(&raw)
        .map_err(|error| format!("board file {} is not valid JSON: {error}", file.display()))?;
    let stored = board.version;
    board.version = BOARD_VERSION;
    if board.next_number < 1 {
        board.next_number = 1;
    }
    Ok((board, stored))
}

/// Keep the file as an older worker wrote it, next to it as `board.v<N>.json`,
/// before the first write that upgrades it. An existing copy is never replaced.
async fn keep_old_version(file: &Path, stored: u32) -> Result<(), String> {
    let copy = file.with_file_name(format!("board.v{stored}.json"));
    if tokio::fs::try_exists(&copy).await.unwrap_or(false) {
        return Ok(());
    }
    // Through a temp file, so a crash never leaves a truncated copy that the next write would trust.
    let temp = PathBuf::from(format!("{}.tmp", copy.display()));
    let failed = |error: std::io::Error| {
        format!(
            "cannot keep a copy of {} as {}: {error}",
            file.display(),
            copy.display()
        )
    };
    tokio::fs::copy(file, &temp).await.map_err(failed)?;
    tokio::fs::rename(&temp, &copy).await.map_err(failed)
}

/// Write to a sibling temp file, then rename: a crash mid-write never leaves
/// a truncated board behind.
pub async fn write_board(file: &Path, board: &Board) -> Result<(), String> {
    if let Some(parent) = file.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let mut body = serde_json::to_string_pretty(board).map_err(|error| error.to_string())?;
    body.push('\n');
    let temp = PathBuf::from(format!(
        "{}.{}.{}.tmp",
        file.display(),
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    ));
    tokio::fs::write(&temp, body)
        .await
        .map_err(|error| format!("cannot write {}: {error}", temp.display()))?;
    tokio::fs::rename(&temp, file)
        .await
        .map_err(|error| format!("cannot replace {}: {error}", file.display()))
}

/// Live configuration shared by every handler; swapped whole on reload.
pub type ConfigCell = Arc<RwLock<KanbanConfig>>;

pub fn config_snapshot(cell: &ConfigCell) -> KanbanConfig {
    cell.read().unwrap_or_else(|p| p.into_inner()).clone()
}

pub struct BoardStore {
    config: ConfigCell,
    write_lock: tokio::sync::Mutex<()>,
}

impl BoardStore {
    pub fn new(config: ConfigCell) -> Self {
        Self {
            config,
            write_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn config(&self) -> KanbanConfig {
        config_snapshot(&self.config)
    }

    pub async fn read(&self) -> Result<Board, String> {
        read_board(&self.config().board_file()).await
    }

    /// Read → mutate → write, serialized. The file is rewritten only when the
    /// mutation succeeds, so a rejected call leaves the board untouched.
    pub async fn update<T>(
        &self,
        mutate: impl FnOnce(&mut Board, &KanbanConfig) -> Result<T, String>,
    ) -> Result<T, String> {
        let _serialized = self.write_lock.lock().await;
        let config = self.config();
        let file = config.board_file();
        let (mut board, stored) = read_stored(&file).await?;
        let result = mutate(&mut board, &config)?;
        if stored < BOARD_VERSION {
            keep_old_version(&file, stored).await?;
        }
        write_board(&file, &board).await?;
        Ok(result)
    }
}
