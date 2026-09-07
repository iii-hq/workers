//! Per-verb function modules and the single `register_all` entry point.
//! `main.rs` calls `register_all` after `register_worker`; each
//! `register_<verb>` below uses `RegisterFunction::new_async` with typed
//! `JsonSchema` request/response structs so the SDK can emit schemas for
//! tools and docs (docs/sops/binary-worker.md §7).
//!
//! WIRE-SURFACE CATALOG — `catalog()` below is the single source of truth
//! for every function's id + registration description, plus the
//! schemars-derived request/response schemas. The golden test
//! `tests/code_golden_schemas.rs` snapshots each entry so ANY change to the
//! agent-facing wire surface shows up as an explicit, reviewed diff.

pub mod create_file;
pub mod delete_file;
pub mod info;
pub mod list_folder;
pub mod move_file;
pub mod read_file;
pub mod read_window;
pub mod search;
pub mod tree;
pub mod update_file;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};

use crate::code::change_journal::{self, ChangeDiffInput};
use crate::code::state::CodeCells;

/// Tolerant batch input: the canonical shape is `{ "files": [...] }`, but a
/// model that just wrote one file frequently sends the spec flat
/// (`{ "path", "content", ... }`) — verify-wake-fix-3 postmortem: that shape
/// bounced with a raw serde "missing field `files`". Accept it as a
/// one-entry batch; anything else gets the contract named back. The
/// published schema stays the canonical batch shape (goldens pin it).
pub(crate) fn files_batch_or_single<T: serde::de::DeserializeOwned>(
    value: serde_json::Value,
    function_id: &str,
) -> Result<(Vec<T>, Option<crate::fs::FsScope>), String> {
    #[derive(serde::Deserialize)]
    struct Batch<T> {
        files: Vec<T>,
        #[serde(default)]
        fs_scope: Option<crate::fs::FsScope>,
    }
    if value.get("files").is_some() {
        let batch: Batch<T> = serde_json::from_value(value)
            .map_err(|e| format!("{function_id}: invalid `files` entry: {e}"))?;
        return Ok((batch.files, batch.fs_scope));
    }
    if value.get("path").is_some() {
        let fs_scope = match value.get("fs_scope") {
            Some(v) => serde_json::from_value(v.clone())
                .map_err(|e| format!("{function_id}: invalid `fs_scope`: {e}"))?,
            None => None,
        };
        let spec: T = serde_json::from_value(value)
            .map_err(|e| format!("{function_id}: invalid file entry: {e}"))?;
        return Ok((vec![spec], fs_scope));
    }
    Err(format!(
        "{function_id} takes {{ \"files\": [{{ \"path\", \"content\", ... }}] }}; a \
         single file may also be passed flat as {{ \"path\", \"content\" }}."
    ))
}

// ---------------------------------------------------------------------------
// Function ids + registration descriptions (ONE place).
//
// Each description carries the jail-contract clause ONCE; schema field docs
// state only what the field is (MOT-4639: the prose IS the contract the model
// sees via functions::info, so keep it short).
// ---------------------------------------------------------------------------

const INFO_ID: &str = "coder::info";
const INFO_DESC: &str = "Report the coder access contract: mode (jailed | unjailed), allowed \
     roots (primary first), the session_root relative paths anchor against, \
     size caps, response budgets, listing/search limits, non-accessible \
     globs and default_exclude_globs. Call it first when a path is \
     rejected.";

const READ_FILE_ID: &str = "coder::read-file";
const READ_FILE_DESC: &str =
    "Read a file. stat: true probes size/mtime/total_lines without content; \
     line_from/line_to (1-based, inclusive) read a window of a file of any \
     size; numbered: true prefixes absolute line numbers; paths[] (XOR \
     path) batch-reads. Paths: relative to the primary root or absolute \
     inside an allowed root (see coder::info).";

const SEARCH_ID: &str = "coder::search";
const SEARCH_DESC: &str = "Search file contents (literal or regex) and/or paths (files and dirs); \
     first match per line only. default_exclude_globs noise is skipped \
     unless use_default_excludes: false. truncated: true means refine the \
     query, not paginate. Paths: relative to the primary root or absolute \
     inside an allowed root (see coder::info).";

