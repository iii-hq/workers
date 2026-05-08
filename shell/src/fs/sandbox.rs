//! Sandbox-target backend. Forwards every fs op to the engine daemon's
//! `sandbox::fs::<op>` trigger via `iii.trigger`. Stateless — engine owns
//! all sandbox state, including channel allocation for write/read
//! passthroughs.

use async_trait::async_trait;
use iii_sdk::IIIError;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::fs::error::FsError;
use crate::fs::{
    ChmodArgs, ChmodResponse, FsBackend, FsCallResult, GrepArgs, GrepResponse, LsArgs, LsResponse,
    MkdirArgs, MkdirResponse, MvArgs, MvResponse, ReadArgs, ReadResponse, RmArgs, RmResponse,
    SedArgs, SedResponse, StatArgs, StatResponse, WriteArgs, WriteResponse,
};

// TriggerFwd + IiiTriggerFwd live in `crate::triggers`; re-exported
// here so existing `use crate::fs::sandbox::{TriggerFwd, IiiTriggerFwd}`
// call sites inside the crate, plus the public
// `iii_shell::fs::sandbox::TriggerFwd` surface consumed by
// `tests/sandbox_dispatch.rs`, keep compiling without mechanical churn.
pub use crate::triggers::{IiiTriggerFwd, TriggerFwd};

pub struct SandboxFsBackend {
    fwd: Arc<dyn TriggerFwd>,
    enabled: bool,
    sandbox_id: Uuid,
}

impl SandboxFsBackend {
    pub fn new(fwd: Arc<dyn TriggerFwd>, enabled: bool, sandbox_id: Uuid) -> Self {
        Self {
            fwd,
            enabled,
            sandbox_id,
        }
    }

    fn check_enabled(&self) -> FsCallResult<()> {
        if !self.enabled {
            return Err(FsError::new("S210", "sandbox target disabled in config"));
        }
        Ok(())
    }

    async fn dispatch<R: serde::de::DeserializeOwned>(
        &self,
        function_id: &str,
        mut payload: Value,
    ) -> FsCallResult<R> {
        self.check_enabled()?;
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("sandbox_id".into(), json!(self.sandbox_id.to_string()));
        }
        let resp = self
            .fwd
            .trigger(function_id, payload)
            .await
            .map_err(map_iii_err)?;
        serde_json::from_value(resp)
            .map_err(|e| FsError::new("S216", format!("bad engine response: {e}")))
    }
}

/// Recover an S2xx code from an `IIIError`. Engine `Remote` errors carry
/// the code structurally; the engine's `invocation_failed` wrapper hides
/// it inside the message string; mock paths emit a JSON payload as
/// `Handler(string)`. Unknown shapes fall through to S216.
fn map_iii_err(err: IIIError) -> FsError {
    match &err {
        IIIError::Remote { code, message, .. } if code.starts_with('S') => {
            return FsError::new(
                map_static_code(code),
                format!("forwarded from engine: {message}"),
            );
        }
        IIIError::Remote { message, .. } => {
            if let Some(c) = scan_s_code(message) {
                return FsError::new(
                    map_static_code(c),
                    format!("forwarded from engine (wrapped): {message}"),
                );
            }
        }
        IIIError::Handler(s) => {
            if let Ok(parsed) = serde_json::from_str::<Value>(s) {
                if let Some(c) = parsed.get("code").and_then(|v| v.as_str()) {
                    let msg = parsed.get("message").and_then(|v| v.as_str()).unwrap_or("");
                    return FsError::new(
                        map_static_code(c),
                        format!("forwarded from engine: {msg}"),
                    );
                }
            }
            if let Some(c) = scan_s_code(s) {
                return FsError::new(
                    map_static_code(c),
                    format!("forwarded from engine (raw): {s}"),
                );
            }
        }
        _ => {}
    }
    FsError::new("S216", format!("engine error: {err:?}"))
}

