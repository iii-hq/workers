//! Integration tests for `SandboxFsBackend`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::IIIError;
use serde_json::{json, Value};
use uuid::Uuid;

use shell::fs::sandbox::{SandboxFsBackend, TriggerFwd};
use shell::fs::{
    ChmodArgs, FsBackend, GrepArgs, LsArgs, MkdirArgs, MvArgs, ReadArgs, RmArgs, SedArgs, StatArgs,
    WriteArgs,
};

#[derive(Debug)]
enum Response {
    Ok(Value),
    HandlerErr(String),
    RemoteErr { code: String, message: String },
}

struct StubFwd {
    captured: Mutex<Vec<(String, Value)>>,
    next: Mutex<Option<Response>>,
}

impl StubFwd {
    fn new(resp: Response) -> Arc<Self> {
        Arc::new(Self {
            captured: Mutex::new(Vec::new()),
            next: Mutex::new(Some(resp)),
        })
    }
    fn last_call(&self) -> (String, Value) {
        self.captured
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("no call captured")
    }
}

#[async_trait]
impl TriggerFwd for StubFwd {
    async fn trigger(&self, fid: &str, payload: Value) -> Result<Value, IIIError> {
        self.captured.lock().unwrap().push((fid.into(), payload));
        match self.next.lock().unwrap().take() {
            Some(Response::Ok(v)) => Ok(v),
            Some(Response::HandlerErr(s)) => Err(IIIError::Handler(s)),
            Some(Response::RemoteErr { code, message }) => Err(IIIError::Remote {
                code,
                message,
                stacktrace: None,
            }),
            None => panic!("StubFwd called more than once with no scripted response"),
        }
    }
}

fn backend(stub: Arc<StubFwd>) -> SandboxFsBackend {
    SandboxFsBackend::new(stub, true, Uuid::new_v4())
}

#[tokio::test]
async fn stat_forwards_path_with_sandbox_id() {
    let stub = StubFwd::new(Response::Ok(json!({
        "name": "x.txt", "is_dir": false, "size": 5, "mode": "0644",
        "mtime": 0, "is_symlink": false,
    })));
    let resp = backend(stub.clone())
        .stat(StatArgs {
            path: "/sb/x.txt".into(),
        })
        .await
        .expect("stat round-trip");
    assert_eq!(resp.0.size, 5);
    let (fid, payload) = stub.last_call();
    assert_eq!(fid, "sandbox::fs::stat");
    assert_eq!(payload["path"], "/sb/x.txt");
    assert!(payload["sandbox_id"].is_string(), "sandbox_id injected");
}

#[tokio::test]
async fn mkdir_forwards_mode_and_parents_flag() {
    let stub = StubFwd::new(Response::Ok(json!({"created": true})));
    let resp = backend(stub.clone())
        .mkdir(MkdirArgs {
            path: "/sb/dir".into(),
            mode: "0700".into(),
            parents: true,
        })
        .await
        .unwrap();
    assert!(resp.created);
    let (_, payload) = stub.last_call();
    assert_eq!(payload["mode"], "0700");
    assert_eq!(payload["parents"], true);
}

#[tokio::test]
async fn rm_forwards_recursive_flag() {
    let stub = StubFwd::new(Response::Ok(json!({"removed": true})));
    backend(stub.clone())
        .rm(RmArgs {
            path: "/sb/d".into(),
            recursive: true,
        })
        .await
        .unwrap();
    let (_, payload) = stub.last_call();
    assert_eq!(payload["recursive"], true);
}

#[tokio::test]
async fn chmod_forwards_uid_gid_recursive() {
    let stub = StubFwd::new(Response::Ok(json!({"updated": 7})));
    let resp = backend(stub.clone())
        .chmod(ChmodArgs {
            path: "/sb/d".into(),
            mode: "0644".into(),
            uid: Some(1000),
            gid: Some(1000),
            recursive: true,
        })
        .await
        .unwrap();
    assert_eq!(resp.entries_changed, 7);
    let (_, payload) = stub.last_call();
    assert_eq!(payload["uid"], 1000);
    assert_eq!(payload["gid"], 1000);
    assert_eq!(payload["recursive"], true);
}

