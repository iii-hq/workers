//! The agent-facing wire surface, pinned.
//!
//! Every registered function with the description and schemas registration
//! emits. `tests/schemas.rs` snapshots this, so a change to a request type, a
//! response type, or the sentence an agent reads when choosing a function
//! lands as a reviewed golden diff instead of as a silently reshaped API.

use schemars::schema::RootSchema;
use serde::Serialize;

use crate::diff::DiffResult;
use crate::functions::types::*;
use crate::functions::{
    GitStatusOutput, DESC_BUFFERS_CLOSE, DESC_BUFFERS_LIST, DESC_CHANGES, DESC_CREATE, DESC_DELETE,
    DESC_DIFF, DESC_FIND, DESC_GIT_COMMIT, DESC_GIT_HUNKS, DESC_GIT_SHOW, DESC_GIT_STASH,
    DESC_GIT_STATUS, DESC_GIT_SYNC, DESC_GIT_UNDO_COMMIT, DESC_MOVE, DESC_OPEN, DESC_SAVE,
    DESC_SEARCH, DESC_TREE, DESC_WORKSPACE_GET, DESC_WORKSPACE_OPEN,
};

#[derive(Debug, Serialize)]
pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: RootSchema,
    pub response_schema: RootSchema,
}

/// Build a schema with the same generator the SDK uses at registration time,
/// so the snapshot equals what the engine publishes.
fn schema_of<T: schemars::JsonSchema>() -> RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req: schemars::JsonSchema, Resp: schemars::JsonSchema>(
    function_id: &'static str,
    description: &'static str,
) -> FunctionSpec {
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

/// In registration order — `tests/schemas.rs` asserts the match.
pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<WorkspaceOpenInput, WorkspaceView>("editor::workspace::open", DESC_WORKSPACE_OPEN),
        spec::<EmptyInput, WorkspaceView>("editor::workspace::get", DESC_WORKSPACE_GET),
        spec::<TreeInput, TreeOutput>("editor::tree", DESC_TREE),
        spec::<EmptyInput, ChangesView>("editor::changes", DESC_CHANGES),
        spec::<OpenInput, OpenOutput>("editor::open", DESC_OPEN),
        spec::<SaveInput, SaveOutput>("editor::save", DESC_SAVE),
        spec::<EmptyInput, BuffersOutput>("editor::buffers::list", DESC_BUFFERS_LIST),
        spec::<BufferCloseInput, BufferCloseOutput>("editor::buffers::close", DESC_BUFFERS_CLOSE),
        spec::<MoveInput, MoveOutput>("editor::move", DESC_MOVE),
        spec::<CreateInput, CreateOutput>("editor::create", DESC_CREATE),
        spec::<DeleteInput, DeleteOutput>("editor::delete", DESC_DELETE),
        spec::<FindInput, FindOutput>("editor::find", DESC_FIND),
        spec::<SearchInput, SearchOutput>("editor::search", DESC_SEARCH),
        spec::<DiffInput, DiffResult>("editor::diff", DESC_DIFF),
        spec::<GitStatusInput, GitStatusOutput>("editor::git::status", DESC_GIT_STATUS),
        spec::<GitHunksInput, GitHunksOutput>("editor::git::hunks", DESC_GIT_HUNKS),
        spec::<GitShowInput, GitShowOutput>("editor::git::show", DESC_GIT_SHOW),
        spec::<GitCommitInput, GitCommitOutput>("editor::git::commit", DESC_GIT_COMMIT),
        spec::<GitSyncInput, GitSyncOutput>("editor::git::sync", DESC_GIT_SYNC),
        spec::<GitStashInput, GitStashOutput>("editor::git::stash", DESC_GIT_STASH),
        spec::<GitUndoCommitInput, GitUndoCommitOutput>(
            "editor::git::undo-commit",
            DESC_GIT_UNDO_COMMIT,
        ),
    ]
}