const UPDATE_FILE_ID: &str = "coder::update-file";
const UPDATE_FILE_DESC: &str =
    "Edit one or more files: batched line ops (1-based, inclusive, applied \
     bottom-up), then regex replace ops; each file commits atomically. \
     To replace a large region use two short anchors joined by .*? with \
     dot_matches_newline: true instead of quoting it. Paths: relative to \
     the primary root or absolute inside an allowed root (see coder::info).";

const CREATE_FILE_ID: &str = "coder::create-file";
const CREATE_FILE_DESC: &str =
    "Create one or more files atomically; per-file overwrite and parents \
     flags. For a conflict-safe overwrite pass the revision from \
     coder::read-file as expected_revision (stale revision: C221, nothing \
     written). Paths: relative to the primary root or absolute inside an \
     allowed root (see coder::info).";

const DELETE_FILE_ID: &str = "coder::delete-file";
const DELETE_FILE_DESC: &str =
    "Remove one or more paths. Directories need recursive: true; missing \
     paths succeed; recursion refuses to descend through non-accessible \
     entries. Paths: relative to the primary root or absolute inside an \
     allowed root (see coder::info).";

const LIST_FOLDER_ID: &str = "coder::list-folder";
const LIST_FOLDER_DESC: &str = "List one folder, paginated and sorted by name. Entries carry only \
     name; entry path = response path + '/' + name. Non-accessible entries \
     are listed with non_accessible: true. Paths: relative to the primary \
     root or absolute inside an allowed root (see coder::info).";

const TREE_ID: &str = "coder::tree";
const TREE_DESC: &str = "Show a directory tree, bounded by max_depth, per_folder_limit \
     and a node budget. Nodes carry only name (child path = parent path + \
     '/' + name); over-limit folders are truncated stubs to paginate with \
     coder::list-folder. Paths: relative to the primary root or absolute \
     inside an allowed root (see coder::info).";

const MOVE_FILE_ID: &str = "coder::move";
const MOVE_FILE_DESC: &str = "Move or rename one or more paths; per-entry overwrite and parents \
     flags. Same-root moves rename atomically; cross-root moves copy+delete \
     files only (move directory contents individually) and remove the copy \
     if the delete fails. Paths: relative to the primary root or absolute \
     inside an allowed root (see coder::info).";

/// One function's complete agent-facing wire surface: id, registration
/// description, and the schemars-derived request/response schemas.
///
/// `catalog()` is the single source of truth for the wire surface;
/// `register_all` consumes only its COUNT (the 1:1 drift guard), while the
/// per-field schema payloads back the wire-schema goldens. The `bin` target
/// never reads those fields, so allow dead_code here rather than drop the
/// catalog's source-of-truth richness.
#[allow(dead_code)]
pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

/// Schema generation MUST mirror iii-sdk's internal `json_schema_for`
/// (`SchemaSettings::draft07()` on the handler's request/response types):
/// `RegisterFunction::new_async` auto-extracts schemas from the SAME
/// structs referenced here, with the same schemars 0.8 generator settings,
/// so a catalog snapshot pins exactly what registration emits.
fn schema_of<T: schemars::JsonSchema>() -> schemars::schema::RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req, Resp>(function_id: &'static str, description: &'static str) -> FunctionSpec
where
    Req: schemars::JsonSchema,
    Resp: schemars::JsonSchema,
{
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

/// The full wire-surface catalog, in registration order. Golden-tested in
/// `tests/code_golden_schemas.rs`; keep in lockstep with `register_all`.
pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<info::InfoInput, info::InfoOutput>(INFO_ID, INFO_DESC),
        spec::<read_file::ReadFileInput, read_file::ReadFileOutput>(READ_FILE_ID, READ_FILE_DESC),
        spec::<search::SearchInput, search::SearchOutput>(SEARCH_ID, SEARCH_DESC),
        spec::<update_file::UpdateFileInput, update_file::UpdateFileOutput>(
            UPDATE_FILE_ID,
            UPDATE_FILE_DESC,
        ),
        spec::<create_file::CreateFileInput, create_file::CreateFileOutput>(
            CREATE_FILE_ID,
            CREATE_FILE_DESC,
        ),
        spec::<delete_file::DeleteFileInput, delete_file::DeleteFileOutput>(
            DELETE_FILE_ID,
            DELETE_FILE_DESC,
        ),
        spec::<list_folder::ListFolderInput, list_folder::ListFolderOutput>(
            LIST_FOLDER_ID,
            LIST_FOLDER_DESC,
        ),
        spec::<tree::TreeInput, tree::TreeOutput>(TREE_ID, TREE_DESC),
        spec::<move_file::MoveFileInput, move_file::MoveFileOutput>(MOVE_FILE_ID, MOVE_FILE_DESC),
    ]
}

