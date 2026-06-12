//! Per-verb function modules and the single `register_all` entry point.
//! `main.rs` calls `register_all` after `register_worker`; each
//! `register_<verb>` below uses `RegisterFunction::new_async` with typed
//! `JsonSchema` request/response structs so the SDK can emit schemas for
//! tools and docs (binary-worker.md §7).
//!
//! WIRE-SURFACE CATALOG — `catalog()` below is the single source of truth
//! for every function's id + registration description, plus the
//! schemars-derived request/response schemas. The golden test
//! `tests/golden_schemas.rs` snapshots each entry so ANY change to the
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

use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, III};

use crate::configuration::ConfigCell;
use crate::path::PathResolver;

// ---------------------------------------------------------------------------
// Function ids + registration descriptions (ONE place).
//
// The jail-contract sentence in every description below also appears in
// the schema field docs of each input struct. This duplication is
// deliberate: the catalog description and the schema docs reach agents
// through DIFFERENT surfaces — `functions::list` shows descriptions only,
// while the schema docs surface via `functions::info`. Do not "DRY" one
// into the other.
// ---------------------------------------------------------------------------

const INFO_ID: &str = "coder::info";
const INFO_DESC: &str = "Report the coder jail: canonical allowed roots (primary first), \
     per-file size caps, response budgets (max_output_bytes, \
     batch_read_budget_bytes, search_response_budget_bytes), \
     listing/search limits, the non-accessible glob patterns, and the \
     default_exclude_globs noise filter applied by tree/search. Call \
     this FIRST when unsure where coder may read or write, or when a \
     path was rejected — paths outside every allowed root need the \
     shell worker's shell::fs::* instead.";

const READ_FILE_ID: &str = "coder::read-file";
const READ_FILE_DESC: &str = "Read a file window-first: probe with stat: true (size/mtime/mode \
     plus total_lines, no content), then fetch just the lines you need \
     with line_from/line_to (1-based, inclusive) — windows keep files \
     larger than max_read_bytes readable window by window, with \
     more_lines/total_lines reporting what remains. numbered: true \
     prefixes each line with its absolute 1-based file line number, \
     matching coder::update-file's line ops exactly. Full reads are \
     budgeted by max_output_bytes (default 128 KiB; per-call override \
     clamped to max_read_bytes) — an over-budget full read fails with \
     a C213 carrying the file's size, line count, and the window/stat \
     recovery calls. Batch mode: pass paths[] (XOR path) \
     to read multiple files in one call — entries are processed in \
     request order against batch_read_budget_bytes, measured in \
     bytes of returned content (after UTF-8 sanitization); per-entry \
     errors (C211/C213) leave other entries unaffected. Paths are relative \
     to the primary allowed root or absolute inside any allowed root \
     (coder::info lists them); for host paths outside the jail use \
     shell::fs::*. Non-accessible paths return C211.";

const SEARCH_ID: &str = "coder::search";
const SEARCH_DESC: &str = "Search file contents and/or paths. Supports literal or regex \
     queries with include/exclude globs; non-accessible files are \
     excluded from both content and path results. Only the FIRST match \
     on each line is reported (one content match per matching line). \
     Optional context_lines_before/context_lines_after (max 10) attach \
     surrounding lines to each content match so many edits can go \
     straight to coder::update-file with no read in between. Noise \
     paths matching default_exclude_globs (.git, node_modules, target, \
     … — coder::info lists them) are skipped by default; pass \
     use_default_excludes: false to search inside them. Files larger \
     than max_read_bytes are silently skipped during content scanning. \
     Results are capped by max_matches AND a response byte budget \
     (search_response_budget_bytes); when truncated is true, refine \
     the query or add include_globs rather than paginate. Paths are relative \
     to the primary allowed root or absolute inside any allowed root \
     (coder::info lists them); for host paths outside the jail use \
     shell::fs::*.";

const UPDATE_FILE_ID: &str = "coder::update-file";
const UPDATE_FILE_DESC: &str = "Apply batched line-oriented and regex edits across one or more \
     files. Request shape: {\"files\": [{\"path\": \"...\", \"ops\": [...]}]}. \
     Line ops: { op: 'insert', at_line, content } | \
     { op: 'remove', from_line, to_line } | \
     { op: 'update_lines', from_line, to_line, content } — 1-based, \
     inclusive, applied bottom-up. Regex op: { op: 'replace', pattern, \
     replacement, ignore_case?, dot_matches_newline?, expect_matches? } \
     runs on the file body after line ops. Replace large regions \
     WITHOUT quoting them: two short anchors joined by .*? with \
     dot_matches_newline: true — always prefer wildcards over pasting \
     the block into the pattern. expect_matches: 1 turns a silent \
     multi-site clobber into a safe pre-write C210; expect_matches: 0 \
     asserts absence. In `replacement`, $1/${name} are capture \
     references and a literal $ must be written $$ (JS/TS template \
     literals: `Hello, $${name}!`); undefined references fail \
     pre-write with C210. Each file commits atomically via temp + \
     rename. On success each applied line op returns a bounded \
     post-apply echo (±2 context lines); regex replace ops return up \
     to 5 per-match-site echoes (first + last line of each replaced \
     region, inner lines elided) — verify from the echoes instead of \
     re-reading the file. Paths are relative to the primary \
     allowed root or absolute inside any allowed root (coder::info \
     lists them); for host paths outside the jail use shell::fs::*.";