fn scan_s_code(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    // The window is 4 bytes (`bytes[i..=i+3]`), so the last valid `i` is
    // `len - 4`. The loop bound `< len - 3` gives `i <= len - 4`.
    // `saturating_sub(3)` collapses to 0 when `len < 4`, which yields an
    // empty range (no false access) on too-short inputs.
    for i in 0..bytes.len().saturating_sub(3) {
        if bytes[i] == b'S'
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 2].is_ascii_digit()
            && bytes[i + 3].is_ascii_digit()
        {
            // Reject matches preceded by alphanumerics so we don't grab
            // the tail of a longer identifier (e.g. "FOO_S211").
            let preceded_by_word =
                i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            if !preceded_by_word {
                return Some(&s[i..i + 4]);
            }
        }
    }
    None
}

fn map_static_code(code: &str) -> &'static str {
    match code {
        "S001" => "S001",
        "S002" => "S002",
        "S003" => "S003",
        "S004" => "S004",
        "S200" => "S200",
        "S210" => "S210",
        "S211" => "S211",
        "S212" => "S212",
        "S213" => "S213",
        "S214" => "S214",
        "S215" => "S215",
        "S216" => "S216",
        "S217" => "S217",
        "S218" => "S218",
        "S219" => "S219",
        _ => "S216",
    }
}

#[async_trait]
impl FsBackend for SandboxFsBackend {
    async fn ls(&self, req: LsArgs) -> FsCallResult<LsResponse> {
        self.dispatch("sandbox::fs::ls", json!({ "path": req.path }))
            .await
    }
    async fn stat(&self, req: StatArgs) -> FsCallResult<StatResponse> {
        // StatResponse is #[serde(transparent)] so it deserializes from
        // the engine's flat FsEntry shape directly.
        self.dispatch("sandbox::fs::stat", json!({ "path": req.path }))
            .await
    }
    async fn mkdir(&self, req: MkdirArgs) -> FsCallResult<MkdirResponse> {
        self.dispatch(
            "sandbox::fs::mkdir",
            json!({ "path": req.path, "mode": req.mode, "parents": req.parents }),
        )
        .await
    }
    async fn rm(&self, req: RmArgs) -> FsCallResult<RmResponse> {
        self.dispatch(
            "sandbox::fs::rm",
            json!({ "path": req.path, "recursive": req.recursive }),
        )
        .await
    }
    async fn chmod(&self, req: ChmodArgs) -> FsCallResult<ChmodResponse> {
        self.dispatch(
            "sandbox::fs::chmod",
            json!({
                "path": req.path, "mode": req.mode,
                "uid": req.uid, "gid": req.gid, "recursive": req.recursive
            }),
        )
        .await
    }
    async fn mv(&self, req: MvArgs) -> FsCallResult<MvResponse> {
        self.dispatch(
            "sandbox::fs::mv",
            json!({ "src": req.src, "dst": req.dst, "overwrite": req.overwrite }),
        )
        .await
    }
    async fn grep(&self, req: GrepArgs) -> FsCallResult<GrepResponse> {
        self.dispatch(
            "sandbox::fs::grep",
            json!({
                "path": req.path, "pattern": req.pattern,
                "recursive": req.recursive, "ignore_case": req.ignore_case,
                "include_glob": req.include_glob, "exclude_glob": req.exclude_glob,
                "max_matches": req.max_matches, "max_line_bytes": req.max_line_bytes
            }),
        )
        .await
    }
    async fn sed(&self, req: SedArgs) -> FsCallResult<SedResponse> {
        self.dispatch(
            "sandbox::fs::sed",
            json!({
                "files": req.files, "path": req.path,
                "recursive": req.recursive,
                "include_glob": req.include_glob, "exclude_glob": req.exclude_glob,
                "pattern": req.pattern, "replacement": req.replacement,
                "regex": req.regex, "first_only": req.first_only,
                "ignore_case": req.ignore_case
            }),
        )
        .await
    }
    async fn write(&self, req: WriteArgs) -> FsCallResult<WriteResponse> {
        self.dispatch(
            "sandbox::fs::write",
            json!({
                "path": req.path,
                "mode": req.mode,
                "parents": req.parents,
                "content": req.content,
            }),
        )
        .await
    }

