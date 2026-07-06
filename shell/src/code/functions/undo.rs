//! `coder::undo` — revert journaled coder writes: the last N records or
//! every record a specific turn produced. Restores run newest-first; every
//! restored path re-validates through the LIVE jail (journal data is never
//! trusted as authority to write). The undo journals its own pre-undo state
//! as an ordinary record first, so undoing an undo redoes the change.

use std::path::Path;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::code::config::CoderConfig;
use crate::code::error::{err_to_string, CoderError};
use crate::code::functions::update_file::atomic_write;
use crate::code::journal::{self, JournalRecord};
use crate::code::path::PathResolver;

// examples are wire-contract; goldens pin them.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[schemars(example = "example_undo_input")]
pub struct UndoInput {
    /// Undo the last N journaled records (default 1). Mutually exclusive
    /// with `turn_id`.
    #[serde(default)]
    pub steps: Option<u32>,
    /// Undo every journaled record this turn produced (see
    /// `coder::checkpoints` for turn ids). Mutually exclusive with `steps`.
    #[serde(default)]
    pub turn_id: Option<String>,
    /// Internal harness filesystem scope; omitted from published schema.
    #[serde(default)]
    #[schemars(skip)]
    pub fs_scope: Option<crate::fs::FsScope>,
}

// examples are wire-contract; goldens pin them.
fn example_undo_input() -> serde_json::Value {
    serde_json::json!({ "steps": 1 })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UndoOutput {
    /// Undone records, newest first. The undo itself was journaled before
    /// any restore, so `coder::undo { steps: 1 }` again redoes the change.
    pub undone: Vec<UndoneRecord>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UndoneRecord {
    pub seq: u64,
    /// The mutation this record captured (e.g. "coder::update-file").
    pub function_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// Files rewritten to their journaled before-image.
    pub restored: Vec<String>,
    /// Files removed (they did not exist before the journaled write).
    pub removed: Vec<String>,
    /// Unrecoverable gaps: oversized before-images, directory operations,
    /// or paths the live jail rejected — inspect these manually.
    pub skipped: Vec<String>,
}

pub async fn handle(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    req: UndoInput,
) -> Result<UndoOutput, String> {
    if req.steps.is_some() && req.turn_id.is_some() {
        return Err(err_to_string(CoderError::BadInput(
            "pass either `steps` or `turn_id`, not both".into(),
        )));
    }
    if !journal::enabled(&cfg) {
        return Err(err_to_string(CoderError::BadInput(
            "the coder write journal is disabled (journal.max_records = 0)".into(),
        )));
    }
    let scope_root = crate::fs::scope_root(req.fs_scope.as_ref()).map(str::to_string);
    tokio::task::spawn_blocking(move || {
        inner(&resolver, &cfg, scope_root.as_deref(), &req).map_err(err_to_string)
    })
    .await
    .map_err(|e| format!("undo task join failed: {e}"))?
}

fn inner(
    resolver: &PathResolver,
    cfg: &CoderConfig,
    scope_root: Option<&str>,
    req: &UndoInput,
) -> Result<UndoOutput, CoderError> {
    let root = resolver.effective_root(scope_root);
    let mut records = journal::list(cfg, &root);
    // Newest first.
    records.reverse();

    let selected: Vec<JournalRecord> = match (&req.steps, &req.turn_id) {
        (_, Some(turn)) => records
            .into_iter()
            .filter(|r| r.turn_id.as_deref() == Some(turn.as_str()))
            .collect(),
        (steps, None) => {
            let n = steps.unwrap_or(1).max(1) as usize;
            records.into_iter().take(n).collect()
        }
    };
    if selected.is_empty() {
        return Err(CoderError::BadInput(
            "nothing to undo — no matching journal records for this \
             workspace (see coder::checkpoints)"
                .into(),
        ));
    }

    let mut undone = Vec::with_capacity(selected.len());
    for record in &selected {
        undone.push(undo_record(resolver, cfg, scope_root, &root, record)?);
        journal::remove_record(cfg, &root, record);
    }
    Ok(UndoOutput { undone })
}

fn undo_record(
    resolver: &PathResolver,
    cfg: &CoderConfig,
    scope_root: Option<&str>,
    root: &Path,
    record: &JournalRecord,
) -> Result<UndoneRecord, CoderError> {
    // Journal the PRE-undo state first (redo path). Skipped entries stay
    // skipped — there is nothing to restore either way.
    let mut redo_entries = Vec::new();
    for entry in &record.entries {
        if entry.skipped {
            continue;
        }
        let path = std::path::PathBuf::from(&entry.path);
        redo_entries.push(journal::EntryInput {
            before: std::fs::read(&path).ok(),
            path,
            skipped: false,
        });
    }
    journal::record(cfg, root, None, "coder::undo", redo_entries);

    let mut restored = Vec::new();
    let mut removed = Vec::new();
    let mut skipped = Vec::new();
    for entry in &record.entries {
        if entry.skipped {
            skipped.push(entry.path.clone());
            continue;
        }
        // Re-jail: the LIVE resolver decides whether this path is writable
        // now; a stale or tampered record must not write outside the jail.
        let abs = match resolver.require_writable_opt(scope_root, &entry.path) {
            Ok(abs) => abs,
            Err(_) => {
                skipped.push(entry.path.clone());
                continue;
            }
        };
        match &entry.blob {
            Some(blob) => {
                let bytes = journal::read_blob(cfg, root, blob)
                    .map_err(|e| CoderError::Io(format!("journal blob unreadable: {e}")))?;
                if let Some(parent) = abs.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| CoderError::io_for_path(e, &entry.path))?;
                }
                atomic_write(&abs, &bytes)?;
                restored.push(entry.path.clone());
            }
            None => {
                // The file did not exist before the journaled write.
                match std::fs::remove_file(&abs) {
                    Ok(()) => removed.push(entry.path.clone()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        removed.push(entry.path.clone())
                    }
                    Err(e) => return Err(CoderError::io_for_path(e, &entry.path)),
                }
            }
        }
    }
    Ok(UndoneRecord {
        seq: record.seq,
        function_id: record.function_id.clone(),
        turn_id: record.turn_id.clone(),
        restored,
        removed,
        skipped,
    })
}