const CREATE_FILE_ID: &str = "coder::create-file";
const CREATE_FILE_DESC: &str = "Create one or more files. Request shape: {\"files\": [{\"path\": \
     \"...\", \"content\": \"...\"}]}. Per-file `overwrite` and `parents` \
     flags; non-accessible paths return C211. Paths are relative to \
     the primary allowed root or absolute inside any allowed root \
     (coder::info lists them); for host paths outside the jail use \
     shell::fs::*.";

const DELETE_FILE_ID: &str = "coder::delete-file";
const DELETE_FILE_DESC: &str = "Remove one or more paths. Request shape: {\"paths\": [\"...\"]}. \
     Directories need `recursive: true`; \
     missing paths are idempotent successes; recursive removal \
     refuses to descend through non-accessible entries. Paths are \
     relative to the primary allowed root or absolute inside any \
     allowed root (coder::info lists them); for host paths outside \
     the jail use shell::fs::*.";

const LIST_FOLDER_ID: &str = "coder::list-folder";
const LIST_FOLDER_DESC: &str = "Paginated single-folder listing, sorted by name. Entries carry \
     only `name`; derive an entry's absolute path as the response's \
     `path` + '/' + name. Non-accessible entries are still listed with a \
     `non_accessible: true` flag. Paths are relative to the primary \
     allowed root or absolute inside any allowed root (coder::info \
     lists them); for host paths outside the jail use shell::fs::*.";

const TREE_ID: &str = "coder::tree";
const TREE_DESC: &str = "Recursive directory snapshot bounded by `max_depth` and a \
     `per_folder_limit`. Slim wire shape: nodes carry only `name` — \
     the root node's path IS the response's top-level `path`; derive \
     any child's path as parent path + '/' + name. Folders that hit \
     the limit are flagged `truncated` and the caller is pointed at \
     coder::list-folder for pagination. Noise directories matching \
     default_exclude_globs (.git, node_modules, target, … — \
     coder::info lists them) appear as childless `truncated` stubs; \
     pass use_default_excludes: false to descend into them. Paths are \
     relative to the primary allowed root or absolute inside any \
     allowed root (coder::info lists them); for host paths outside \
     the jail use shell::fs::*.";

const MOVE_FILE_ID: &str = "coder::move";
const MOVE_FILE_DESC: &str = "Move or rename one or more paths inside the jail. Request shape: \
     {\"files\": [{\"from\": \"...\", \"to\": \"...\"}]}. Paths are \
     relative to the primary allowed root or absolute inside any \
     allowed root (coder::info lists them); for host paths outside \
     the jail use shell::fs::*. Per-entry `overwrite` and `parents` \
     flags. Same-root moves use a per-file-atomic rename; cross-root \
     moves use copy+delete (files only — cross-root directory moves \
     are unsupported, move files individually). Copy+delete is \
     rollback-safe: if source deletion fails after a successful copy \
     the copy is removed and the error names the failure; if rollback \
     also fails the error names both states for manual cleanup.";

/// One function's complete agent-facing wire surface: id, registration
/// description, and the schemars-derived request/response schemas.
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
/// `tests/golden_schemas.rs`; keep in lockstep with `register_all`.
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

