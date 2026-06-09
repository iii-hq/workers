pub mod error;
pub mod host;
pub mod sandbox;
pub mod wire;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::fs::error::FsError;
use crate::fs::wire::{FsEntry, FsMatch, FsSedFileResult};

// `Target` lives in `crate::target`; re-exported here so existing
// `use crate::fs::Target` call sites inside the crate keep compiling
// without mechanical churn while the type's home is neutral between
// `fs` and `exec` consumers.
pub use crate::target::Target;

/// Wire-identical mirror of `iii_sdk::channels::StreamChannelRef`. The SDK
/// type lacks `JsonSchema` in 0.11.3, which would block typed registration
/// of `shell::fs::write`/`read`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct ContentRef {
    /// Opaque identifier for the open stream channel.
    pub channel_id: String,
    /// Secret key that authorises access to this channel.
    pub access_key: String,
    /// Direction of data flow: "read" (consume) or "write" (produce).
    #[serde(default)]
    pub direction: ContentDirection,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum ContentDirection {
    #[default]
    Read,
    Write,
}

impl From<ContentRef> for iii_sdk::channels::StreamChannelRef {
    fn from(c: ContentRef) -> Self {
        iii_sdk::channels::StreamChannelRef {
            channel_id: c.channel_id,
            access_key: c.access_key,
            direction: match c.direction {
                ContentDirection::Read => iii_sdk::channels::ChannelDirection::Read,
                ContentDirection::Write => iii_sdk::channels::ChannelDirection::Write,
            },
        }
    }
}

impl From<iii_sdk::channels::StreamChannelRef> for ContentRef {
    fn from(c: iii_sdk::channels::StreamChannelRef) -> Self {
        ContentRef {
            channel_id: c.channel_id,
            access_key: c.access_key,
            direction: match c.direction {
                iii_sdk::channels::ChannelDirection::Read => ContentDirection::Read,
                iii_sdk::channels::ChannelDirection::Write => ContentDirection::Write,
            },
        }
    }
}

// Backend args — `FsBackend` trait inputs, no `target`, no `JsonSchema`.

#[derive(Debug, Deserialize)]
pub struct LsArgs {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct StatArgs {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct MkdirArgs {
    pub path: String,
    #[serde(default = "default_mkdir_mode")]
    pub mode: String,
    #[serde(default)]
    pub parents: bool,
}

#[derive(Debug, Deserialize)]
pub struct RmArgs {
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Deserialize)]
pub struct ChmodArgs {
    pub path: String,
    pub mode: String,
    #[serde(default)]
    pub uid: Option<u32>,
    #[serde(default)]
    pub gid: Option<u32>,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Deserialize)]
pub struct MvArgs {
    pub src: String,
    pub dst: String,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Deserialize)]
pub struct GrepArgs {
    pub path: String,
    pub pattern: String,
    #[serde(default = "default_true")]
    pub recursive: bool,
    #[serde(default)]
    pub ignore_case: bool,
    #[serde(default)]
    pub include_glob: Vec<String>,
    #[serde(default)]
    pub exclude_glob: Vec<String>,
    #[serde(default = "default_max_matches")]
    pub max_matches: u64,
    #[serde(default = "default_max_line_bytes")]
    pub max_line_bytes: u64,
}
fn default_max_matches() -> u64 {
    10_000
}
fn default_max_line_bytes() -> u64 {
    4096
}
fn default_mkdir_mode() -> String {
    "0755".to_string()
}
fn default_write_mode() -> String {
    "0644".to_string()
}

#[derive(Debug, Deserialize)]
pub struct SedArgs {
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default = "default_true")]
    pub recursive: bool,
    #[serde(default)]
    pub include_glob: Vec<String>,
    #[serde(default)]
    pub exclude_glob: Vec<String>,
    pub pattern: String,
    pub replacement: String,
    #[serde(default = "default_true")]
    pub regex: bool,
    #[serde(default)]
    pub first_only: bool,
    #[serde(default)]
    pub ignore_case: bool,
}
fn default_true() -> bool {
    true
}

/// Backend-side bytes to write: either inline UTF-8 text (the common
/// agent path, HOST only) or a streaming channel ref (host or sandbox, for
/// large/streamed payloads).
#[derive(Debug)]
pub enum WriteContent {
    Inline(String),
    Stream(iii_sdk::channels::StreamChannelRef),
}

