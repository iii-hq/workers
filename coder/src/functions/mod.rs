//! Per-verb function modules and the single `register_all` entry point.
//! `main.rs` calls `register_all` after `register_worker`; each
//! `register_<verb>` below uses `RegisterFunction::new_async` with typed
//! `JsonSchema` request/response structs so the SDK can emit schemas for
//! tools and docs (binary-worker.md §7).

pub mod create_file;
pub mod delete_file;
pub mod list_folder;
pub mod read_file;
pub mod search;
pub mod tree;
pub mod update_file;

use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, III};

use crate::config::CoderConfig;
use crate::path::PathResolver;

pub fn register_all(iii: &III, resolver: Arc<PathResolver>, cfg: Arc<CoderConfig>) {
    register_read_file(iii, resolver.clone(), cfg.clone());
    register_search(iii, resolver.clone(), cfg.clone());
    register_update_file(iii, resolver.clone(), cfg.clone());
    register_create_file(iii, resolver.clone(), cfg.clone());
    register_delete_file(iii, resolver.clone());
    register_list_folder(iii, resolver.clone(), cfg.clone());
    register_tree(iii, resolver, cfg);
    tracing::info!("coder registered 7 functions");
}

fn register_read_file(iii: &III, resolver: Arc<PathResolver>, cfg: Arc<CoderConfig>) {
    iii.register_function(
        "coder::read-file",
        RegisterFunction::new_async(move |req: read_file::ReadFileInput| {
            let resolver = resolver.clone();
            let cfg = cfg.clone();
            async move {
                read_file::handle(resolver, cfg, req)
                    .await
                    .map_err(IIIError::from)
            }
        })
        .description(
            "Read a file relative to base_path. Returns content plus \
             size/mtime/mode. Capped by max_read_bytes; non-accessible \
             paths return C211.",
        ),
    );
}

fn register_search(iii: &III, resolver: Arc<PathResolver>, cfg: Arc<CoderConfig>) {
    iii.register_function(
        "coder::search",
        RegisterFunction::new_async(move |req: search::SearchInput| {
            let resolver = resolver.clone();
            let cfg = cfg.clone();
            async move {
                search::handle(resolver, cfg, req)
                    .await
                    .map_err(IIIError::from)
            }
        })
        .description(
            "Search file contents and/or paths under base_path. Supports \
             literal or regex queries with include/exclude globs; \
             non-accessible files are excluded from both content and path \
             results.",
        ),
    );
}

fn register_update_file(iii: &III, resolver: Arc<PathResolver>, cfg: Arc<CoderConfig>) {
    iii.register_function(
        "coder::update-file",
        RegisterFunction::new_async(move |req: update_file::UpdateFileInput| {
            let resolver = resolver.clone();
            let cfg = cfg.clone();
            async move {
                update_file::handle(resolver, cfg, req)
                    .await
                    .map_err(IIIError::from)
            }
        })
        .description(
            "Apply batched line-oriented and regex edits across one or more \
             files. Line ops: { op: 'insert', at_line, content } | \
             { op: 'remove', from_line, to_line } | \
             { op: 'update_lines', from_line, to_line, content } — 1-based, \
             inclusive, applied bottom-up. Regex op: { op: 'replace', pattern, \
             replacement, ignore_case? } runs on the file body after line \
             ops. Each file commits atomically via temp + rename.",
        ),
    );
}

fn register_create_file(iii: &III, resolver: Arc<PathResolver>, cfg: Arc<CoderConfig>) {
    iii.register_function(
        "coder::create-file",
        RegisterFunction::new_async(move |req: create_file::CreateFileInput| {
            let resolver = resolver.clone();
            let cfg = cfg.clone();
            async move {
                create_file::handle(resolver, cfg, req)
                    .await
                    .map_err(IIIError::from)
            }
        })
        .description(
            "Create one or more files. Per-file `overwrite` and `parents` \
             flags; non-accessible paths return C211.",
        ),
    );
}

fn register_delete_file(iii: &III, resolver: Arc<PathResolver>) {
    iii.register_function(
        "coder::delete-file",
        RegisterFunction::new_async(move |req: delete_file::DeleteFileInput| {
            let resolver = resolver.clone();
            async move {
                delete_file::handle(resolver, req)
                    .await
                    .map_err(IIIError::from)
            }
        })
        .description(
            "Remove one or more paths. Directories need `recursive: true`; \
             missing paths are idempotent successes; recursive removal \
             refuses to descend through non-accessible entries.",
        ),
    );
}

fn register_list_folder(iii: &III, resolver: Arc<PathResolver>, cfg: Arc<CoderConfig>) {
    iii.register_function(
        "coder::list-folder",
        RegisterFunction::new_async(move |req: list_folder::ListFolderInput| {
            let resolver = resolver.clone();
            let cfg = cfg.clone();
            async move {
                list_folder::handle(resolver, cfg, req)
                    .await
                    .map_err(IIIError::from)
            }
        })
        .description(
            "Paginated single-folder listing, sorted by name. \
             Non-accessible entries are still listed with a \
             `non_accessible: true` flag.",
        ),
    );
}

fn register_tree(iii: &III, resolver: Arc<PathResolver>, cfg: Arc<CoderConfig>) {
    iii.register_function(
        "coder::tree",
        RegisterFunction::new_async(move |req: tree::TreeInput| {
            let resolver = resolver.clone();
            let cfg = cfg.clone();
            async move {
                tree::handle(resolver, cfg, req)
                    .await
                    .map_err(IIIError::from)
            }
        })
        .description(
            "Recursive directory snapshot bounded by `max_depth` and a \
             `per_folder_limit`. Folders that hit the limit are flagged \
             `truncated` and the caller is pointed at coder::list-folder \
             for pagination.",
        ),
    );
}