    async fn read(&self, req: ReadArgs) -> FsCallResult<ReadResponse> {
        self.dispatch("sandbox::fs::read", json!({ "path": req.path }))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct StubFwd {
        captured: Mutex<Option<(String, Value)>>,
        respond_with: Result<Value, &'static str>,
    }

    #[async_trait]
    impl TriggerFwd for StubFwd {
        async fn trigger(&self, fid: &str, payload: Value) -> Result<Value, IIIError> {
            *self.captured.lock().unwrap() = Some((fid.into(), payload));
            match &self.respond_with {
                Ok(v) => Ok(v.clone()),
                Err(json) => Err(IIIError::Handler(json.to_string())),
            }
        }
    }

    #[tokio::test]
    async fn ls_forwards_payload_with_sandbox_id() {
        let stub = Arc::new(StubFwd {
            captured: Mutex::new(None),
            respond_with: Ok(json!({ "entries": [] })),
        });
        let id = Uuid::new_v4();
        let b = SandboxFsBackend::new(stub.clone(), true, id);
        let resp = b.ls(LsArgs { path: "/x".into() }).await.unwrap();
        assert_eq!(resp.entries.len(), 0);
        let (fid, payload) = stub.captured.lock().unwrap().clone().unwrap();
        assert_eq!(fid, "sandbox::fs::ls");
        assert_eq!(payload.get("path").unwrap().as_str(), Some("/x"));
        assert_eq!(
            payload.get("sandbox_id").unwrap().as_str(),
            Some(id.to_string().as_str()),
        );
    }

    #[tokio::test]
    async fn disabled_returns_s210() {
        let stub = Arc::new(StubFwd {
            captured: Mutex::new(None),
            respond_with: Ok(json!({ "entries": [] })),
        });
        let b = SandboxFsBackend::new(stub, false, Uuid::new_v4());
        let err = b.ls(LsArgs { path: "/x".into() }).await.unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn engine_s211_forwards_as_s211() {
        let stub = Arc::new(StubFwd {
            captured: Mutex::new(None),
            respond_with: Err(r#"{"code":"S211","message":"not found"}"#),
        });
        let b = SandboxFsBackend::new(stub, true, Uuid::new_v4());
        let err = b.ls(LsArgs { path: "/x".into() }).await.unwrap_err();
        assert_eq!(err.code, "S211");
    }

    #[tokio::test]
    async fn engine_s002_sandbox_not_found_forwards() {
        let stub = Arc::new(StubFwd {
            captured: Mutex::new(None),
            respond_with: Err(r#"{"code":"S002","message":"sandbox not found"}"#),
        });
        let b = SandboxFsBackend::new(stub, true, Uuid::new_v4());
        let err = b.ls(LsArgs { path: "/x".into() }).await.unwrap_err();
        assert_eq!(err.code, "S002");
    }

    #[tokio::test]
    async fn write_forwards_payload_with_content_and_sandbox_id() {
        let stub = Arc::new(StubFwd {
            captured: Mutex::new(None),
            respond_with: Ok(json!({ "bytes_written": 5, "path": "/sb/x" })),
        });
        let id = Uuid::new_v4();
        let b = SandboxFsBackend::new(stub.clone(), true, id);
        let content = iii_sdk::channels::StreamChannelRef {
            channel_id: "c-1".into(),
            access_key: "k-1".into(),
            direction: iii_sdk::channels::ChannelDirection::Read,
        };
        let resp = b
            .write(WriteArgs {
                path: "/sb/x".into(),
                mode: "0644".into(),
                parents: false,
                content: content.clone(),
            })
            .await
            .unwrap();
        assert_eq!(resp.bytes_written, 5);
        let (fid, payload) = stub.captured.lock().unwrap().clone().unwrap();
        assert_eq!(fid, "sandbox::fs::write");
        let obj = payload.as_object().unwrap();
        assert_eq!(obj["sandbox_id"].as_str(), Some(id.to_string().as_str()));
        assert_eq!(obj["path"].as_str(), Some("/sb/x"));
        assert_eq!(obj["mode"].as_str(), Some("0644"));
        assert_eq!(obj["parents"].as_bool(), Some(false));
        assert_eq!(obj["content"]["channel_id"].as_str(), Some("c-1"));
        assert_eq!(obj["content"]["access_key"].as_str(), Some("k-1"));
        assert_eq!(obj["content"]["direction"].as_str(), Some("read"));
    }

    #[tokio::test]
    async fn read_forwards_path_only_and_returns_engine_response() {
        let stub = Arc::new(StubFwd {
            captured: Mutex::new(None),
            respond_with: Ok(json!({
                "content": { "channel_id": "ec", "access_key": "ek", "direction": "read" },
                "size": 42,
                "mode": "0644",
                "mtime": 1700000000
            })),
        });
        let id = Uuid::new_v4();
        let b = SandboxFsBackend::new(stub.clone(), true, id);
        let resp = b
            .read(ReadArgs {
                path: "/sb/y".into(),
            })
            .await
            .unwrap();
        assert_eq!(resp.size, 42);
        assert_eq!(resp.content.channel_id, "ec");
        assert_eq!(resp.content.access_key, "ek");
        let (fid, payload) = stub.captured.lock().unwrap().clone().unwrap();
        assert_eq!(fid, "sandbox::fs::read");
        let obj = payload.as_object().unwrap();
        assert_eq!(obj["sandbox_id"].as_str(), Some(id.to_string().as_str()));
        assert_eq!(obj["path"].as_str(), Some("/sb/y"));
        assert!(
            obj.get("content").is_none(),
            "read request must not carry a content field"
        );
    }

    #[tokio::test]
    async fn engine_remote_with_s_code_forwards_directly() {
        struct RemoteFwd;
        #[async_trait]
        impl TriggerFwd for RemoteFwd {
            async fn trigger(&self, _fid: &str, _payload: Value) -> Result<Value, IIIError> {
                Err(IIIError::Remote {
                    code: "S214".into(),
                    message: "directory not empty".into(),
                    stacktrace: None,
                })
            }
        }
        let b = SandboxFsBackend::new(Arc::new(RemoteFwd), true, Uuid::new_v4());
        let err = b
            .rm(RmArgs {
                path: "/x".into(),
                recursive: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S214");
    }

    #[tokio::test]
    async fn engine_remote_with_invocation_failed_recovers_s_code_from_message() {
        // Engine wraps handler errors as Remote { code: "invocation_failed",
        // message: "handler error: ...S211..." } — recover the S-code from
        // the message body.
        struct WrappedFwd;
        #[async_trait]
        impl TriggerFwd for WrappedFwd {
            async fn trigger(&self, _fid: &str, _payload: Value) -> Result<Value, IIIError> {
                Err(IIIError::Remote {
                    code: "invocation_failed".into(),
                    message: "handler error: S211: not found".into(),
                    stacktrace: None,
                })
            }
        }
        let b = SandboxFsBackend::new(Arc::new(WrappedFwd), true, Uuid::new_v4());
        let err = b.ls(LsArgs { path: "/x".into() }).await.unwrap_err();
        assert_eq!(err.code, "S211");
    }

    #[tokio::test]
    async fn grep_forwards_full_payload() {
        let stub = Arc::new(StubFwd {
            captured: Mutex::new(None),
            respond_with: Ok(json!({ "matches": [], "truncated": false })),
        });
        let id = Uuid::new_v4();
        let b = SandboxFsBackend::new(stub.clone(), true, id);
        let _ = b
            .grep(GrepArgs {
                path: "/x".into(),
                pattern: "p".into(),
                recursive: true,
                ignore_case: true,
                include_glob: vec!["*.rs".into()],
                exclude_glob: vec!["target/*".into()],
                max_matches: 10,
                max_line_bytes: 4096,
            })
            .await
            .unwrap();
        let (fid, payload) = stub.captured.lock().unwrap().clone().unwrap();
        assert_eq!(fid, "sandbox::fs::grep");
        let obj = payload.as_object().unwrap();
        assert_eq!(obj["sandbox_id"].as_str(), Some(id.to_string().as_str()));
        assert_eq!(obj["path"].as_str(), Some("/x"));
        assert_eq!(obj["pattern"].as_str(), Some("p"));
        assert_eq!(obj["recursive"].as_bool(), Some(true));
        assert_eq!(obj["ignore_case"].as_bool(), Some(true));
        assert_eq!(obj["max_matches"].as_u64(), Some(10));
        assert_eq!(obj["max_line_bytes"].as_u64(), Some(4096));
        assert_eq!(obj["include_glob"][0].as_str(), Some("*.rs"));
        assert_eq!(obj["exclude_glob"][0].as_str(), Some("target/*"));
    }
}