#[derive(Debug)]
pub struct WriteArgs {
    pub path: String,
    pub mode: String,
    pub parents: bool,
    pub content: WriteContent,
}

#[derive(Debug, Deserialize)]
pub struct ReadArgs {
    pub path: String,
}

// Registration-boundary requests — each carries `target` plus the matching
// `*Args` fields and derives `JsonSchema` for the SDK.

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LsRequest {
    /// host (default) or { kind: "sandbox", sandbox_id }.
    #[serde(default)]
    pub target: Target,
    /// Jail-relative when fs.host_root is set, else absolute.
    pub path: String,
}
impl LsRequest {
    pub fn split(self) -> (Target, LsArgs) {
        (self.target, LsArgs { path: self.path })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StatRequest {
    /// host (default) or { kind: "sandbox", sandbox_id }.
    #[serde(default)]
    pub target: Target,
    /// Jail-relative when fs.host_root is set, else absolute.
    pub path: String,
}
impl StatRequest {
    pub fn split(self) -> (Target, StatArgs) {
        (self.target, StatArgs { path: self.path })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MkdirRequest {
    /// host (default) or { kind: "sandbox", sandbox_id }.
    #[serde(default)]
    pub target: Target,
    /// Jail-relative when fs.host_root is set, else absolute.
    pub path: String,
    /// Octal permission string, e.g. "0755".
    #[serde(default = "default_mkdir_mode")]
    pub mode: String,
    /// Create missing parent directories.
    #[serde(default)]
    pub parents: bool,
}
impl MkdirRequest {
    pub fn split(self) -> (Target, MkdirArgs) {
        (
            self.target,
            MkdirArgs {
                path: self.path,
                mode: self.mode,
                parents: self.parents,
            },
        )
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RmRequest {
    /// host (default) or { kind: "sandbox", sandbox_id }.
    #[serde(default)]
    pub target: Target,
    /// Jail-relative when fs.host_root is set, else absolute.
    pub path: String,
    /// Required to delete a non-empty directory.
    #[serde(default)]
    pub recursive: bool,
}
impl RmRequest {
    pub fn split(self) -> (Target, RmArgs) {
        (
            self.target,
            RmArgs {
                path: self.path,
                recursive: self.recursive,
            },
        )
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ChmodRequest {
    /// host (default) or { kind: "sandbox", sandbox_id }.
    #[serde(default)]
    pub target: Target,
    /// Jail-relative when fs.host_root is set, else absolute.
    pub path: String,
    /// Octal permission string, e.g. "0755".
    pub mode: String,
    /// Optional chown to this numeric uid.
    #[serde(default)]
    pub uid: Option<u32>,
    /// Optional chown to this numeric gid.
    #[serde(default)]
    pub gid: Option<u32>,
    /// Apply mode/owner change to all files under the path recursively.
    #[serde(default)]
    pub recursive: bool,
}
impl ChmodRequest {
    pub fn split(self) -> (Target, ChmodArgs) {
        (
            self.target,
            ChmodArgs {
                path: self.path,
                mode: self.mode,
                uid: self.uid,
                gid: self.gid,
                recursive: self.recursive,
            },
        )
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MvRequest {
    /// host (default) or { kind: "sandbox", sandbox_id }.
    #[serde(default)]
    pub target: Target,
    /// Source path; jail-relative when fs.host_root is set, else absolute.
    pub src: String,
    /// Destination path; jail-relative when fs.host_root is set, else absolute.
    pub dst: String,
    /// Replace an existing destination instead of returning an error.
    #[serde(default)]
    pub overwrite: bool,
}
impl MvRequest {
    pub fn split(self) -> (Target, MvArgs) {
        (
            self.target,
            MvArgs {
                src: self.src,
                dst: self.dst,
                overwrite: self.overwrite,
            },
        )
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GrepRequest {
    /// host (default) or { kind: "sandbox", sandbox_id }.
    #[serde(default)]
    pub target: Target,
    /// Jail-relative when fs.host_root is set, else absolute.
    pub path: String,
    /// Rust regex (RE2-like) matched against each line.
    pub pattern: String,
    /// Descend into subdirectories (default true).
    #[serde(default = "default_true")]
    pub recursive: bool,
    /// Match pattern case-insensitively.
    #[serde(default)]
    pub ignore_case: bool,
    /// Glob filters restricting which file paths are searched.
    #[serde(default)]
    pub include_glob: Vec<String>,
    /// Glob filters excluding file paths from the search.
    #[serde(default)]
    pub exclude_glob: Vec<String>,
    /// Stop collecting matches after this many results (default 10 000).
    #[serde(default = "default_max_matches")]
    pub max_matches: u64,
    /// Skip lines longer than this many bytes (default 4 096).
    #[serde(default = "default_max_line_bytes")]
    pub max_line_bytes: u64,
}
impl GrepRequest {
    pub fn split(self) -> (Target, GrepArgs) {
        (
            self.target,
            GrepArgs {
                path: self.path,
                pattern: self.pattern,
                recursive: self.recursive,
                ignore_case: self.ignore_case,
                include_glob: self.include_glob,
                exclude_glob: self.exclude_glob,
                max_matches: self.max_matches,
                max_line_bytes: self.max_line_bytes,
            },
        )
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SedRequest {
    /// host (default) or { kind: "sandbox", sandbox_id }.
    #[serde(default)]
    pub target: Target,
    /// Explicit list of file paths to edit; provide either this or `path`, not both.
    #[serde(default)]
    pub files: Vec<String>,
    /// Root path to walk for files; used with `recursive`, `include_glob`, `exclude_glob`.
    #[serde(default)]
    pub path: Option<String>,
    /// Descend into subdirectories when `path` is set (default true).
    #[serde(default = "default_true")]
    pub recursive: bool,
    /// Glob filters restricting which file paths are edited.
    #[serde(default)]
    pub include_glob: Vec<String>,
    /// Glob filters excluding file paths from editing.
    #[serde(default)]
    pub exclude_glob: Vec<String>,
    /// Rust regex by default; set regex:false for a literal string.
    pub pattern: String,
    /// String to substitute for each match.
    pub replacement: String,
    /// Treat pattern as a regex (default true) or a literal string (false).
    #[serde(default = "default_true")]
    pub regex: bool,
    /// Replace only the first match per file instead of all matches.
    #[serde(default)]
    pub first_only: bool,
    /// Match pattern case-insensitively.
    #[serde(default)]
    pub ignore_case: bool,
}
impl SedRequest {
    pub fn split(self) -> (Target, SedArgs) {
        (
            self.target,
            SedArgs {
                files: self.files,
                path: self.path,
                recursive: self.recursive,
                include_glob: self.include_glob,
                exclude_glob: self.exclude_glob,
                pattern: self.pattern,
                replacement: self.replacement,
                regex: self.regex,
                first_only: self.first_only,
                ignore_case: self.ignore_case,
            },
        )
    }
}

/// Wire form of write content. A plain JSON **string** is written inline (host
/// target only); a JSON **object** is a streaming `ContentRef` (host or sandbox,
/// for large/streamed payloads). Untagged, so callers just pass `"content":
/// "text"` or `"content": { channel_id, access_key, direction }`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum WriteContentWire {
    /// Inline UTF-8 text, written verbatim. HOST target only.
    Inline(String),
    /// Open write-stream channel ref. Required for sandbox targets and large
    /// or streamed payloads.
    Stream(ContentRef),
}
impl From<WriteContentWire> for WriteContent {
    fn from(w: WriteContentWire) -> Self {
        match w {
            WriteContentWire::Inline(s) => WriteContent::Inline(s),
            WriteContentWire::Stream(r) => WriteContent::Stream(r.into()),
        }
    }
}

/// One file in a batch `shell::fs::write` (`files: [...]`).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteFileSpec {
    /// Jail-relative when fs.host_root is set, else absolute.
    pub path: String,
    /// Inline string (recommended) or a streaming ContentRef.
    pub content: WriteContentWire,
    /// Octal permission string, e.g. "0644".
    #[serde(default = "default_write_mode")]
    pub mode: String,
    /// Create missing parent directories.
    #[serde(default)]
    pub parents: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteRequest {
    /// host (default) or { kind: "sandbox", sandbox_id }.
    #[serde(default)]
    pub target: Target,
    /// Single-file form: the path to write. Jail-relative when fs.host_root is
    /// set, else absolute. Omit when using `files`.
    #[serde(default)]
    pub path: Option<String>,
    /// Single-file content: a plain string written inline (host target only),
    /// OR a ContentRef { channel_id, access_key, direction } for an open write
    /// stream channel. Omit when using `files`.
    #[serde(default)]
    pub content: Option<WriteContentWire>,
    /// Octal permission string for the single-file form, e.g. "0644".
    #[serde(default = "default_write_mode")]
    pub mode: String,
    /// Create missing parent directories (single-file form).
    #[serde(default)]
    pub parents: bool,
    /// Batch form: write several files in one call. When present, the
    /// single-file fields (`path`/`content`/`mode`/`parents`) must be omitted.
    #[serde(default)]
    pub files: Option<Vec<WriteFileSpec>>,
}
impl WriteRequest {
    /// Normalize into `(target, one-or-more backend WriteArgs)`. Rejects (S210)
    /// an ambiguous request (both single and `files`) or an empty one.
    pub fn into_specs(self) -> Result<(Target, Vec<WriteArgs>), FsError> {
        if let Some(files) = self.files {
            if self.path.is_some() || self.content.is_some() {
                return Err(FsError::new(
                    "S210",
                    "provide either the single-file `path`+`content` or a `files` array, not both",
                ));
            }
            if files.is_empty() {
                return Err(FsError::new(
                    "S210",
                    "`files` is empty; provide at least one { path, content } entry",
                ));
            }
            let specs = files
                .into_iter()
                .map(|f| WriteArgs {
                    path: f.path,
                    mode: f.mode,
                    parents: f.parents,
                    content: f.content.into(),
                })
                .collect();
            Ok((self.target, specs))
        } else {
            let path = self
                .path
                .ok_or_else(|| FsError::new("S210", "missing `path` (or pass a `files` array)"))?;
            let content = self.content.ok_or_else(|| {
                FsError::new(
                    "S210",
                    "missing `content` (a string to write inline, or a ContentRef)",
                )
            })?;
            Ok((
                self.target,
                vec![WriteArgs {
                    path,
                    mode: self.mode,
                    parents: self.parents,
                    content: content.into(),
                }],
            ))
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadRequest {
    /// host (default) or { kind: "sandbox", sandbox_id }.
    #[serde(default)]
    pub target: Target,
    /// Jail-relative when fs.host_root is set, else absolute.
    pub path: String,
}
impl ReadRequest {
    pub fn split(self) -> (Target, ReadArgs) {
        (self.target, ReadArgs { path: self.path })
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LsResponse {
    /// Metadata for each entry in the directory.
    pub entries: Vec<FsEntry>,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct StatResponse(pub FsEntry);
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MkdirResponse {
    /// True when a new directory was created; false when it already existed
    /// (only possible with `parents: true`, which is idempotent).
    pub created: bool,
    /// The directory path that was targeted. Empty for sandbox targets.
    #[serde(default)]
    pub path: String,
    /// True when the path already existed and `parents` was set. Host only;
    /// sandbox targets default this to false (not a signal there).
    #[serde(default)]
    pub already_existed: bool,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RmResponse {
    /// True when the path was removed.
    pub removed: bool,
    /// The path that was targeted. Empty for sandbox targets.
    #[serde(default)]
    pub path: String,
    /// True when the path existed before removal. Host only; sandbox targets
    /// default this to false, which does NOT mean the path was absent.
    #[serde(default)]
    pub was_present: bool,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ChmodResponse {
    /// Number of filesystem entries whose mode/owner changed.
    #[serde(alias = "updated")]
    pub entries_changed: u64,
    /// The path that was targeted. Empty for sandbox targets.
    #[serde(default)]
    pub path: String,
    /// Whether the change was applied recursively. Host only.
    #[serde(default)]
    pub recursive: bool,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MvResponse {
    /// True when the move/rename succeeded.
    pub moved: bool,
    /// Source path. Empty for sandbox targets.
    #[serde(default)]
    pub src: String,
    /// Destination path. Empty for sandbox targets.
    #[serde(default)]
    pub dst: String,
    /// True when an existing destination was overwritten. Host only (sandbox
    /// targets default this to false, not a signal there).
    /// Best-effort: derived from a pre-rename existence check, so under a
    /// concurrent writer racing the destination it may under-report (and an
    /// `overwrite:false` move can still replace a file created in that window).
    #[serde(default)]
    pub overwrote: bool,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GrepResponse {
    /// All collected match locations up to `max_matches`.
    pub matches: Vec<FsMatch>,
    /// True when the result was capped by `max_matches` or `max_line_bytes`.
    pub truncated: bool,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SedResponse {
    /// Per-file replacement details.
    pub results: Vec<FsSedFileResult>,
    /// Sum of replacements made across all files.
    pub total_replacements: u64,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WriteFileResult {
    /// Path of the written file.
    pub path: String,
    /// Bytes written to this file.
    pub bytes_written: u64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WriteResponse {
    /// Bytes written: this file for a single write; the sum across `files` for
    /// a batch write.
    pub bytes_written: u64,
    /// Path written for a single write; empty for a batch (see `files`).
    pub path: String,
    /// Per-file results for a batch (`files: [...]`) write; empty for a
    /// single-file write.
    #[serde(default)]
    pub files: Vec<WriteFileResult>,
}

/// Backend-internal read result, holding the SDK's `StreamChannelRef`
/// directly so the host backend can return what `create_channel` produced
/// without conversion. Wire-facing reads serialize via `ReadResponseWire`.
#[derive(Debug, Serialize, Deserialize)]
pub struct ReadResponse {
    pub content: iii_sdk::channels::StreamChannelRef,
    pub size: u64,
    pub mode: String,
    pub mtime: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ReadResponseWire {
    /// Channel reference for streaming the file content back to the caller.
    pub content: ContentRef,
    /// File size in bytes at the time of the read.
    pub size: u64,
    /// Octal permission string of the file, e.g. "0644".
    pub mode: String,
    /// Last-modified time as a Unix timestamp (seconds).
    pub mtime: i64,
}
impl From<ReadResponse> for ReadResponseWire {
    fn from(r: ReadResponse) -> Self {
        ReadResponseWire {
            content: r.content.into(),
            size: r.size,
            mode: r.mode,
            mtime: r.mtime,
        }
    }
}

pub type FsCallResult<T> = Result<T, FsError>;

#[async_trait]
pub trait FsBackend: Send + Sync {
    async fn ls(&self, req: LsArgs) -> FsCallResult<LsResponse>;
    async fn stat(&self, req: StatArgs) -> FsCallResult<StatResponse>;
    async fn mkdir(&self, req: MkdirArgs) -> FsCallResult<MkdirResponse>;
    async fn rm(&self, req: RmArgs) -> FsCallResult<RmResponse>;
    async fn chmod(&self, req: ChmodArgs) -> FsCallResult<ChmodResponse>;
    async fn mv(&self, req: MvArgs) -> FsCallResult<MvResponse>;
    async fn grep(&self, req: GrepArgs) -> FsCallResult<GrepResponse>;
    async fn sed(&self, req: SedArgs) -> FsCallResult<SedResponse>;
    async fn write(&self, req: WriteArgs) -> FsCallResult<WriteResponse>;
    async fn read(&self, req: ReadArgs) -> FsCallResult<ReadResponse>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn target_defaults_to_host_when_field_missing() {
        let v: Target = serde_json::from_value(serde_json::json!({"kind": "host"})).unwrap();
        assert_eq!(v, Target::Host);
    }

    #[test]
    fn target_parses_sandbox() {
        let id = Uuid::new_v4();
        let v: Target = serde_json::from_value(
            serde_json::json!({"kind": "sandbox", "sandbox_id": id.to_string()}),
        )
        .unwrap();
        assert_eq!(v, Target::Sandbox { sandbox_id: id });
    }

    #[test]
    fn target_rejects_unknown_kind() {
        let r: Result<Target, _> = serde_json::from_value(serde_json::json!({"kind": "wat"}));
        assert!(r.is_err());
    }

    #[test]
    fn grep_args_recursive_defaults_true() {
        let r: GrepArgs =
            serde_json::from_value(serde_json::json!({"path":"/x","pattern":"foo"})).unwrap();
        assert!(r.recursive);
        assert_eq!(r.max_matches, 10_000);
        assert_eq!(r.max_line_bytes, 4096);
    }

    #[test]
    fn sed_args_regex_defaults_true() {
        let r: SedArgs = serde_json::from_value(
            serde_json::json!({"path":"/x","pattern":"a","replacement":"b"}),
        )
        .unwrap();
        assert!(r.regex);
        assert!(r.recursive);
    }

    #[test]
    fn mkdir_args_mode_defaults_0755() {
        let r: MkdirArgs = serde_json::from_value(serde_json::json!({"path":"/x"})).unwrap();
        assert_eq!(r.mode, "0755");
        assert!(!r.parents);
    }

    #[test]
    fn write_request_single_defaults_mode_0644_and_parses_contentref() {
        let req: WriteRequest = serde_json::from_value(serde_json::json!({
            "path":"/x",
            "content": { "channel_id": "c-1", "access_key": "k-1", "direction": "read" }
        }))
        .unwrap();
        let (_t, specs) = req.into_specs().unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].mode, "0644");
        match &specs[0].content {
            WriteContent::Stream(r) => assert_eq!(r.channel_id, "c-1"),
            other => panic!("expected stream content, got {other:?}"),
        }
    }

    #[test]
    fn write_request_inline_string_content() {
        let req: WriteRequest = serde_json::from_value(serde_json::json!({
            "path": "/x", "content": "hello world"
        }))
        .unwrap();
        let (_t, specs) = req.into_specs().unwrap();
        assert_eq!(specs.len(), 1);
        match &specs[0].content {
            WriteContent::Inline(s) => assert_eq!(s, "hello world"),
            other => panic!("expected inline content, got {other:?}"),
        }
    }

    #[test]
    fn write_request_batch_files() {
        let req: WriteRequest = serde_json::from_value(serde_json::json!({
            "files": [
                {"path": "/a", "content": "A"},
                {"path": "/b", "content": "B", "mode": "0600"}
            ]
        }))
        .unwrap();
        let (_t, specs) = req.into_specs().unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].path, "/a");
        assert_eq!(specs[1].mode, "0600");
    }

    #[test]
    fn write_request_rejects_both_single_and_files() {
        let req: WriteRequest = serde_json::from_value(serde_json::json!({
            "path": "/a", "content": "A",
            "files": [{"path":"/b","content":"B"}]
        }))
        .unwrap();
        assert_eq!(req.into_specs().unwrap_err().code, "S210");
    }

    #[test]
    fn write_request_rejects_missing_content() {
        let req: WriteRequest = serde_json::from_value(serde_json::json!({"path":"/a"})).unwrap();
        assert_eq!(req.into_specs().unwrap_err().code, "S210");
    }

    #[test]
    fn ls_request_target_defaults_to_host() {
        let r: LsRequest = serde_json::from_value(serde_json::json!({"path": "/x"})).unwrap();
        assert_eq!(r.target, Target::Host);
    }

    #[test]
    fn ls_request_with_sandbox_target_round_trips() {
        let id = Uuid::new_v4();
        let r: LsRequest = serde_json::from_value(serde_json::json!({
            "target": {"kind": "sandbox", "sandbox_id": id.to_string()},
            "path": "/x",
        }))
        .unwrap();
        assert_eq!(r.target, Target::Sandbox { sandbox_id: id });
        let (t, args) = r.split();
        assert_eq!(t, Target::Sandbox { sandbox_id: id });
        assert_eq!(args.path, "/x");
    }

    #[test]
    fn write_request_contentref_normalizes_to_stream() {
        let req: WriteRequest = serde_json::from_value(serde_json::json!({
            "path": "/x",
            "content": {"channel_id": "c", "access_key": "k", "direction": "read"},
        }))
        .unwrap();
        let (_t, specs) = req.into_specs().unwrap();
        match &specs[0].content {
            WriteContent::Stream(r) => assert_eq!(r.channel_id, "c"),
            other => panic!("expected stream content, got {other:?}"),
        }
    }

    #[test]
    fn mkdir_response_deserializes_legacy_engine_shape() {
        // Sandbox round-trip: engine returns only `created`; new fields default.
        let r: MkdirResponse =
            serde_json::from_value(serde_json::json!({"created": true})).unwrap();
        assert!(r.created);
        assert!(!r.already_existed);
        assert_eq!(r.path, "");
    }

    #[test]
    fn chmod_response_accepts_legacy_updated_key() {
        // Engine returns `updated`; alias maps it onto `entries_changed`.
        let r: ChmodResponse = serde_json::from_value(serde_json::json!({"updated": 5})).unwrap();
        assert_eq!(r.entries_changed, 5);
    }

    #[test]
    fn mv_and_rm_responses_deserialize_legacy_shape() {
        let m: MvResponse = serde_json::from_value(serde_json::json!({"moved": true})).unwrap();
        assert!(m.moved);
        assert!(!m.overwrote);
        let r: RmResponse = serde_json::from_value(serde_json::json!({"removed": true})).unwrap();
        assert!(r.removed);
        assert!(!r.was_present);
    }
}
