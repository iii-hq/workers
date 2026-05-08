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
    pub channel_id: String,
    pub access_key: String,
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

#[derive(Debug, Deserialize)]
pub struct WriteArgs {
    pub path: String,
    #[serde(default = "default_write_mode")]
    pub mode: String,
    #[serde(default)]
    pub parents: bool,
    pub content: iii_sdk::channels::StreamChannelRef,
}

#[derive(Debug, Deserialize)]
pub struct ReadArgs {
    pub path: String,
}

// Registration-boundary requests — each carries `target` plus the matching
// `*Args` fields and derives `JsonSchema` for the SDK.

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LsRequest {
    #[serde(default)]
    pub target: Target,
    pub path: String,
}
impl LsRequest {
    pub fn split(self) -> (Target, LsArgs) {
        (self.target, LsArgs { path: self.path })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StatRequest {
    #[serde(default)]
    pub target: Target,
    pub path: String,
}
impl StatRequest {
    pub fn split(self) -> (Target, StatArgs) {
        (self.target, StatArgs { path: self.path })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MkdirRequest {
    #[serde(default)]
    pub target: Target,
    pub path: String,
    #[serde(default = "default_mkdir_mode")]
    pub mode: String,
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
    #[serde(default)]
    pub target: Target,
    pub path: String,
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
    #[serde(default)]
    pub target: Target,
    pub path: String,
    pub mode: String,
    #[serde(default)]
    pub uid: Option<u32>,
    #[serde(default)]
    pub gid: Option<u32>,
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
    #[serde(default)]
    pub target: Target,
    pub src: String,
    pub dst: String,
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
    #[serde(default)]
    pub target: Target,
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
    #[serde(default)]
    pub target: Target,
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteRequest {
    #[serde(default)]
    pub target: Target,
    pub path: String,
    #[serde(default = "default_write_mode")]
    pub mode: String,
    #[serde(default)]
    pub parents: bool,
    pub content: ContentRef,
}
impl WriteRequest {
    pub fn split(self) -> (Target, WriteArgs) {
        (
            self.target,
            WriteArgs {
                path: self.path,
                mode: self.mode,
                parents: self.parents,
                content: self.content.into(),
            },
        )
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadRequest {
    #[serde(default)]
    pub target: Target,
    pub path: String,
}
impl ReadRequest {
    pub fn split(self) -> (Target, ReadArgs) {
        (self.target, ReadArgs { path: self.path })
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LsResponse {
    pub entries: Vec<FsEntry>,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct StatResponse(pub FsEntry);
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MkdirResponse {
    pub created: bool,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RmResponse {
    pub removed: bool,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ChmodResponse {
    pub updated: u64,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MvResponse {
    pub moved: bool,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GrepResponse {
    pub matches: Vec<FsMatch>,
    pub truncated: bool,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SedResponse {
    pub results: Vec<FsSedFileResult>,
    pub total_replacements: u64,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WriteResponse {
    pub bytes_written: u64,
    pub path: String,
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
    pub content: ContentRef,
    pub size: u64,
    pub mode: String,
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
    fn write_args_mode_defaults_0644() {
        let r: WriteArgs = serde_json::from_value(serde_json::json!({
            "path":"/x",
            "content": {
                "channel_id": "c-1",
                "access_key": "k-1",
                "direction": "read"
            }
        }))
        .unwrap();
        assert_eq!(r.mode, "0644");
        assert_eq!(r.content.channel_id, "c-1");
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
    fn write_request_splits_contentref_to_streamchannelref() {
        let req: WriteRequest = serde_json::from_value(serde_json::json!({
            "path": "/x",
            "content": {"channel_id": "c", "access_key": "k", "direction": "read"},
        }))
        .unwrap();
        let (_t, args) = req.split();
        assert_eq!(args.content.channel_id, "c");
    }
}
