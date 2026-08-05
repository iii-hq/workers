//! Wire types for every registered function.
//!
//! Kept in one module because the functions share a small vocabulary (`Hunk`,
//! path, mtime, content version) and splitting them one file per function would
//! mean a copy of the same doc comments — what an mtime is for, what a version
//! is for — in every one of them.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::diff::Hunk;
use crate::workspace::Buffer;

// ----------------------------------------------------- editor::workspace::*

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorkspaceOpenInput {
    /// Directory to work in. Jail-relative when shell's `fs.host_roots` are
    /// set, else absolute. A plain folder is enough — a git repository is an
    /// overlay, never a requirement.
    pub root: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct EmptyInput {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WorkspaceView {
    /// The active root every other path in this response is relative to.
    pub root: String,
    /// Files currently open against this root, shared by every surface.
    pub buffers: Vec<Buffer>,
    /// Folders expanded in the tree, root-relative.
    pub expanded: Vec<String>,
}

// ---------------------------------------------------------------- editor::tree

/// Create a file or a folder.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EntryKind {
    File,
    Folder,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateInput {
    /// Root-relative path to create. Missing parent folders are created.
    pub path: String,
    /// What to create. Defaults to a file.
    #[serde(default = "default_file_kind")]
    pub kind: EntryKind,
    /// Initial contents for a file. Ignored for a folder.
    #[serde(default)]
    pub content: Option<String>,
}

fn default_file_kind() -> EntryKind {
    EntryKind::File
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreateOutput {
    pub path: String,
    pub kind: EntryKind,
    /// Always true on success; the call errors rather than reporting false.
    pub created: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteInput {
    /// Root-relative path to remove.
    pub path: String,
    /// Required to remove a non-empty folder.
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeleteOutput {
    pub path: String,
    pub deleted: bool,
    /// Buffers closed because their file is gone. Leaving them open would let
    /// the next save recreate a file the user just deleted.
    pub buffers_closed: Vec<String>,
    pub buffers: Vec<Buffer>,
}

// -------------------------------------------------------------- editor::search

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchInput {
    /// Rust regex matched against each line.
    pub pattern: String,
    /// Match case-insensitively.
    #[serde(default)]
    pub ignore_case: bool,
    /// Glob filters restricting which files are searched, e.g. `["**/*.rs"]`.
    #[serde(default)]
    pub include_glob: Vec<String>,
    /// Stop after this many matching lines. Defaults to the worker's
    /// `search_max_matches`.
    #[serde(default)]
    pub max_matches: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchHit {
    /// 1-based line number.
    pub line: u64,
    /// The matching line, as shell returned it.
    pub text: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchFile {
    pub path: String,
    pub hits: Vec<SearchHit>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchOutput {
    /// Matches grouped by file, in first-match order — the shape a result
    /// panel renders, rather than a flat list every caller has to group.
    pub files: Vec<SearchFile>,
    /// Total matching lines across every file.
    pub total: u32,
    /// True when the search stopped at `max_matches`.
    pub truncated: bool,
}

// ------------------------------------------------------------- editor::git::*

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GitCommitInput {
    /// Commit message. Passed as a single `-m` argument.
    pub message: String,
    /// Stage every change first (`git add -A`). On by default, matching what
    /// an editor's commit command does; pass false to commit only the index.
    #[serde(default = "default_true")]
    pub stage_all: bool,
    /// Repository to run in. Defaults to the workspace root.
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GitCommitOutput {
    /// False when there was nothing staged to commit.
    pub committed: bool,
    /// Full SHA of the new commit, when one was made.
    pub sha: Option<String>,
    /// git's own summary line, verbatim.
    pub summary: String,
}

/// Which remote operation to run.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SyncAction {
    Fetch,
    /// Fast-forward only. A pull that would merge fails instead, so this can
    /// never produce a conflicted tree under open buffers.
    Pull,
    Push,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GitSyncInput {
    /// Which remote operation to run.
    pub action: SyncAction,
    /// Repository to run in. Defaults to the workspace root.
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GitSyncOutput {
    pub action: SyncAction,
    pub ok: bool,
    /// git's output, trimmed. Both streams: git reports progress on stderr.
    pub summary: String,
    /// Ahead/behind after the operation, so a caller does not need a second
    /// round trip to find out whether it changed anything.
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StashAction {
    Push,
    Pop,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GitStashInput {
    /// Stash the working tree, or restore the most recent stash.
    pub action: StashAction,
    /// Repository to run in. Defaults to the workspace root.
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GitStashOutput {
    pub action: StashAction,
    pub ok: bool,
    pub summary: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct GitUndoCommitInput {
    /// Repository to run in. Defaults to the workspace root.
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GitUndoCommitOutput {
    /// SHA that was undone.
    pub undone_sha: String,
    /// Its message, so a caller can put it straight back in a commit box.
    pub message: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct TreeInput {
    /// Folder to list, relative to the workspace root. Defaults to the root.
    #[serde(default)]
    pub path: Option<String>,
    /// Levels to descend. Defaults to 4 — deep enough to navigate, shallow
    /// enough that one call does not walk a whole monorepo.
    #[serde(default)]
    pub max_depth: Option<u32>,
    /// Root-relative folders to mark expanded before listing. Expansion is
    /// part of the shared workspace, so it survives a reload and both surfaces
    /// agree on it.
    #[serde(default)]
    pub expand: Vec<String>,
    /// Root-relative folders to collapse. Collapsing takes its descendants
    /// with it.
    #[serde(default)]
    pub collapse: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TreeOutput {
    pub root: String,
    /// Canonical absolute path of the listed folder, as shell resolved it.
    pub path: String,
    /// The listing exactly as the shell worker returned it (nested
    /// `{name, kind, size, mtime, children}` nodes). Passed through rather
    /// than re-modelled so this worker does not pin shell's response shape.
    pub tree: serde_json::Value,
    /// Folders the workspace has expanded, root-relative.
    pub expanded: Vec<String>,
}

// ------------------------------------------------------------ editor::buffers::*

#[derive(Debug, Serialize, JsonSchema)]
pub struct BuffersOutput {
    pub root: String,
    pub buffers: Vec<Buffer>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BufferCloseInput {
    /// Root-relative path of the buffer to close.
    pub path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BufferCloseOutput {
    /// False when nothing was open at that path.
    pub closed: bool,
    pub root: String,
    pub buffers: Vec<Buffer>,
}

// ---------------------------------------------------------------- editor::move

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveInput {
    /// Existing root-relative path.
    pub from: String,
    /// Destination root-relative path.
    pub to: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MoveOutput {
    pub from: String,
    pub to: String,
    /// Open buffers and expanded folders rewritten to the new location. A
    /// folder move rewrites everything beneath it, which is the whole reason
    /// moves go through this function instead of `shell::fs::mv` directly.
    pub remapped: u32,
    pub root: String,
    pub buffers: Vec<Buffer>,
}

// ---------------------------------------------------------------- editor::diff

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiffInput {
    /// The text as it was.
    pub before: String,
    /// The text as it will be.
    pub after: String,
    /// Path used to label the patch header. Nothing is read from disk — this
    /// is presentation only.
    #[serde(default)]
    pub path: Option<String>,
    /// Unchanged lines kept around each hunk (`-U` of `git diff`). Defaults to
    /// the worker's `diff_context_lines`.
    #[serde(default)]
    pub context_lines: Option<u32>,
}

// ----------------------------------------------------------- editor::git::status

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct GitStatusInput {
    /// Directory to run git in. Defaults to the workspace root. Confined by
    /// shell's jail exactly like any other `shell::exec` call.
    #[serde(default)]
    pub cwd: Option<String>,
}

// ------------------------------------------------------------ editor::git::hunks

/// Which copy of the file the working tree is compared against.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Against {
    /// Unstaged edits: working tree vs the index.
    #[default]
    Worktree,
    /// Staged edits: index vs HEAD.
    Index,
    /// Everything since the last commit: working tree vs HEAD.
    Head,
    /// Everything not yet pushed: working tree vs the branch's upstream
    /// (`@{upstream}`). Fails when the branch has no upstream configured,
    /// which is a real answer rather than an error to swallow.
    Upstream,
}

impl Against {
    pub fn as_str(self) -> &'static str {
        match self {
            Against::Worktree => "worktree",
            Against::Index => "index",
            Against::Head => "head",
            Against::Upstream => "upstream",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GitHunksInput {
    /// Repository-relative path to inspect.
    pub path: String,
    /// Directory to run git in. Defaults to the workspace root.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Which comparison to make. Defaults to `worktree`.
    #[serde(default)]
    pub against: Against,
    /// Unchanged lines kept around each hunk in `patch`. Defaults to 0, which
    /// keeps `hunks` exactly the lines that changed — the ranges a gutter
    /// paints. Pass 3 or so for a patch a person will read, and note that the
    /// reported ranges widen to include the context you asked for.
    #[serde(default)]
    pub context_lines: Option<u32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GitHunksOutput {
    pub path: String,
    /// Echo of the comparison performed, so a cached response is self-describing.
    pub against: Against,
    /// Changed ranges, in file order. Empty when the file matches.
    pub hunks: Vec<Hunk>,
    pub added: u32,
    pub removed: u32,
    /// True when git reports the path as untracked, in which case there is
    /// nothing to compare against and `hunks` is empty.
    pub untracked: bool,
    /// The rendered unified patch, for showing a person what changed. Empty
    /// when there is no difference, and capped by `max_diff_bytes` — a caller
    /// that only wants the ranges can ignore it.
    pub patch: String,
}

// --------------------------------------------------------- editor::git::show

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GitShowInput {
    /// Root-relative path.
    pub path: String,
    /// Revision to read the file at. Defaults to `HEAD`.
    #[serde(default)]
    pub rev: Option<String>,
    /// Repository to run in. Defaults to the workspace root.
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GitShowOutput {
    pub path: String,
    pub rev: String,
    /// The file's contents at that revision. Empty when the path did not exist
    /// there — which is what `exists: false` distinguishes from an empty file.
    pub content: String,
    /// False when the path is absent at that revision (a file being added).
    pub exists: bool,
}

#[cfg(test)]
mod request_tests {
    //! What a caller actually gets when it leaves a field out, and what
    //! spellings the wire accepts. The golden schemas pin the *shape* of these
    //! requests; these pin what deserializing one does — a default that flipped
    //! would change behaviour without moving a single line of the schema's
    //! `type`/`required` structure.

    use super::*;
    use serde_json::json;

    /// Defaults are behaviour, not decoration: `stage_all` decides whether a
    /// commit sweeps up unstaged work, and `include_untracked` decides whether
    /// the file an agent just wrote is findable at all.
    #[test]
    fn omitted_fields_take_their_documented_defaults() {
        let commit: GitCommitInput =
            serde_json::from_value(json!({ "message": "m" })).expect("a message is enough");
        assert!(
            commit.stage_all,
            "a commit stages everything unless told not to"
        );
        assert!(commit.cwd.is_none());

        let find: FindInput =
            serde_json::from_value(json!({ "query": "q" })).expect("a query is enough");
        assert!(
            find.include_untracked,
            "a just-created file has to be findable"
        );
        assert!(find.limit.is_none());

        let create: CreateInput =
            serde_json::from_value(json!({ "path": "a.rs" })).expect("a path is enough");
        assert!(matches!(create.kind, EntryKind::File), "a file by default");
        assert!(create.content.is_none());

        let hunks: GitHunksInput =
            serde_json::from_value(json!({ "path": "a.rs" })).expect("a path is enough");
        assert!(matches!(hunks.against, Against::Worktree));
        assert_eq!(
            hunks.context_lines, None,
            "the handler reads absent as -U0; any context here widens every gutter range"
        );

        let tree: TreeInput = serde_json::from_value(json!({})).expect("tree takes no arguments");
        assert!(tree.path.is_none() && tree.max_depth.is_none());
        assert!(tree.expand.is_empty() && tree.collapse.is_empty());

        let search: SearchInput =
            serde_json::from_value(json!({ "pattern": "x" })).expect("a pattern is enough");
        assert!(!search.ignore_case);
        assert!(search.include_glob.is_empty());
        assert_eq!(search.max_matches, None);

        let delete: DeleteInput =
            serde_json::from_value(json!({ "path": "a" })).expect("a path is enough");
        assert!(!delete.recursive, "a recursive delete has to be asked for");

        let _: EmptyInput = serde_json::from_value(json!({})).expect("{} is the whole request");
    }

    /// Neither guard, one guard, or both: the four shapes `editor::save`
    /// branches on all have to survive the wire, including the legacy caller
    /// that has never heard of a version.
    #[test]
    fn a_save_carries_whichever_guard_the_caller_holds() {
        let bare: SaveInput = serde_json::from_value(json!({ "path": "a.rs", "content": "x" }))
            .expect("an unguarded overwrite is a legal request");
        assert!(bare.expected_mtime.is_none() && bare.expected_version.is_none());

        let legacy: SaveInput =
            serde_json::from_value(json!({ "path": "a.rs", "content": "x", "expected_mtime": 7 }))
                .expect("the pre-version request shape still parses");
        assert_eq!(legacy.expected_mtime, Some(7));
        assert!(legacy.expected_version.is_none());

        let versioned: SaveInput = serde_json::from_value(
            json!({ "path": "a.rs", "content": "x", "expected_version": "b-000000000000000a" }),
        )
        .expect("a version-only request parses");
        assert_eq!(
            versioned.expected_version.as_deref(),
            Some("b-000000000000000a")
        );
        assert!(versioned.expected_mtime.is_none());

        let both: SaveInput = serde_json::from_value(json!({
            "path": "a.rs", "content": "x", "expected_mtime": 7, "expected_version": "v"
        }))
        .expect("sending both is legal; the version decides");
        assert_eq!(both.expected_mtime, Some(7));
        assert_eq!(both.expected_version.as_deref(), Some("v"));
    }

    /// The enum spellings are the wire. An agent sends these strings, so a
    /// rename breaks every stored call without touching a single signature.
    #[test]
    fn enum_variants_keep_their_wire_spellings() {
        for (text, variant) in [
            ("worktree", Against::Worktree),
            ("index", Against::Index),
            ("head", Against::Head),
            ("upstream", Against::Upstream),
        ] {
            assert_eq!(
                serde_json::to_value(variant).expect("serializes"),
                json!(text)
            );
            let parsed: Against = serde_json::from_value(json!(text)).expect("a known comparison");
            assert_eq!(
                serde_json::to_value(parsed).expect("serializes"),
                json!(text)
            );
            assert_eq!(
                variant.as_str(),
                text,
                "as_str drifted from the serialized name"
            );
        }
        for (text, variant) in [
            ("fetch", SyncAction::Fetch),
            ("pull", SyncAction::Pull),
            ("push", SyncAction::Push),
        ] {
            assert_eq!(
                serde_json::to_value(variant).expect("serializes"),
                json!(text)
            );
            let parsed: SyncAction = serde_json::from_value(json!(text)).expect("a known action");
            assert_eq!(
                serde_json::to_value(parsed).expect("serializes"),
                json!(text)
            );
        }
        for (text, variant) in [("push", StashAction::Push), ("pop", StashAction::Pop)] {
            assert_eq!(
                serde_json::to_value(variant).expect("serializes"),
                json!(text)
            );
            let parsed: StashAction = serde_json::from_value(json!(text)).expect("a known action");
            assert_eq!(
                serde_json::to_value(parsed).expect("serializes"),
                json!(text)
            );
        }
        for (text, variant) in [("file", EntryKind::File), ("folder", EntryKind::Folder)] {
            assert_eq!(
                serde_json::to_value(variant).expect("serializes"),
                json!(text)
            );
            let parsed: EntryKind = serde_json::from_value(json!(text)).expect("a known kind");
            assert_eq!(
                serde_json::to_value(parsed).expect("serializes"),
                json!(text)
            );
        }
    }

    /// A spelling this worker does not know must be refused, not defaulted:
    /// `editor::git::sync` quietly fetching because it could not read `pusj`
    /// would be a silent no-op where the caller asked to publish work.
    #[test]
    fn an_unknown_enum_spelling_is_refused() {
        assert!(serde_json::from_value::<SyncAction>(json!("pusj")).is_err());
        assert!(serde_json::from_value::<Against>(json!("HEAD")).is_err());
        assert!(serde_json::from_value::<StashAction>(json!("drop")).is_err());
        assert!(serde_json::from_value::<EntryKind>(json!("directory")).is_err());
    }

    /// A required field is required. A save with no content must be refused at
    /// the door rather than written as an empty file.
    #[test]
    fn a_request_missing_a_required_field_is_refused() {
        assert!(serde_json::from_value::<SaveInput>(json!({ "path": "a.rs" })).is_err());
        assert!(serde_json::from_value::<MoveInput>(json!({ "from": "a" })).is_err());
        assert!(serde_json::from_value::<WorkspaceOpenInput>(json!({})).is_err());
        assert!(serde_json::from_value::<SearchInput>(json!({})).is_err());
        assert!(serde_json::from_value::<GitShowInput>(json!({ "rev": "HEAD" })).is_err());
    }
}

// ---------------------------------------------------------------- editor::find

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindInput {
    /// Fuzzy query. Matched as a subsequence against every tracked path, with
    /// basename and word-boundary hits ranked highest. Empty returns the first
    /// `limit` paths unranked.
    pub query: String,
    /// Rows to return. Defaults to the worker's `find_limit`.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Include files git does not track yet (still honouring `.gitignore`).
    /// On by default — a file the agent just created is exactly the one you
    /// are looking for. Ignored outside a repository, where the workspace
    /// listing is the source of candidates.
    #[serde(default = "default_true")]
    pub include_untracked: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FindMatch {
    pub path: String,
    /// Higher is better. Comparable only within one response.
    pub score: i32,
    /// Byte offsets into `path` that matched, in order — enough to highlight
    /// the match without re-running the matcher in the UI.
    pub positions: Vec<u32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FindOutput {
    pub matches: Vec<FindMatch>,
    /// Paths considered. Compare against `truncated` to know whether the whole
    /// repo was ranked.
    pub scanned: u32,
    /// True when not every path in the workspace was ranked: it held more than
    /// `max_find_candidates`, or the folder walk stopped at its visit budget.
    /// Either way this ranking is over a prefix of the workspace.
    pub truncated: bool,
    /// True when candidates came from git's listing (so `.gitignore` was
    /// honoured), false when they came from the folder walk.
    pub from_git: bool,
}

// ---------------------------------------------------------------- editor::open

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenInput {
    /// Jail-relative when `shell`'s `fs.host_roots` are set, else absolute —
    /// the same path vocabulary as `shell::fs::read`.
    pub path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct OpenOutput {
    pub path: String,
    /// File contents as text. Binary files are refused rather than mangled.
    pub content: String,
    /// Monaco language id for the path, e.g. `rust`, `typescript`, `plaintext`.
    pub language: String,
    pub size: u64,
    /// Last-modified time, Unix seconds. Pass it back as `expected_mtime` on
    /// `editor::save` to get the conflict guard.
    pub mtime: i64,
    /// Opaque version of the `content` above. Pass it back as
    /// `expected_version` on `editor::save` for a guard that does not depend on
    /// the filesystem's timestamp resolution: it catches a write that landed
    /// inside the same second, which `mtime` cannot. Compare it for equality
    /// only — the encoding is this worker's business and may change.
    pub version: String,
    /// True when the file exceeded `max_file_bytes` and `content` holds only
    /// its beginning. Saving a truncated buffer back would delete the rest of
    /// the file, so `editor::save` refuses one.
    pub truncated: bool,
}

// ---------------------------------------------------------------- editor::save

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveInput {
    pub path: String,
    /// Full new contents. This is a whole-file write, not a patch.
    pub content: String,
    /// The `mtime` from the `editor::open` this edit started from. When it no
    /// longer matches the file on disk, the write is refused and the
    /// divergence comes back as a patch. Omit only when deliberately
    /// overwriting whatever is there.
    ///
    /// Resolution is one second, which is all the filesystem reports. Two
    /// writes inside the same second are therefore indistinguishable to this
    /// field, and the second one wins silently — send `expected_version`
    /// instead to close that window.
    #[serde(default)]
    pub expected_mtime: Option<i64>,
    /// The `version` from the `editor::open` — or from the previous
    /// `editor::save` — this edit started from. It is a version of the
    /// *content*, so it catches a write that landed inside the same filesystem
    /// second and it does not care whether a clock or a checkout moved the
    /// mtime.
    ///
    /// When this is present it is the guard and `expected_mtime` is ignored.
    /// When it is absent the `expected_mtime` comparison applies exactly as
    /// before, so a caller that has never heard of a version is unaffected.
    #[serde(default)]
    pub expected_version: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SaveOutput {
    pub path: String,
    /// True when the file was written.
    pub saved: bool,
    /// True when the write was refused because the file changed underneath.
    pub conflict: bool,
    /// Last-modified time after the write. Feed it into the next save.
    pub mtime: i64,
    /// Version of the content this file now holds: what was written on success,
    /// what is on disk on a conflict. Feed it into the next save as
    /// `expected_version`.
    pub version: String,
    /// What was on disk when a conflict was detected.
    pub disk_mtime: Option<i64>,
    /// Version of the disk contents when a conflict was detected — the value
    /// to send as `expected_version` once you have reconciled with them.
    pub disk_version: Option<String>,
    /// On conflict: a unified diff from the current disk contents to the
    /// contents you tried to write, so the divergence is reviewable without a
    /// second round trip.
    pub conflict_patch: Option<String>,
    /// Lines this save added relative to what was on disk before it.
    pub added: u32,
    /// Lines this save removed.
    pub removed: u32,
    /// True when the file did not exist and was created.
    pub created: bool,
}

/// One recorded change, as `editor::changes` returns it. Mirrors the
/// `editor::changed` event: the log is those events, kept.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChangeRecord {
    /// Path relative to the workspace root the change was recorded against.
    pub path: String,
    /// Function that performed the write, e.g. `shell::fs::write`.
    pub cause: String,
    /// `created`, `modified`, `deleted`, or `moved`.
    pub kind: String,
    pub added: u32,
    pub removed: u32,
    /// Unified patch for the change, empty when there was nothing to compare.
    #[serde(default)]
    pub patch: String,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub root: String,
    /// The harness session and turn the write happened in, absent when it
    /// happened outside a turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

/// Response of `editor::changes`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChangesView {
    /// Newest first, one entry per path.
    pub changes: Vec<ChangeRecord>,
}