#[tokio::test]
async fn mv_forwards_overwrite_flag() {
    let stub = StubFwd::new(Response::Ok(json!({"moved": true})));
    backend(stub.clone())
        .mv(MvArgs {
            src: "/sb/a".into(),
            dst: "/sb/b".into(),
            overwrite: true,
        })
        .await
        .unwrap();
    let (_, payload) = stub.last_call();
    assert_eq!(payload["src"], "/sb/a");
    assert_eq!(payload["dst"], "/sb/b");
    assert_eq!(payload["overwrite"], true);
}

#[tokio::test]
async fn grep_forwards_full_payload() {
    let stub = StubFwd::new(Response::Ok(json!({
        "matches": [{"path": "/sb/f", "line": 1, "content": "x"}],
        "truncated": false,
    })));
    backend(stub.clone())
        .grep(GrepArgs {
            path: "/sb".into(),
            pattern: "needle".into(),
            recursive: true,
            ignore_case: true,
            include_glob: vec!["*.rs".into()],
            exclude_glob: vec!["*.bak".into()],
            max_matches: 50,
            max_line_bytes: 8192,
        })
        .await
        .unwrap();
    let (_, p) = stub.last_call();
    assert_eq!(p["pattern"], "needle");
    assert_eq!(p["ignore_case"], true);
    assert_eq!(p["max_matches"], 50);
    assert_eq!(p["max_line_bytes"], 8192);
    assert_eq!(p["include_glob"], json!(["*.rs"]));
    assert_eq!(p["exclude_glob"], json!(["*.bak"]));
}

#[tokio::test]
async fn sed_forwards_full_payload() {
    let stub = StubFwd::new(Response::Ok(json!({
        "results": [{"path": "/sb/a", "replacements": 1, "success": true}],
        "total_replacements": 1,
    })));
    backend(stub.clone())
        .sed(SedArgs {
            files: vec!["/sb/a".into()],
            path: None,
            recursive: false,
            include_glob: vec![],
            exclude_glob: vec![],
            pattern: "foo".into(),
            replacement: "bar".into(),
            regex: true,
            first_only: false,
            ignore_case: false,
        })
        .await
        .unwrap();
    let (_, p) = stub.last_call();
    assert_eq!(p["pattern"], "foo");
    assert_eq!(p["replacement"], "bar");
    assert_eq!(p["regex"], true);
    assert_eq!(p["first_only"], false);
}

#[tokio::test]
async fn read_forwards_path_and_returns_engine_response() {
    let stub = StubFwd::new(Response::Ok(json!({
        "content": {"channel_id": "c1", "access_key": "k1", "direction": "read"},
        "size": 100,
        "mode": "0644",
        "mtime": 1700000000,
    })));
    let resp = backend(stub.clone())
        .read(ReadArgs {
            path: "/sb/file".into(),
        })
        .await
        .unwrap();
    assert_eq!(resp.size, 100);
    assert_eq!(resp.mode, "0644");
    let (_, p) = stub.last_call();
    assert_eq!(p["path"], "/sb/file");
}

#[tokio::test]
async fn write_forwards_content_ref_verbatim() {
    let stub = StubFwd::new(Response::Ok(json!({
        "bytes_written": 42, "path": "/sb/x",
    })));
    let content_ref = iii_sdk::channels::StreamChannelRef {
        channel_id: "c-input".into(),
        access_key: "k-input".into(),
        direction: iii_sdk::channels::ChannelDirection::Read,
    };
    let resp = backend(stub.clone())
        .write(WriteArgs {
            path: "/sb/x".into(),
            mode: "0600".into(),
            parents: false,
            content: content_ref.clone(),
        })
        .await
        .unwrap();
    assert_eq!(resp.bytes_written, 42);
    let (_, p) = stub.last_call();
    assert_eq!(p["content"]["channel_id"], "c-input");
    assert_eq!(p["content"]["access_key"], "k-input");
    assert_eq!(p["mode"], "0600");
    assert_eq!(p["parents"], false);
}

