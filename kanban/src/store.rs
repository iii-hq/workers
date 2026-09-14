//! The board file: `<data_path>/board.json`, one JSON document holding every
//! ticket (soft-deleted rows included) with its comments and activity inline.
//! Reads are lock-free; every mutation runs read → mutate → atomic write
//! under one mutex so concurrent function calls never interleave on disk.

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Board {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default = "one_u64")]
    pub next_number: u64,
    #[serde(default)]
    pub tickets: IndexMap<String, Ticket>,
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
            version: 1,
            next_number: 1,
            tickets: IndexMap::new(),
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
    let raw = match tokio::fs::read_to_string(file).await {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Board::empty()),
        Err(error) => {
            return Err(format!(
                "cannot read board file {}: {error}",
                file.display()
            ));
        }
    };
    let mut board: Board = serde_json::from_str(&raw)
        .map_err(|error| format!("board file {} is not valid JSON: {error}", file.display()))?;
    board.version = 1;
    if board.next_number < 1 {
        board.next_number = 1;
    }
    Ok(board)
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
        let mut board = read_board(&file).await?;
        let result = mutate(&mut board, &config)?;
        write_board(&file, &board).await?;
        Ok(result)
    }
}
