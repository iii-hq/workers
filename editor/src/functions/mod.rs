//! Registration and handler bodies for the `editor::*` surface.
//!
//! The shape follows the editor this is modelled on: a **workspace** — a root,
//! the buffers open against it, and which folders are expanded — rather than a
//! bag of stateless verbs. That workspace lives in the `state` worker, so it is
//! a fact on the bus instead of something a browser tab happens to remember.
//! That is what lets an agent and a person share one editor: the agent opens a
//! file and it appears in your tabs, you open one and the agent can see it.
//!
//! Everything a filesystem already does is delegated. `shell` reads, writes,
//! moves and lists (`coder::tree` is its walk, not a reimplementation here);
//! `state` persists. What is left is the editor's own model: diffing, ranking
//! paths, refusing a stale write, and keeping open buffers correct when a
//! folder moves underneath them.
//!
//! A git repository is an overlay. Branch and marks appear when the root
//! happens to be a repo; nothing else changes when it is not.

pub mod types;

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};

use crate::bus::Bus;
use crate::config::WorkerConfig;
use crate::configuration::ConfigCell;
use crate::diff::DiffResult;
use crate::git::{parse_hunk_headers, parse_status, StatusReport};
use crate::workspace::{session_key, Buffer, Session, ACTIVE_ROOT_KEY};
use crate::{diff, fuzzy, lang, tree};

use types::*;

pub const DESC_WORKSPACE_OPEN: &str =
    "Set the directory the editor works in. Any folder will do — a git repository is an \
     overlay, not a requirement. Returns the buffers and expanded folders remembered for it.";
pub const DESC_WORKSPACE_GET: &str =
    "The active workspace: its root, the files open against it, and which folders are \
     expanded. Shared by every surface, so this is what the agent and the console both see.";
pub const DESC_TREE: &str =
    "List a folder in the workspace, with the expansion state the workspace remembers. \
     The walk, the noise-folder excludes and the jail are the shell worker's.";
pub const DESC_OPEN: &str =
    "Read a text file and record it as an open buffer, with the metadata needed to write \
     it back safely: its language id, and the mtime and content version to hand to \
     editor::save.";
pub const DESC_SAVE: &str =
    "Write a file, refusing the write when it changed underneath since the editor::open it \
     started from. On refusal the divergence comes back as a patch.";
pub const DESC_BUFFERS_LIST: &str = "Files currently open in the workspace.";
pub const DESC_BUFFERS_CLOSE: &str = "Close one open buffer. The file on disk is untouched.";
pub const DESC_MOVE: &str =
    "Move or rename a path and rewrite every open buffer and expanded folder at or under \
     it. Moving a folder with `shell::fs::mv` alone leaves buffers pointing at the old \
     location, which silently recreates it on the next save.";
pub const DESC_FIND: &str =
    "Fuzzy file finder over the workspace, ranked the way an editor's open-file palette \
     ranks. Candidates come from git when the root is a repository and from the folder \
     listing when it is not.";
pub const DESC_DIFF: &str =
    "Unified diff between two texts. Pure: nothing is read from disk. Use it to show what \
     an edit will do before writing it, or to explain what a write did.";
pub const DESC_GIT_STATUS: &str =
    "Working-tree status as typed rows: branch, upstream, ahead/behind, and one entry per \
     changed path. Fails when the root is not a repository — that is an absent overlay, \
     not a broken workspace.";
pub const DESC_CREATE: &str =
    "Create a file or folder in the workspace, with parents as needed. A file may be \
     seeded with content.";
pub const DESC_DELETE: &str =
    "Remove a path and close any buffer it held. An open buffer for a deleted file would \
     recreate it on the next save.";
pub const DESC_SEARCH: &str =
    "Search file contents across the workspace, grouped by file — the shell worker's \
     recursive grep, shaped into what a results panel renders.";
pub const DESC_GIT_COMMIT: &str =
    "Stage and commit. Returns the new SHA, or committed:false when there was nothing \
     staged.";
pub const DESC_GIT_SYNC: &str =
    "Fetch, fast-forward pull, or push. Pull is --ff-only on purpose: a merge under open \
     buffers is how an editor ends up showing a conflicted tree it never asked for.";
pub const DESC_GIT_STASH: &str = "Stash the working tree, or pop the most recent stash.";
pub const DESC_GIT_UNDO_COMMIT: &str =
    "Undo the last commit, keeping its changes staged (reset --soft HEAD~1). Returns the \
     SHA and message that were undone.";
pub const DESC_GIT_SHOW: &str =
    "Read a file's contents at a revision (HEAD by default). Pair it with the working copy \
     to render a real side-by-side or unified diff, rather than parsing a patch.";
pub const DESC_GIT_HUNKS: &str =
    "What changed in one file: the rendered patch plus its line ranges. Compares the \
     working tree against the index, the index against HEAD, or the working tree against \
     HEAD — so it shows an edit made by anything, including an agent that never called \
     this worker.";

/// `cfg` is the live snapshot cell, not a captured value: the configuration
/// worker can change these limits at any time, and a handler that closed over
/// an `Arc<WorkerConfig>` at registration would keep serving the boot-time
/// numbers forever.
pub fn register_all(iii: &Arc<IIIClient>, cfg: &ConfigCell, bus: &Arc<Bus>) {
    register_workspace_open(iii, bus);
    register_workspace_get(iii, bus);
    register_tree(iii, bus);
    register_open(iii, cfg, bus);
    register_save(iii, cfg, bus);
    register_buffers_list(iii, bus);
    register_buffers_close(iii, bus);
    register_move(iii, bus);
    register_create(iii, bus);
    register_delete(iii, bus);
    register_find(iii, cfg, bus);
    register_search(iii, cfg, bus);
    register_diff(iii, cfg);
    register_git_status(iii, bus);
    register_git_hunks(iii, cfg, bus);
    register_git_show(iii, bus);
    register_git_commit(iii, bus);
    register_git_sync(iii, bus);
    register_git_stash(iii, bus);
    register_git_undo_commit(iii, bus);
    tracing::info!("all functions registered");
}