pub fn register_all(iii: &IIIClient, cells: CodeCells) {
    // DRIFT GUARD: the register_* calls below and the entries in
    // `catalog()` must stay 1:1 — catalog() feeds the wire-schema goldens
    // (tests/code_golden_schemas.rs). Adding a function to one list but not
    // the other trips the debug_assert below (exercised engine-free by
    // `tests::register_all_count_matches_catalog`).
    let mut registered: usize = 0;
    register_info(iii, cells.clone());
    registered += 1;
    register_read_file(iii, cells.clone());
    registered += 1;
    register_search(iii, cells.clone());
    registered += 1;
    register_update_file(iii, cells.clone());
    registered += 1;
    register_create_file(iii, cells.clone());
    registered += 1;
    register_delete_file(iii, cells.clone());
    registered += 1;
    register_change_diff(iii, cells.clone());
    register_list_folder(iii, cells.clone());
    registered += 1;
    register_tree(iii, cells.clone());
    registered += 1;
    register_move_file(iii, cells);
    registered += 1;
    debug_assert_eq!(
        registered,
        catalog().len(),
        "register_all and catalog() drifted — register every catalog() \
         entry (and vice versa), then regenerate the wire-schema goldens \
         (UPDATE_GOLDENS=1 cargo test)"
    );
    tracing::info!(count = registered, "coder registered functions");
}

fn register_change_diff(iii: &IIIClient, cells: CodeCells) {
    iii.register_function(
        "coder::change-diff",
        RegisterFunction::new_async(move |req: ChangeDiffInput| {
            let journal = cells.changes.clone();
            async move { change_journal::diff(&journal, req).map_err(Error::from) }
        })
        .description("Internal console UI: retrieve an exact before/after snapshot by change id.")
        .metadata(serde_json::json!({ "internal": true })),
    );
}

fn register_info(iii: &IIIClient, cells: CodeCells) {
    iii.register_function(
        INFO_ID,
        RegisterFunction::new_async(move |req: info::InfoInput| {
            let cells = cells.clone();
            async move {
                let resolver = cells.resolver.read().await.clone();
                let cfg = cells.config.read().await.clone();
                info::handle(resolver, cfg, req).await.map_err(Error::from)
            }
        })
        .description(INFO_DESC),
    );
}