pub fn register_all(iii: &III, resolver: Arc<PathResolver>, cfg: ConfigCell) {
    // DRIFT GUARD: the register_* calls below and the entries in
    // `catalog()` must stay 1:1 — catalog() feeds the wire-schema goldens
    // (tests/golden_schemas.rs). Adding a function to one list but not
    // the other trips the debug_assert below (exercised engine-free by
    // `tests::register_all_count_matches_catalog`).
    let mut registered: usize = 0;
    register_info(iii, resolver.clone(), cfg.clone());
    registered += 1;
    register_read_file(iii, resolver.clone(), cfg.clone());
    registered += 1;
    register_search(iii, resolver.clone(), cfg.clone());
    registered += 1;
    register_update_file(iii, resolver.clone(), cfg.clone());
    registered += 1;
    register_create_file(iii, resolver.clone(), cfg.clone());
    registered += 1;
    register_delete_file(iii, resolver.clone());
    registered += 1;
    register_list_folder(iii, resolver.clone(), cfg.clone());
    registered += 1;
    register_tree(iii, resolver.clone(), cfg.clone());
    registered += 1;
    register_move_file(iii, resolver);
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

fn register_info(iii: &III, resolver: Arc<PathResolver>, cfg: ConfigCell) {
    iii.register_function(
        INFO_ID,
        RegisterFunction::new_async(move |_req: info::InfoInput| {
            let resolver = resolver.clone();
            let cfg = cfg.clone();
            async move {
                let cfg = cfg.read().await.clone();
                info::handle(resolver, cfg).await.map_err(IIIError::from)
            }
        })
        .description(INFO_DESC),
    );
}

fn register_read_file(iii: &III, resolver: Arc<PathResolver>, cfg: ConfigCell) {
    iii.register_function(
        READ_FILE_ID,
        RegisterFunction::new_async(move |req: read_file::ReadFileInput| {
            let resolver = resolver.clone();
            let cfg = cfg.clone();
            async move {
                let cfg = cfg.read().await.clone();
                read_file::handle(resolver, cfg, req)
                    .await
                    .map_err(IIIError::from)
            }
        })
        .description(READ_FILE_DESC),
    );
}

fn register_search(iii: &III, resolver: Arc<PathResolver>, cfg: ConfigCell) {
    iii.register_function(
        SEARCH_ID,
        RegisterFunction::new_async(move |req: search::SearchInput| {
            let resolver = resolver.clone();
            let cfg = cfg.clone();
            async move {
                let cfg = cfg.read().await.clone();
                search::handle(resolver, cfg, req)
                    .await
                    .map_err(IIIError::from)
            }
        })
        .description(SEARCH_DESC),
    );
}

fn register_update_file(iii: &III, resolver: Arc<PathResolver>, cfg: ConfigCell) {
    iii.register_function(
        UPDATE_FILE_ID,
        RegisterFunction::new_async(move |req: update_file::UpdateFileInput| {
            let resolver = resolver.clone();
            let cfg = cfg.clone();
            async move {
                let cfg = cfg.read().await.clone();
                update_file::handle(resolver, cfg, req)
                    .await
                    .map_err(IIIError::from)
            }
        })
        .description(UPDATE_FILE_DESC),
    );
}

fn register_create_file(iii: &III, resolver: Arc<PathResolver>, cfg: ConfigCell) {
    iii.register_function(
        CREATE_FILE_ID,
        RegisterFunction::new_async(move |req: create_file::CreateFileInput| {
            let resolver = resolver.clone();
            let cfg = cfg.clone();
            async move {
                let cfg = cfg.read().await.clone();
                create_file::handle(resolver, cfg, req)
                    .await
                    .map_err(IIIError::from)
            }
        })
        .description(CREATE_FILE_DESC),
    );
}

fn register_delete_file(iii: &III, resolver: Arc<PathResolver>) {
    iii.register_function(
        DELETE_FILE_ID,
        RegisterFunction::new_async(move |req: delete_file::DeleteFileInput| {
            let resolver = resolver.clone();
            async move {
                delete_file::handle(resolver, req)
                    .await
                    .map_err(IIIError::from)
            }
        })
        .description(DELETE_FILE_DESC),
    );
}

fn register_list_folder(iii: &III, resolver: Arc<PathResolver>, cfg: ConfigCell) {
    iii.register_function(
        LIST_FOLDER_ID,
        RegisterFunction::new_async(move |req: list_folder::ListFolderInput| {
            let resolver = resolver.clone();
            let cfg = cfg.clone();
            async move {
                let cfg = cfg.read().await.clone();
                list_folder::handle(resolver, cfg, req)
                    .await
                    .map_err(IIIError::from)
            }
        })
        .description(LIST_FOLDER_DESC),
    );
}

fn register_tree(iii: &III, resolver: Arc<PathResolver>, cfg: ConfigCell) {
    iii.register_function(
        TREE_ID,
        RegisterFunction::new_async(move |req: tree::TreeInput| {
            let resolver = resolver.clone();
            let cfg = cfg.clone();
            async move {
                let cfg = cfg.read().await.clone();
                tree::handle(resolver, cfg, req)
                    .await
                    .map_err(IIIError::from)
            }
        })
        .description(TREE_DESC),
    );
}

fn register_move_file(iii: &III, resolver: Arc<PathResolver>) {
    iii.register_function(
        MOVE_FILE_ID,
        RegisterFunction::new_async(move |req: move_file::MoveFileInput| {
            let resolver = resolver.clone();
            async move {
                move_file::handle(resolver, req)
                    .await
                    .map_err(IIIError::from)
            }
        })
        .description(MOVE_FILE_DESC),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DRIFT GUARD execution: `III::new` only buffers registrations into a
    /// channel (no connection, no runtime needed), so `register_all` runs
    /// engine-free here and its debug_assert fires in `cargo test` when
    /// the register_* calls and `catalog()` fall out of 1:1.
    #[test]
    fn register_all_count_matches_catalog() {
        use crate::config::CoderConfig;
        use tokio::sync::RwLock;
        let iii = III::new("ws://127.0.0.1:1");
        let cfg = CoderConfig::default();
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg)));
        register_all(&iii, resolver, cell);
    }
}