/// Every registered function, in registration order, for the golden test.
pub fn function_ids() -> Vec<&'static str> {
    vec![
        "editor::workspace::open",
        "editor::workspace::get",
        "editor::tree",
        "editor::open",
        "editor::save",
        "editor::buffers::list",
        "editor::buffers::close",
        "editor::move",
        "editor::create",
        "editor::delete",
        "editor::find",
        "editor::search",
        "editor::diff",
        "editor::git::status",
        "editor::git::hunks",
        "editor::git::show",
        "editor::git::commit",
        "editor::git::sync",
        "editor::git::stash",
        "editor::git::undo-commit",
    ]
}

/// Response type of `editor::git::status`, named for the catalog.
pub type GitStatusOutput = StatusReport;

// ---------------------------------------------------------------- workspace

/// The active root, or `.` — shell's own working directory — when nothing has
/// been opened yet. Defaulting rather than erroring means every function works
/// on a fresh install without a setup step.
async fn active_root(bus: &Bus) -> String {
    bus.state_get(ACTIVE_ROOT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| ".".to_string())
}

/// A corrupt or absent session reads as an empty one. Losing the tab list is a
/// far better failure than refusing to open the editor.
async fn load_session(bus: &Bus, root: &str) -> Session {
    bus.state_get(&session_key(root))
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

async fn save_session(bus: &Bus, root: &str, session: &Session) -> Result<(), Error> {
    let value = serde_json::to_value(session)
        .map_err(|e| Error::Handler(format!("session failed to serialize: {e}")))?;
    bus.state_set(&session_key(root), value).await
}

fn view(root: String, session: Session) -> WorkspaceView {
    WorkspaceView {
        root,
        buffers: session.buffers,
        expanded: session.expanded,
    }
}

fn register_workspace_open(iii: &Arc<IIIClient>, bus: &Arc<Bus>) {
    let bus = bus.clone();
    iii.register_function(
        "editor::workspace::open",
        RegisterFunction::new_async(move |req: WorkspaceOpenInput| {
            let bus = bus.clone();
            async move {
                // Prove the folder is reachable before recording it, so a typo
                // fails here rather than as a confusing error on every later call.
                bus.tree(&req.root, 1).await?;
                bus.state_set(ACTIVE_ROOT_KEY, serde_json::json!(req.root))
                    .await?;
                let session = load_session(&bus, &req.root).await;
                Ok::<_, Error>(view(req.root, session))
            }
        })
        .description(DESC_WORKSPACE_OPEN),
    );
}

fn register_workspace_get(iii: &Arc<IIIClient>, bus: &Arc<Bus>) {
    let bus = bus.clone();
    iii.register_function(
        "editor::workspace::get",
        RegisterFunction::new_async(move |_req: EmptyInput| {
            let bus = bus.clone();
            async move {
                let root = active_root(&bus).await;
                let session = load_session(&bus, &root).await;
                Ok::<_, Error>(view(root, session))
            }
        })
        .description(DESC_WORKSPACE_GET),
    );
}

fn register_tree(iii: &Arc<IIIClient>, bus: &Arc<Bus>) {
    let bus = bus.clone();
    iii.register_function(
        "editor::tree",
        RegisterFunction::new_async(move |req: TreeInput| {
            let bus = bus.clone();
            async move {
                let root = active_root(&bus).await;
                let target = match req.path.as_deref() {
                    None | Some("") | Some(".") => root.clone(),
                    Some(rel) if rel.starts_with('/') => rel.to_string(),
                    Some(rel) => format!("{root}/{rel}"),
                };
                let listing = bus.tree(&target, req.max_depth.unwrap_or(4)).await?;

                let mut session = load_session(&bus, &root).await;
                let before = session.expanded.clone();
                for path in &req.expand {
                    session.expand(path);
                }
                for path in &req.collapse {
                    session.collapse(path);
                }
                if session.expanded != before {
                    save_session(&bus, &root, &session).await?;
                }
                Ok::<_, Error>(TreeOutput {
                    path: listing
                        .get("path")
                        .and_then(|p| p.as_str())
                        .unwrap_or(&target)
                        .to_string(),
                    root,
                    tree: listing,
                    expanded: session.expanded,
                })
            }
        })
        .description(DESC_TREE),
    );
}

// ------------------------------------------------------------------ buffers

fn register_buffers_list(iii: &Arc<IIIClient>, bus: &Arc<Bus>) {
    let bus = bus.clone();
    iii.register_function(
        "editor::buffers::list",
        RegisterFunction::new_async(move |_req: EmptyInput| {
            let bus = bus.clone();
            async move {
                let root = active_root(&bus).await;
                let session = load_session(&bus, &root).await;
                Ok::<_, Error>(BuffersOutput {
                    root,
                    buffers: session.buffers,
                })
            }
        })
        .description(DESC_BUFFERS_LIST),
    );
}

fn register_buffers_close(iii: &Arc<IIIClient>, bus: &Arc<Bus>) {
    let bus = bus.clone();
    iii.register_function(
        "editor::buffers::close",
        RegisterFunction::new_async(move |req: BufferCloseInput| {
            let bus = bus.clone();
            async move {
                let root = active_root(&bus).await;
                let mut session = load_session(&bus, &root).await;
                let closed = session.close(&req.path);
                if closed {
                    save_session(&bus, &root, &session).await?;
                }
                Ok::<_, Error>(BufferCloseOutput {
                    closed,
                    root,
                    buffers: session.buffers,
                })
            }
        })
        .description(DESC_BUFFERS_CLOSE),
    );
}

fn register_move(iii: &Arc<IIIClient>, bus: &Arc<Bus>) {
    let bus = bus.clone();
    iii.register_function(
        "editor::move",
        RegisterFunction::new_async(move |req: MoveInput| {
            let bus = bus.clone();
            async move {
                let root = active_root(&bus).await;
                bus.mv(&joined(&root, &req.from), &joined(&root, &req.to))
                    .await?;

                // The remap runs only after the move succeeded: rewriting
                // buffers for a move that failed would point them at a path
                // that does not exist.
                let mut session = load_session(&bus, &root).await;
                let remapped = session.remap(&req.from, &req.to) as u32;
                if remapped > 0 {
                    save_session(&bus, &root, &session).await?;
                }
                Ok::<_, Error>(MoveOutput {
                    from: req.from,
                    to: req.to,
                    remapped,
                    root,
                    buffers: session.buffers,
                })
            }
        })
        .description(DESC_MOVE),
    );
}

/// Join a root-relative path onto the root, leaving absolute paths alone.
fn joined(root: &str, rel: &str) -> String {
    if rel.starts_with('/') || root == "." {
        rel.to_string()
    } else {
        format!("{root}/{rel}")
    }
}

// -------------------------------------------------------------- open / save

fn register_open(iii: &Arc<IIIClient>, cfg: &ConfigCell, bus: &Arc<Bus>) {
    let cfg = cfg.clone();
    let bus = bus.clone();
    iii.register_function(
        "editor::open",
        RegisterFunction::new_async(move |req: OpenInput| {
            let cfg = cfg.clone();
            let bus = bus.clone();
            async move {
                let cfg = cfg.read().await.clone();
                let root = active_root(&bus).await;
                let file = bus
                    .read(&joined(&root, &req.path), cfg.max_file_bytes)
                    .await?;
                let language = lang::for_path(&req.path).to_string();

                // Over the content actually returned, so the version and the
                // bytes the caller holds always describe each other — a
                // truncated read is versioned as the prefix it is.
                let version = content_version(&file.content);

                // A truncated read must not become a buffer: saving it back
                // would delete the rest of the file, and the guard for that
                // belongs at the door rather than in every caller.
                if !file.truncated {
                    let mut session = load_session(&bus, &root).await;
                    session.upsert(Buffer {
                        path: req.path.clone(),
                        mtime: file.mtime,
                        version: version.clone(),
                        language: language.clone(),
                    });
                    save_session(&bus, &root, &session).await?;
                }

                Ok::<_, Error>(OpenOutput {
                    path: req.path,
                    content: file.content,
                    language,
                    size: file.size,
                    mtime: file.mtime,
                    version,
                    truncated: file.truncated,
                })
            }
        })
        .description(DESC_OPEN),
    );
}

fn register_save(iii: &Arc<IIIClient>, cfg: &ConfigCell, bus: &Arc<Bus>) {
    let cfg = cfg.clone();
    let bus = bus.clone();
    iii.register_function(
        "editor::save",
        RegisterFunction::new_async(move |req: SaveInput| {
            let cfg = cfg.clone();
            let bus = bus.clone();
            async move {
                let cfg = cfg.read().await.clone();
                save(&bus, &cfg, req).await
            }
        })
        .description(DESC_SAVE),
    );
}

/// Opaque content version: byte length and a 64-bit FNV-1a digest of the bytes.
///
/// Deliberately not a cryptographic hash. This answers "is this still the
/// content I read", a question whose worst outcome is a refused save, and it is
/// not an authorization decision — the filesystem boundary is `shell`'s jail.
/// Qualifying the digest with the length makes an accidental collision
/// implausible for the sizes an editor deals in, and computing it costs nothing
/// extra because `save` has already read the file to count the lines it
/// changed. No hashing dependency is added for it.
///
/// The encoding is private: callers compare for equality and never parse it.
fn content_version(content: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for byte in content.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{:x}-{:016x}", content.len(), hash)
}

/// Whether the file on disk diverged from the copy this save started from.
///
/// Two guards, and the caller picks which by the field it sends.
///
/// `expected_version` is the strong one. An mtime cannot see a write that
/// landed inside the same second, and a same-second write is exactly how one
/// agent silently overwrites another. A nanosecond mtime would fix that too,
/// but this worker cannot get one: the timestamps it sees come from
/// `shell::fs::stat`, which reports whole seconds, and `editor` owns no
/// filesystem access to look closer. A content version also survives what a
/// timestamp does not — a clock stepping backwards, a checkout or a restore
/// rewriting mtimes, a filesystem with coarse granularity — and here it is the
/// cheaper of the two, because the read it needs has already happened.
///
/// `expected_mtime` is the original guard, kept bit for bit: a caller that
/// knows nothing about versions behaves exactly as it did before.
fn conflicted(
    expected_version: Option<&str>,
    expected_mtime: Option<i64>,
    disk_version: &str,
    disk_mtime: i64,
) -> bool {
    match (expected_version, expected_mtime) {
        (Some(expected), _) => expected != disk_version,
        (None, Some(expected)) => expected != disk_mtime,
        // Neither guard armed: an overwrite the caller asked for.
        (None, None) => false,
    }
}

/// The conflict guard, split out so its branches stay readable.
///
/// Ordering matters: the pre-write read happens *before* the write, because it
/// is what the response's `added`/`removed` describe and what the content
/// version is taken over. Doing it after would diff the file against itself and
/// report every save as a no-op.
async fn save(bus: &Bus, cfg: &WorkerConfig, req: SaveInput) -> Result<SaveOutput, Error> {
    let root = active_root(bus).await;
    let full = joined(&root, &req.path);
    let existing = bus.stat(&full).await.ok();

    // One read, shared by the guard and the line counts. Every save of an
    // existing file already had to read it once; reading here rather than in
    // each branch keeps it at once now that the guard needs the bytes too.
    let on_disk = match existing.as_ref() {
        Some(_) => Some(bus.read(&full, cfg.max_file_bytes).await?),
        None => None,
    };
    let disk_version = on_disk.as_ref().map(|f| content_version(&f.content));

    if let (Some(stat), Some(disk), Some(version)) =
        (existing.as_ref(), on_disk.as_ref(), disk_version.as_deref())
    {
        if conflicted(
            req.expected_version.as_deref(),
            req.expected_mtime,
            version,
            stat.mtime,
        ) {
            let divergence = diff::diff(
                &disk.content,
                &req.content,
                Some(&req.path),
                cfg.diff_context_lines,
                cfg.max_diff_bytes,
            );
            return Ok(SaveOutput {
                path: req.path,
                saved: false,
                conflict: true,
                mtime: stat.mtime,
                version: version.to_string(),
                disk_mtime: Some(stat.mtime),
                disk_version: Some(version.to_string()),
                conflict_patch: Some(divergence.patch),
                added: divergence.added,
                removed: divergence.removed,
                created: false,
            });
        }
    }

    // A guard token for a file that is not there means it was deleted under the
    // editor. Recreating it silently would undo that deletion.
    if (req.expected_mtime.is_some() || req.expected_version.is_some()) && existing.is_none() {
        return Err(Error::Handler(format!(
            "{} no longer exists; re-open it before saving",
            req.path
        )));
    }

    let previous: Option<DiffResult> = match on_disk.as_ref() {
        Some(before) => {
            if before.truncated {
                return Err(Error::Handler(format!(
                    "{} is larger than max_file_bytes; saving would truncate it",
                    req.path
                )));
            }
            Some(diff::diff(
                &before.content,
                &req.content,
                Some(&req.path),
                cfg.diff_context_lines,
                cfg.max_diff_bytes,
            ))
        }
        None => None,
    };

    bus.write(&full, &req.content).await?;
    let after = bus.stat(&full).await?;
    let version = content_version(&req.content);

    // The buffer's mtime and version are now stale everywhere else; refreshing
    // them here is what lets a second surface save next without a spurious
    // conflict.
    let mut session = load_session(bus, &root).await;
    session.upsert(Buffer {
        path: req.path.clone(),
        mtime: after.mtime,
        version: version.clone(),
        language: lang::for_path(&req.path).to_string(),
    });
    save_session(bus, &root, &session).await?;

    Ok(SaveOutput {
        // A file that did not exist is entirely added — reporting 0 there would
        // make "created a 400-line file" indistinguishable from a no-op.
        added: previous
            .as_ref()
            .map(|d| d.added)
            .unwrap_or_else(|| req.content.lines().count() as u32),
        removed: previous.as_ref().map(|d| d.removed).unwrap_or(0),
        path: req.path,
        saved: true,
        conflict: false,
        mtime: after.mtime,
        version,
        disk_mtime: None,
        disk_version: None,
        conflict_patch: None,
        created: existing.is_none(),
    })
}

// -------------------------------------------------------------------- find

fn register_find(iii: &Arc<IIIClient>, cfg: &ConfigCell, bus: &Arc<Bus>) {
    let cfg = cfg.clone();
    let bus = bus.clone();
    iii.register_function(
        "editor::find",
        RegisterFunction::new_async(move |req: FindInput| {
            let cfg = cfg.clone();
            let bus = bus.clone();
            async move {
                let cfg = cfg.read().await.clone();
                let root = active_root(&bus).await;
                let found = candidates(&bus, &root, &req, &cfg).await?;
                let refs: Vec<&str> = found.paths.iter().map(String::as_str).collect();
                let limit = req.limit.map(|l| l as usize).unwrap_or(cfg.find_limit);
                let ranked = fuzzy::rank(&req.query, &refs, limit);

                Ok::<_, Error>(FindOutput {
                    matches: ranked
                        .into_iter()
                        .map(|(path, m)| FindMatch {
                            path: path.to_string(),
                            score: m.score,
                            positions: m.positions.into_iter().map(|p| p as u32).collect(),
                        })
                        .collect(),
                    scanned: refs.len() as u32,
                    truncated: found.truncated,
                    from_git: found.from_git,
                })
            }
        })
        .description(DESC_FIND),
    );
}

/// Paths to rank, where they came from, and whether they are all of them.
struct Candidates {
    paths: Vec<String>,
    /// True when git's listing supplied them, so `.gitignore` was honoured.
    from_git: bool,
    /// True when the workspace held more paths than were collected — the
    /// candidate cap, or the folder walk's visit budget. Either way the ranking
    /// is over a prefix of the workspace and the response has to say so.
    truncated: bool,
}

/// git's listing is preferred where it exists — it already honours
/// `.gitignore`, which is most of what makes a picker usable in a real
/// checkout. A plain folder is not a degraded case, just a different source:
/// the workspace listing, which shell already excludes noise from.
async fn candidates(
    bus: &Bus,
    root: &str,
    req: &FindInput,
    cfg: &WorkerConfig,
) -> Result<Candidates, Error> {
    let mut args = vec!["ls-files", "--cached"];
    if req.include_untracked {
        args.push("--others");
        args.push("--exclude-standard");
    }
    let git = bus.git(&args, Some(root)).await?;
    if git.exit_code == Some(0) {
        let mut paths: Vec<String> = git
            .stdout
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();
        paths.truncate(cfg.max_find_candidates);
        return Ok(Candidates {
            truncated: paths.len() >= cfg.max_find_candidates,
            paths,
            from_git: true,
        });
    }

    let listing = bus.tree(root, 12).await?;
    let walked = tree::walk(&listing, cfg.max_find_candidates);
    Ok(Candidates {
        truncated: walked.truncated || walked.paths.len() >= cfg.max_find_candidates,
        paths: walked.paths,
        from_git: false,
    })
}

// -------------------------------------------------------------------- diff

fn register_diff(iii: &Arc<IIIClient>, cfg: &ConfigCell) {
    let cfg = cfg.clone();
    iii.register_function(
        "editor::diff",
        RegisterFunction::new_async(move |req: DiffInput| {
            let cfg = cfg.clone();
            async move {
                let cfg = cfg.read().await.clone();
                let context = req
                    .context_lines
                    .map(|c| c as usize)
                    .unwrap_or(cfg.diff_context_lines);
                Ok::<_, Error>(diff::diff(
                    &req.before,
                    &req.after,
                    req.path.as_deref(),
                    context,
                    cfg.max_diff_bytes,
                ))
            }
        })
        .description(DESC_DIFF),
    );
}

// --------------------------------------------------------------------- git

fn register_git_status(iii: &Arc<IIIClient>, bus: &Arc<Bus>) {
    let bus = bus.clone();
    iii.register_function(
        "editor::git::status",
        RegisterFunction::new_async(move |req: GitStatusInput| {
            let bus = bus.clone();
            async move {
                let cwd = match req.cwd {
                    Some(dir) => dir,
                    None => active_root(&bus).await,
                };
                let out = bus
                    .git_ok(
                        &[
                            "status",
                            "--porcelain=v2",
                            "--branch",
                            "--untracked-files=all",
                        ],
                        Some(&cwd),
                    )
                    .await?;
                Ok::<_, Error>(parse_status(&out.stdout))
            }
        })
        .description(DESC_GIT_STATUS),
    );
}

fn register_git_hunks(iii: &Arc<IIIClient>, cfg: &ConfigCell, bus: &Arc<Bus>) {
    let cfg = cfg.clone();
    let bus = bus.clone();
    iii.register_function(
        "editor::git::hunks",
        RegisterFunction::new_async(move |req: GitHunksInput| {
            let cfg = cfg.clone();
            let bus = bus.clone();
            async move {
                let cfg = cfg.read().await.clone();
                let cwd = match req.cwd {
                    Some(dir) => dir,
                    None => active_root(&bus).await,
                };
                // Defaults to zero, and that is load-bearing: `Hunk` is
                // documented as the ranges a gutter paints, and any context at
                // all widens them past the lines that actually changed. A
                // caller who wants a readable patch asks for context
                // explicitly and accepts the wider ranges that come with it.
                let context = format!("-U{}", req.context_lines.unwrap_or(0));
                let mut args = vec!["diff", &context, "--no-color"];
                match req.against {
                    Against::Worktree => {}
                    Against::Index => args.push("--cached"),
                    Against::Head => args.push("HEAD"),
                    // `@{upstream}` is git's own name for it, so this works on
                    // any branch without the caller knowing the remote or the
                    // branch name.
                    Against::Upstream => args.push("@{upstream}"),
                }
                args.push("--");
                args.push(&req.path);

                let out = bus.git_ok(&args, Some(&cwd)).await?;
                let hunks = parse_hunk_headers(&out.stdout);

                // An untracked file produces no diff at all, which is
                // indistinguishable from "unchanged" unless we ask.
                let untracked = if hunks.is_empty() {
                    let probe = bus
                        .git(
                            &["ls-files", "--error-unmatch", "--", &req.path],
                            Some(&cwd),
                        )
                        .await?;
                    probe.exit_code != Some(0)
                } else {
                    false
                };

                let added = hunks.iter().map(|h| h.added).sum();
                let removed = hunks.iter().map(|h| h.removed).sum();
                // Bounded like every other patch this worker emits: a huge
                // diff must not become an unbounded response.
                // `String::truncate` panics on a non-boundary index, and a
                // byte cap lands mid-codepoint the moment a diff contains any
                // multibyte character. Cut back to the last boundary at or
                // before the cap.
                let patch = truncate_on_boundary(out.stdout, cfg.max_diff_bytes);
                Ok::<_, Error>(GitHunksOutput {
                    path: req.path,
                    against: req.against,
                    hunks,
                    added,
                    removed,
                    untracked,
                    patch,
                })
            }
        })
        .description(DESC_GIT_HUNKS),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joined_leaves_absolute_paths_alone() {
        assert_eq!(joined("/repo", "/etc/hosts"), "/etc/hosts");
    }

    #[test]
    fn joined_uses_shell_cwd_when_no_root_is_set() {
        assert_eq!(joined(".", "src/main.rs"), "src/main.rs");
    }

    #[test]
    fn joined_prefixes_the_root() {
        assert_eq!(joined("/repo", "src/main.rs"), "/repo/src/main.rs");
    }

    #[test]
    fn function_ids_are_unique_and_namespaced() {
        let ids = function_ids();
        let mut seen = std::collections::HashSet::new();
        for id in &ids {
            assert!(
                id.starts_with("editor::"),
                "{id} is outside the worker namespace"
            );
            assert!(
                !id.contains('_'),
                "{id} uses snake_case; ids are kebab-case"
            );
            assert!(seen.insert(*id), "{id} is registered twice");
        }
    }

    #[test]
    fn a_content_version_is_stable_and_content_addressed() {
        assert_eq!(
            content_version("fn main() {}"),
            content_version("fn main() {}")
        );
        assert_ne!(content_version("a\nb\n"), content_version("a\nB\n"));
        // Same length, transposed bytes: the digest, not the length, has to
        // carry this.
        assert_ne!(content_version("ab"), content_version("ba"));
        // Length is part of the token, so a prefix can never collide with the
        // whole.
        assert_ne!(content_version(""), content_version("\0"));
    }

    /// The bug this guard exists for: a second write inside the same
    /// filesystem second leaves `mtime` unchanged, so the mtime guard waves it
    /// through and the save silently overwrites the other writer.
    #[test]
    fn a_same_second_write_is_caught_by_the_version_and_missed_by_the_mtime() {
        let started_from = content_version("original\n");
        let now_on_disk = content_version("someone else's edit\n");
        let same_second = 1_700_000_000;

        assert!(
            conflicted(Some(&started_from), None, &now_on_disk, same_second),
            "the version guard must refuse a write over content it did not read"
        );
        assert!(
            !conflicted(None, Some(same_second), &now_on_disk, same_second),
            "documents the mtime guard's blind spot: it cannot see this at all"
        );
        // Both sent: the version decides, so a caller that sends the stale
        // mtime alongside is still protected.
        assert!(conflicted(
            Some(&started_from),
            Some(same_second),
            &now_on_disk,
            same_second
        ));
    }

    #[test]
    fn a_matching_version_is_not_a_conflict() {
        let v = content_version("unchanged\n");
        assert!(!conflicted(Some(&v), None, &v, 1_700_000_000));
        // An mtime that moved without the content changing is not a divergence
        // for a version-guarded caller — a touch or a rewrite of identical
        // bytes is nothing to refuse.
        assert!(!conflicted(Some(&v), Some(1), &v, 999));
    }

    /// The legacy path, unchanged: mtime only, compared exactly as before.
    #[test]
    fn the_mtime_guard_still_applies_when_no_version_is_sent() {
        let v = content_version("whatever\n");
        assert!(!conflicted(None, Some(42), &v, 42));
        assert!(conflicted(None, Some(41), &v, 42));
    }

    #[test]
    fn no_guard_token_means_a_deliberate_overwrite() {
        assert!(!conflicted(
            None,
            None,
            &content_version("anything"),
            1_700_000_000
        ));
    }

    /// The catalog is what ships to the registry; a function registered but
    /// left out of it would have no published schema.
    #[test]
    fn every_registered_function_is_in_the_catalog() {
        let cataloged: Vec<&str> = crate::surface::catalog()
            .iter()
            .map(|s| s.function_id)
            .collect();
        assert_eq!(function_ids(), cataloged);
    }
}

// ------------------------------------------------------------------ file ops

fn register_create(iii: &Arc<IIIClient>, bus: &Arc<Bus>) {
    let bus = bus.clone();
    iii.register_function(
        "editor::create",
        RegisterFunction::new_async(move |req: CreateInput| {
            let bus = bus.clone();
            async move {
                let root = active_root(&bus).await;
                let full = joined(&root, &req.path);
                match req.kind {
                    EntryKind::Folder => bus.mkdir(&full).await?,
                    EntryKind::File => {
                        // The parent may not exist yet, and shell::fs::write
                        // does not create one, so ask for it first.
                        if let Some((parent, _)) = req.path.rsplit_once('/') {
                            bus.mkdir(&joined(&root, parent)).await?;
                        }
                        bus.write(&full, req.content.as_deref().unwrap_or(""))
                            .await?;
                    }
                }
                Ok::<_, Error>(CreateOutput {
                    path: req.path,
                    kind: req.kind,
                    created: true,
                })
            }
        })
        .description(DESC_CREATE),
    );
}

fn register_delete(iii: &Arc<IIIClient>, bus: &Arc<Bus>) {
    let bus = bus.clone();
    iii.register_function(
        "editor::delete",
        RegisterFunction::new_async(move |req: DeleteInput| {
            let bus = bus.clone();
            async move {
                let root = active_root(&bus).await;
                bus.rm(&joined(&root, &req.path), req.recursive).await?;

                // Close buffers for anything that is now gone. A folder delete
                // takes every buffer beneath it, not just an exact match — a
                // buffer left open would recreate the file on its next save.
                let mut session = load_session(&bus, &root).await;
                let prefix = format!("{}/", req.path);
                let closed: Vec<String> = session
                    .buffers
                    .iter()
                    .map(|b| b.path.clone())
                    .filter(|p| p == &req.path || p.starts_with(&prefix))
                    .collect();
                for path in &closed {
                    session.close(path);
                }
                // `collapse` mutates `expanded` as well, so a delete that closed
                // no buffers can still have changed the session. Compare the
                // whole record rather than gating on the buffer list.
                let before = session.clone();
                session.collapse(&req.path);
                if !closed.is_empty() || session != before {
                    save_session(&bus, &root, &session).await?;
                }

                Ok::<_, Error>(DeleteOutput {
                    path: req.path,
                    deleted: true,
                    buffers_closed: closed,
                    buffers: session.buffers,
                })
            }
        })
        .description(DESC_DELETE),
    );
}

// -------------------------------------------------------------------- search

fn register_search(iii: &Arc<IIIClient>, cfg: &ConfigCell, bus: &Arc<Bus>) {
    let cfg = cfg.clone();
    let bus = bus.clone();
    iii.register_function(
        "editor::search",
        RegisterFunction::new_async(move |req: SearchInput| {
            let cfg = cfg.clone();
            let bus = bus.clone();
            async move {
                let cfg = cfg.read().await.clone();
                let root = active_root(&bus).await;
                let max = req.max_matches.unwrap_or(cfg.search_max_matches);
                let raw = bus
                    .grep(&root, &req.pattern, req.ignore_case, &req.include_glob, max)
                    .await?;
                Ok::<_, Error>(group_matches(&raw, &root))
            }
        })
        .description(DESC_SEARCH),
    );
}

/// Group shell's flat match list by file, in first-match order.
///
/// Order is preserved rather than sorted: a results panel reads top-down, and
/// re-ordering between keystrokes makes the list impossible to follow.
///
/// Paths come back root-relative. shell answers with absolute ones, and every
/// other function on this worker speaks root-relative — a caller should never
/// have to know which of the two a given response used.
fn group_matches(raw: &serde_json::Value, root: &str) -> SearchOutput {
    let mut files: Vec<SearchFile> = Vec::new();
    let mut total = 0u32;

    let matches = raw
        .get("matches")
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    for m in matches {
        let (Some(path), Some(line)) = (
            m.get("path").and_then(|v| v.as_str()),
            m.get("line").and_then(|v| v.as_u64()),
        ) else {
            continue;
        };
        let text = m
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let path = relative_to(path, root);
        total += 1;
        match files.iter_mut().find(|f| f.path == path) {
            Some(file) => file.hits.push(SearchHit { line, text }),
            None => files.push(SearchFile {
                path,
                hits: vec![SearchHit { line, text }],
            }),
        }
    }

    SearchOutput {
        files,
        total,
        truncated: raw
            .get("truncated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    }
}

// --------------------------------------------------------------- git writes

/// Strip the workspace root from an absolute path, leaving anything that does
/// not sit under it untouched.
fn relative_to(path: &str, root: &str) -> String {
    if root == "." {
        return path.to_string();
    }
    let trimmed = root.trim_end_matches('/');
    path.strip_prefix(trimmed)
        .map(|rest| rest.trim_start_matches('/'))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// Cut a string to at most `max_bytes`, never mid-codepoint.
///
/// `String::truncate` panics on a non-boundary index, and a byte cap lands
/// inside a multibyte character the moment a diff contains one. Cutting back to
/// the previous boundary loses at most three bytes of an already-truncated
/// patch, which is the right trade against a panicking handler.
fn truncate_on_boundary(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    let mut cut = max_bytes;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    text
}

/// Ahead/behind straight from git, for the sync response.
async fn ahead_behind(bus: &Bus, cwd: &str) -> (u32, u32) {
    let Ok(out) = bus
        .git(
            &[
                "status",
                "--porcelain=v2",
                "--branch",
                "--untracked-files=no",
            ],
            Some(cwd),
        )
        .await
    else {
        return (0, 0);
    };
    let report = parse_status(&out.stdout);
    (report.ahead, report.behind)
}

/// Both streams, trimmed. git reports progress on stderr even on success, so
/// taking stdout alone renders a successful push as an empty response.
fn summarize(out: &crate::bus::ExecOutcome) -> String {
    let mut text = out.stdout.trim().to_string();
    let err = out.stderr.trim();
    if !err.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(err);
    }
    text
}

fn register_git_commit(iii: &Arc<IIIClient>, bus: &Arc<Bus>) {
    let bus = bus.clone();
    iii.register_function(
        "editor::git::commit",
        RegisterFunction::new_async(move |req: GitCommitInput| {
            let bus = bus.clone();
            async move {
                let cwd = match req.cwd {
                    Some(dir) => dir,
                    None => active_root(&bus).await,
                };
                if req.stage_all {
                    bus.git_ok(&["add", "-A"], Some(&cwd)).await?;
                }
                let out = bus.git(&["commit", "-m", &req.message], Some(&cwd)).await?;

                // "nothing to commit" exits non-zero but is not a failure: a
                // commit button pressed on a clean tree should say so, not raise.
                if out.exit_code != Some(0) {
                    let text = summarize(&out);
                    if text.contains("nothing to commit") || text.contains("no changes added") {
                        return Ok::<_, Error>(GitCommitOutput {
                            committed: false,
                            sha: None,
                            summary: text,
                        });
                    }
                    return Err(Error::Handler(format!("git commit failed: {text}")));
                }

                let sha = bus
                    .git(&["rev-parse", "HEAD"], Some(&cwd))
                    .await
                    .ok()
                    .filter(|o| o.exit_code == Some(0))
                    .map(|o| o.stdout.trim().to_string());

                Ok::<_, Error>(GitCommitOutput {
                    committed: true,
                    sha,
                    summary: summarize(&out),
                })
            }
        })
        .description(DESC_GIT_COMMIT),
    );
}

fn register_git_sync(iii: &Arc<IIIClient>, bus: &Arc<Bus>) {
    let bus = bus.clone();
    iii.register_function(
        "editor::git::sync",
        RegisterFunction::new_async(move |req: GitSyncInput| {
            let bus = bus.clone();
            async move {
                let cwd = match req.cwd {
                    Some(dir) => dir,
                    None => active_root(&bus).await,
                };
                let args: Vec<&str> = match req.action {
                    SyncAction::Fetch => vec!["fetch"],
                    SyncAction::Pull => vec!["pull", "--ff-only"],
                    SyncAction::Push => vec!["push"],
                };
                let out = bus.git(&args, Some(&cwd)).await?;
                let (ahead, behind) = ahead_behind(&bus, &cwd).await;
                Ok::<_, Error>(GitSyncOutput {
                    action: req.action,
                    ok: out.exit_code == Some(0),
                    summary: summarize(&out),
                    ahead,
                    behind,
                })
            }
        })
        .description(DESC_GIT_SYNC),
    );
}

fn register_git_stash(iii: &Arc<IIIClient>, bus: &Arc<Bus>) {
    let bus = bus.clone();
    iii.register_function(
        "editor::git::stash",
        RegisterFunction::new_async(move |req: GitStashInput| {
            let bus = bus.clone();
            async move {
                let cwd = match req.cwd {
                    Some(dir) => dir,
                    None => active_root(&bus).await,
                };
                let args: Vec<&str> = match req.action {
                    StashAction::Push => vec!["stash", "push"],
                    StashAction::Pop => vec!["stash", "pop"],
                };
                let out = bus.git(&args, Some(&cwd)).await?;
                Ok::<_, Error>(GitStashOutput {
                    action: req.action,
                    ok: out.exit_code == Some(0),
                    summary: summarize(&out),
                })
            }
        })
        .description(DESC_GIT_STASH),
    );
}

fn register_git_undo_commit(iii: &Arc<IIIClient>, bus: &Arc<Bus>) {
    let bus = bus.clone();
    iii.register_function(
        "editor::git::undo-commit",
        RegisterFunction::new_async(move |req: GitUndoCommitInput| {
            let bus = bus.clone();
            async move {
                let cwd = match req.cwd {
                    Some(dir) => dir,
                    None => active_root(&bus).await,
                };
                // Read the commit BEFORE undoing it: afterwards HEAD points at
                // its parent and neither the sha nor the message is recoverable
                // for the response.
                let sha = bus.git_ok(&["rev-parse", "HEAD"], Some(&cwd)).await?;
                let message = bus
                    .git_ok(&["log", "-1", "--pretty=%B"], Some(&cwd))
                    .await?;
                bus.git_ok(&["reset", "--soft", "HEAD~1"], Some(&cwd))
                    .await?;
                Ok::<_, Error>(GitUndoCommitOutput {
                    undone_sha: sha.stdout.trim().to_string(),
                    message: message.stdout.trim().to_string(),
                })
            }
        })
        .description(DESC_GIT_UNDO_COMMIT),
    );
}

#[cfg(test)]
mod parity_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn matches_group_by_file_in_first_match_order() {
        let raw = json!({
            "matches": [
                { "path": "b.rs", "line": 3, "content": "hit one" },
                { "path": "a.rs", "line": 9, "content": "hit two" },
                { "path": "b.rs", "line": 41, "content": "hit three" }
            ],
            "truncated": false
        });
        let out = group_matches(&raw, ".");
        assert_eq!(
            out.files
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            vec!["b.rs", "a.rs"],
            "file order follows first match, never sorted"
        );
        assert_eq!(out.files[0].hits.len(), 2);
        assert_eq!(out.files[0].hits[1].line, 41);
        assert_eq!(out.total, 3);
        assert!(!out.truncated);
    }

    #[test]
    fn truncation_is_carried_through() {
        let raw = json!({ "matches": [], "truncated": true });
        assert!(group_matches(&raw, ".").truncated);
    }

    #[test]
    fn a_malformed_match_is_skipped_not_fatal() {
        let raw = json!({
            "matches": [ { "path": "a.rs" }, { "path": "a.rs", "line": 2 } ]
        });
        let out = group_matches(&raw, ".");
        assert_eq!(out.total, 1, "the entry without a line is dropped");
        assert_eq!(out.files[0].hits[0].text, "", "missing content reads empty");
    }

    #[test]
    fn search_paths_come_back_root_relative() {
        let raw = json!({
            "matches": [ { "path": "/repo/src/a.rs", "line": 1, "content": "x" } ]
        });
        assert_eq!(group_matches(&raw, "/repo").files[0].path, "src/a.rs");
    }

    #[test]
    fn a_path_outside_the_root_is_left_alone() {
        let raw = json!({
            "matches": [ { "path": "/elsewhere/a.rs", "line": 1, "content": "x" } ]
        });
        assert_eq!(
            group_matches(&raw, "/repo").files[0].path,
            "/elsewhere/a.rs"
        );
    }

    /// The case that used to panic: a cap landing inside a multibyte char.
    #[test]
    fn truncate_never_splits_a_codepoint() {
        // "é" is two bytes, so a cap of 2 lands inside the second one.
        let text = "aé".to_string();
        assert_eq!(text.len(), 3);
        assert_eq!(truncate_on_boundary(text.clone(), 2), "a");
        // A cap on a boundary keeps everything up to it.
        assert_eq!(truncate_on_boundary(text.clone(), 3), "aé");
        // Under the cap is returned untouched.
        assert_eq!(truncate_on_boundary(text, 99), "aé");
    }

    #[test]
    fn truncate_handles_a_cap_of_zero() {
        assert_eq!(truncate_on_boundary("é".to_string(), 0), "");
    }

    /// A patch of nothing but multibyte characters must still cut cleanly.
    #[test]
    fn truncate_survives_an_all_multibyte_string() {
        let text = "→→→→".to_string();
        for cap in 0..=text.len() {
            let cut = truncate_on_boundary(text.clone(), cap);
            assert!(cut.len() <= cap, "cap {cap} overshot");
            assert!(text.starts_with(&cut), "cap {cap} produced a non-prefix");
        }
    }

    #[test]
    fn summarize_keeps_stderr_when_stdout_is_empty() {
        let out = crate::bus::ExecOutcome {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: "Everything up-to-date".to_string(),
            timed_out: false,
            stdout_truncated: false,
        };
        assert_eq!(summarize(&out), "Everything up-to-date");
    }

    #[test]
    fn summarize_joins_both_streams() {
        let out = crate::bus::ExecOutcome {
            exit_code: Some(0),
            stdout: "out".to_string(),
            stderr: "err".to_string(),
            timed_out: false,
            stdout_truncated: false,
        };
        assert_eq!(summarize(&out), "out\nerr");
    }
}

fn register_git_show(iii: &Arc<IIIClient>, bus: &Arc<Bus>) {
    let bus = bus.clone();
    iii.register_function(
        "editor::git::show",
        RegisterFunction::new_async(move |req: GitShowInput| {
            let bus = bus.clone();
            async move {
                let cwd = match req.cwd {
                    Some(dir) => dir,
                    None => active_root(&bus).await,
                };
                let rev = req.rev.unwrap_or_else(|| "HEAD".to_string());
                let spec = format!("{rev}:{}", req.path);
                let out = bus.git(&["show", &spec], Some(&cwd)).await?;

                // A path absent at that revision is an answer, not a failure:
                // it is exactly what a newly added file looks like.
                if out.exit_code != Some(0) {
                    return Ok::<_, Error>(GitShowOutput {
                        path: req.path,
                        rev,
                        content: String::new(),
                        exists: false,
                    });
                }
                Ok::<_, Error>(GitShowOutput {
                    path: req.path,
                    rev,
                    content: out.stdout,
                    exists: true,
                })
            }
        })
        .description(DESC_GIT_SHOW),
    );
}