fn register_read_file(iii: &IIIClient, cells: CodeCells) {
    iii.register_function(
        READ_FILE_ID,
        RegisterFunction::new_async(move |req: read_file::ReadFileInput| {
            let cells = cells.clone();
            async move {
                let resolver = cells.resolver.read().await.clone();
                let resolver = resolver.session_scoped(
                    crate::fs::scope_root(req.fs_scope.as_ref()),
                    crate::fs::scope_grants(req.fs_scope.as_ref()),
                );
                let cfg = cells.config.read().await.clone();
                read_file::handle(resolver, cfg, req)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(READ_FILE_DESC),
    );
}

fn register_search(iii: &IIIClient, cells: CodeCells) {
    iii.register_function(
        SEARCH_ID,
        RegisterFunction::new_async(move |req: search::SearchInput| {
            let cells = cells.clone();
            async move {
                let resolver = cells.resolver.read().await.clone();
                let resolver = resolver.session_scoped(
                    crate::fs::scope_root(req.fs_scope.as_ref()),
                    crate::fs::scope_grants(req.fs_scope.as_ref()),
                );
                let cfg = cells.config.read().await.clone();
                search::handle(resolver, cfg, req)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(SEARCH_DESC),
    );
}

fn register_update_file(iii: &IIIClient, cells: CodeCells) {
    iii.register_function(
        UPDATE_FILE_ID,
        RegisterFunction::new_async(move |req: update_file::UpdateFileInput| {
            let cells = cells.clone();
            async move {
                let resolver = cells.resolver.read().await.clone();
                let resolver = resolver.session_scoped(
                    crate::fs::scope_root(req.fs_scope.as_ref()),
                    crate::fs::scope_grants(req.fs_scope.as_ref()),
                );
                let cfg = cells.config.read().await.clone();
                update_file::handle_with_journal(resolver, cfg, cells.changes.clone(), req)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(UPDATE_FILE_DESC)
        .metadata(serde_json::json!({ "display": true })),
    );
}

fn register_create_file(iii: &IIIClient, cells: CodeCells) {
    iii.register_function(
        CREATE_FILE_ID,
        RegisterFunction::new_async(move |req: create_file::CreateFileInput| {
            let cells = cells.clone();
            async move {
                let resolver = cells.resolver.read().await.clone();
                let resolver = resolver.session_scoped(
                    crate::fs::scope_root(req.fs_scope.as_ref()),
                    crate::fs::scope_grants(req.fs_scope.as_ref()),
                );
                let cfg = cells.config.read().await.clone();
                create_file::handle_with_journal(resolver, cfg, cells.changes.clone(), req)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(CREATE_FILE_DESC)
        .metadata(serde_json::json!({ "display": true })),
    );
}

fn register_delete_file(iii: &IIIClient, cells: CodeCells) {
    iii.register_function(
        DELETE_FILE_ID,
        RegisterFunction::new_async(move |req: delete_file::DeleteFileInput| {
            let cells = cells.clone();
            async move {
                let resolver = cells.resolver.read().await.clone();
                let resolver = resolver.session_scoped(
                    crate::fs::scope_root(req.fs_scope.as_ref()),
                    crate::fs::scope_grants(req.fs_scope.as_ref()),
                );
                delete_file::handle_with_journal(resolver, cells.changes.clone(), req)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(DELETE_FILE_DESC)
        .metadata(serde_json::json!({ "display": true })),
    );
}

fn register_list_folder(iii: &IIIClient, cells: CodeCells) {
    iii.register_function(
        LIST_FOLDER_ID,
        RegisterFunction::new_async(move |req: list_folder::ListFolderInput| {
            let cells = cells.clone();
            async move {
                let resolver = cells.resolver.read().await.clone();
                let resolver = resolver.session_scoped(
                    crate::fs::scope_root(req.fs_scope.as_ref()),
                    crate::fs::scope_grants(req.fs_scope.as_ref()),
                );
                let cfg = cells.config.read().await.clone();
                list_folder::handle(resolver, cfg, req)
                    .await
                    .map_err(Error::from)
            }
        })
        .description(LIST_FOLDER_DESC),
    );
}

fn register_tree(iii: &IIIClient, cells: CodeCells) {
    iii.register_function(
        TREE_ID,
        RegisterFunction::new_async(move |req: tree::TreeInput| {
            let cells = cells.clone();
            async move {
                let resolver = cells.resolver.read().await.clone();
                let resolver = resolver.session_scoped(
                    crate::fs::scope_root(req.fs_scope.as_ref()),
                    crate::fs::scope_grants(req.fs_scope.as_ref()),
                );
                let cfg = cells.config.read().await.clone();
                tree::handle(resolver, cfg, req).await.map_err(Error::from)
            }
        })
        .description(TREE_DESC),
    );
}

fn register_move_file(iii: &IIIClient, cells: CodeCells) {
    iii.register_function(
        MOVE_FILE_ID,
        RegisterFunction::new_async(move |req: move_file::MoveFileInput| {
            let cells = cells.clone();
            async move {
                let resolver = cells.resolver.read().await.clone();
                let resolver = resolver.session_scoped(
                    crate::fs::scope_root(req.fs_scope.as_ref()),
                    crate::fs::scope_grants(req.fs_scope.as_ref()),
                );
                move_file::handle(resolver, req).await.map_err(Error::from)
            }
        })
        .description(MOVE_FILE_DESC),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// DRIFT GUARD execution: `IIIClient::new` only buffers registrations into a
    /// channel (no connection, no runtime needed), so `register_all` runs
    /// engine-free here and its debug_assert fires in `cargo test` when
    /// the register_* calls and `catalog()` fall out of 1:1.
    #[test]
    fn register_all_count_matches_catalog() {
        use crate::code::config::CoderConfig;
        use crate::code::path::PathResolver;
        use crate::code::state::CodeCells;
        use tokio::sync::RwLock;
        let iii = IIIClient::new("ws://127.0.0.1:1");
        let cfg = CoderConfig::default();
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        let cells = CodeCells {
            config: Arc::new(RwLock::new(Arc::new(cfg))),
            resolver: Arc::new(RwLock::new(resolver)),
            changes: Default::default(),
        };
        register_all(&iii, cells);
    }
}