#[tokio::test]
async fn remote_error_with_s_code_round_trips() {
    let stub = StubFwd::new(Response::RemoteErr {
        code: "S213".into(),
        message: "destination exists".into(),
    });
    let err = backend(stub)
        .ls(LsArgs { path: "/sb".into() })
        .await
        .unwrap_err();
    assert_eq!(err.code, "S213");
    assert!(err.message.contains("forwarded from engine"));
}

#[tokio::test]
async fn remote_error_with_unknown_s_code_collapses_to_s216() {
    let stub = StubFwd::new(Response::RemoteErr {
        code: "S299".into(),
        message: "future code".into(),
    });
    let err = backend(stub)
        .ls(LsArgs { path: "/sb".into() })
        .await
        .unwrap_err();
    assert_eq!(err.code, "S216");
}

#[tokio::test]
async fn remote_error_invocation_failed_with_s_code_in_message_recovers() {
    let stub = StubFwd::new(Response::RemoteErr {
        code: "invocation_failed".into(),
        message: "handler error: S211: not found in /sb/missing".into(),
    });
    let err = backend(stub)
        .stat(StatArgs {
            path: "/sb/missing".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, "S211");
    assert!(err.message.contains("wrapped"));
}

#[tokio::test]
async fn remote_error_invocation_failed_without_s_code_falls_back_to_s216() {
    let stub = StubFwd::new(Response::RemoteErr {
        code: "invocation_failed".into(),
        message: "handler error: just a string with no code".into(),
    });
    let err = backend(stub)
        .ls(LsArgs { path: "/sb".into() })
        .await
        .unwrap_err();
    assert_eq!(err.code, "S216");
}

#[tokio::test]
async fn handler_error_with_json_payload_recovers_code() {
    let stub = StubFwd::new(Response::HandlerErr(
        r#"{"code":"S214","message":"directory not empty"}"#.into(),
    ));
    let err = backend(stub)
        .rm(RmArgs {
            path: "/sb/d".into(),
            recursive: false,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, "S214");
}

#[tokio::test]
async fn handler_error_with_raw_s_code_in_string_recovers_via_scan() {
    let stub = StubFwd::new(Response::HandlerErr(
        "something broke S217 bad regex".into(),
    ));
    let err = backend(stub)
        .grep(GrepArgs {
            path: "/sb".into(),
            pattern: "[".into(),
            recursive: true,
            ignore_case: false,
            include_glob: vec![],
            exclude_glob: vec![],
            max_matches: 10,
            max_line_bytes: 4096,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, "S217");
}

#[tokio::test]
async fn handler_error_without_s_code_falls_back_to_s216() {
    let stub = StubFwd::new(Response::HandlerErr("just a plain old string".into()));
    let err = backend(stub)
        .ls(LsArgs { path: "/sb".into() })
        .await
        .unwrap_err();
    assert_eq!(err.code, "S216");
}

#[tokio::test]
async fn engine_response_missing_required_field_is_s216() {
    let stub = StubFwd::new(Response::Ok(json!({})));
    let err = backend(stub)
        .ls(LsArgs { path: "/sb".into() })
        .await
        .unwrap_err();
    assert_eq!(err.code, "S216");
    assert!(err.message.contains("bad engine response"));
}

#[tokio::test]
async fn disabled_sandbox_returns_s210_without_calling_engine() {
    let stub = StubFwd::new(Response::Ok(json!({"entries": []})));
    let b = SandboxFsBackend::new(stub.clone(), false, Uuid::new_v4());
    let err = b.ls(LsArgs { path: "/sb".into() }).await.unwrap_err();
    assert_eq!(err.code, "S210");
    assert!(
        stub.captured.lock().unwrap().is_empty(),
        "disabled sandbox should short-circuit before forwarding",
    );
}

#[tokio::test]
async fn scan_s_code_skips_codes_glued_to_an_identifier() {
    let stub = StubFwd::new(Response::HandlerErr("fooS211bar should not match".into()));
    let err = backend(stub)
        .ls(LsArgs { path: "/sb".into() })
        .await
        .unwrap_err();
    assert_eq!(err.code, "S216");
}
