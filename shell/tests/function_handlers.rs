//! Integration tests for the typed `functions::*::handle` async fns.

use std::sync::Arc;

use iii_sdk::III;
use serde_json::{json, Value};

use shell::config::ShellConfig;
use shell::functions;
use shell::functions::types::{ExecBgRequest, ExecRequest, KillRequest, StatusRequest};
use shell::jobs::{self, now_ms, JobHandle, JobRecord, JobStatus};

async fn seed(handle: JobHandle) -> String {
    match jobs::try_reserve_and_insert(handle, usize::MAX).await {
        Ok(id) => id,
        Err(_) => panic!("usize::MAX cap must always accept"),
    }
}

/// Build a test config with `allow` populated as the shell-side allowlist.
/// Empty `allow` (the default in most call sites that don't care about
/// policy) leaves the list empty → pass-through, matching today's default
/// where the approval-gate (when present) is the sole policy layer.
fn cfg_with_allow(allow: &[&str]) -> Arc<ShellConfig> {
    cfg_with_policy(allow, &[])
}

fn cfg_with_policy(allow: &[&str], deny: &[&str]) -> Arc<ShellConfig> {
    let mut c = ShellConfig {
        max_timeout_ms: 5000,
        default_timeout_ms: 1500,
        max_output_bytes: 4096,
        allowlist: allow.iter().map(|s| s.to_string()).collect(),
        denylist_patterns: deny.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    c.compile_denylist().expect("test denylist compiles");
    Arc::new(c)
}

fn fresh_iii() -> III {
    III::new("ws://stub-not-connected:0")
}

fn tmpdir(prefix: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("shell-fn-{}-{}", prefix, uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn typed<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).expect("test fixture must match the typed shape")
}

fn resp<T: serde::Serialize>(t: T) -> Value {
    serde_json::to_value(t).expect("typed response must serialize")
}

/// Run a payload through the same deserialize path the SDK uses at the engine
/// boundary, returning the error string a caller (e.g. an LLM) would actually
/// see. Mirrors `IntoAsyncHandler::into_handler` in `iii-sdk`.
fn parse_err<T: serde::de::DeserializeOwned>(v: Value) -> String {
    match serde_json::from_value::<T>(v) {
        Ok(_) => panic!("expected deserialization to fail"),
        Err(e) => e.to_string(),
    }
}

#[tokio::test]
async fn exec_handler_runs_allowlisted_command() {
    let cfg = cfg_with_allow(&["echo"]);
    let r = resp(
        functions::exec::handle(
            cfg,
            fresh_iii(),
            typed::<ExecRequest>(json!({"command": "echo", "args": ["ok"]})),
        )
        .await
        .unwrap(),
    );
    assert_eq!(r["exit_code"], 0);
    assert_eq!(r["stdout"], "ok\n");
    assert_eq!(r["timed_out"], false);
}

/// `command` is a required field — serde produces "missing field `command`"
/// when absent. The SDK forwards that string verbatim as the trigger error.
#[test]
fn exec_request_rejects_missing_command() {
    let err = parse_err::<ExecRequest>(json!({"args": ["ok"]}));
    assert!(err.contains("missing"), "got: {err}");
    assert!(err.contains("command"), "got: {err}");
}

/// Regression: LLMs commonly send the subprocess-style argv array
/// `{"command": ["sh", "-lc", "..."]}` here. The error message MUST distinguish
/// "wrong type" from "missing" so the LLM can self-correct on its next turn,
/// and MUST hint at the right shape (split program/args across two fields).
#[test]
fn exec_request_rejects_array_command_with_helpful_error() {
    let err = parse_err::<ExecRequest>(json!({"command": ["sh", "-lc", "ls -la"]}));
    assert!(!err.contains("missing"), "got: {err}");
    assert!(
        err.contains("'command'") && err.contains("string"),
        "got: {err}"
    );
    assert!(
        err.contains("args"),
        "error must hint at 'args' field, got: {err}"
    );
}

/// With a non-empty shell-side allowlist, an unlisted command is rejected
/// at the shell boundary before spawn — this is the fallback floor for
/// standalone deployments where no approval-gate sits upstream.
#[tokio::test]
async fn exec_handler_rejects_unlisted_command() {
    let cfg = cfg_with_allow(&["echo"]);
    let err = functions::exec::handle(
        cfg,
        fresh_iii(),
        typed::<ExecRequest>(json!({"command": "nmap", "args": ["scanme.nmap.org"]})),
    )
    .await
    .expect_err("unlisted command must be rejected");
    assert!(err.contains("not in allowlist"), "got: {err}");
}

#[tokio::test]
async fn exec_bg_handler_rejects_unlisted_command() {
    let cfg = cfg_with_allow(&["sleep"]);
    let err = functions::exec_bg::handle(
        cfg,
        fresh_iii(),
        typed::<ExecBgRequest>(json!({"command": "nmap", "args": ["scanme.nmap.org"]})),
    )
    .await
    .expect_err("unlisted background command must be rejected");
    assert!(err.contains("not in allowlist"), "got: {err}");
}

#[tokio::test]
async fn exec_handler_rejects_denylisted_command_before_spawn() {
    let cfg = cfg_with_policy(&["rm"], &[r"rm\s+-rf\s+/"]);
    let err = functions::exec::handle(
        cfg,
        fresh_iii(),
        typed::<ExecRequest>(json!({"command": "rm", "args": ["-rf", "/"]})),
    )
    .await
    .expect_err("denylist must reject even when allowlist permits");
    assert!(err.contains("denylist"), "got: {err}");
}

#[tokio::test]
async fn exec_bg_handler_rejects_denylisted_command_before_spawn() {
    let cfg = cfg_with_policy(&["rm"], &[r"rm\s+-rf\s+/"]);
    let err = functions::exec_bg::handle(
        cfg,
        fresh_iii(),
        typed::<ExecBgRequest>(json!({"command": "rm", "args": ["-rf", "/"]})),
    )
    .await
    .expect_err("denylist must reject background command before spawn");
    assert!(err.contains("denylist"), "got: {err}");
}

/// With both shell-side lists empty (today's default), every argv runs —
/// the approval-gate, when wired upstream, remains sole authority.
#[tokio::test]
async fn exec_handler_passthrough_when_lists_empty() {
    let cfg = cfg_with_allow(&[]);
    let r = resp(
        functions::exec::handle(
            cfg,
            fresh_iii(),
            typed::<ExecRequest>(json!({"command": "echo", "args": ["pass"]})),
        )
        .await
        .unwrap(),
    );
    assert_eq!(r["exit_code"], 0);
    assert_eq!(r["stdout"], "pass\n");
}

/// `args[i]` validation is per-index; a non-string element must be rejected
/// with a message that names which index failed and what it actually was.
#[test]
fn exec_request_rejects_non_string_arg() {
    let err = parse_err::<ExecRequest>(json!({"command": "echo", "args": ["a", 5]}));
    assert!(err.contains("must be a string"), "got: {err}");
    assert!(err.contains("args[1]"), "got: {err}");
}

/// `timeout_ms: -1` and `timeout_ms: 1.5` were the original silent-fallback
/// cases on the loose `Value` handler. The custom `deserialize_timeout_ms`
/// preserves that semantic on the typed struct: bad values become None and
/// the call falls through to `cfg.default_timeout_ms`.
#[test]
fn exec_request_silently_drops_negative_or_float_timeout() {
    let req: ExecRequest = typed(json!({"command": "echo", "args": [], "timeout_ms": -1}));
    assert!(req.timeout_ms.is_none());
    let req: ExecRequest = typed(json!({"command": "echo", "args": [], "timeout_ms": 1.5}));
    assert!(req.timeout_ms.is_none());
}

#[tokio::test]
async fn exec_handler_returns_truncated_flag_at_max_output_bytes() {
    let cfg = cfg_with_allow(&["sh"]);
    let r = resp(
        functions::exec::handle(
            cfg,
            fresh_iii(),
            typed::<ExecRequest>(json!({
                "command": "sh",
                "args": ["-c", "printf 'x%.0s' $(seq 1 8000)"],
            })),
        )
        .await
        .unwrap(),
    );
    assert_eq!(r["exit_code"], 0);
    assert_eq!(r["stdout_truncated"], true);
    assert_eq!(r["stdout"].as_str().unwrap().len(), 4096);
}

#[tokio::test]
async fn exec_bg_handler_spawns_returns_job_id_and_argv() {
    let cfg = cfg_with_allow(&["sleep"]);
    let r = resp(
        functions::exec_bg::handle(
            cfg,
            fresh_iii(),
            typed::<ExecBgRequest>(json!({"command": "sleep", "args": ["0.1"]})),
        )
        .await
        .unwrap(),
    );
    assert!(r["job_id"].is_string());
    assert_eq!(r["argv"], json!(["sleep", "0.1"]));
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
}

/// Regression: same as `exec_request_rejects_array_command_with_helpful_error`,
/// for the background variant.
#[test]
fn exec_bg_request_rejects_array_command_with_helpful_error() {
    let err = parse_err::<ExecBgRequest>(json!({"command": ["sh", "-lc", "ls -la"]}));
    assert!(!err.contains("missing"), "got: {err}");
    assert!(
        err.contains("'command'") && err.contains("string"),
        "got: {err}"
    );
    assert!(
        err.contains("args"),
        "error must hint at 'args' field, got: {err}"
    );
}

#[test]
fn exec_bg_request_rejects_non_string_arg() {
    let err = parse_err::<ExecBgRequest>(json!({"command": "echo", "args": [42]}));
    assert!(err.contains("must be a string"), "got: {err}");
    assert!(err.contains("args[0]"), "got: {err}");
}

#[tokio::test]
async fn status_handler_returns_record_for_inserted_job() {
    let id = "fn-status-handler-test-1";
    seed(JobHandle {
        record: JobRecord {
            id: id.into(),
            argv: vec!["echo".into()],
            started_at_ms: now_ms(),
            finished_at_ms: None,
            status: JobStatus::Running,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        },
        child: None,
    })
    .await;
    let r = resp(
        functions::status::handle(typed::<StatusRequest>(json!({"job_id": id})))
            .await
            .unwrap(),
    );
    assert_eq!(r["job"]["id"], id);
    assert_eq!(r["job"]["status"], "running");
}

#[tokio::test]
async fn status_handler_rejects_unknown_job_id() {
    let err = functions::status::handle(typed::<StatusRequest>(
        json!({"job_id": "fn-status-never-existed"}),
    ))
    .await
    .unwrap_err();
    assert!(err.contains("no such job"));
}

#[tokio::test]
async fn status_handler_rejects_missing_job_id() {
    let r: Result<StatusRequest, _> = serde_json::from_value(json!({}));
    assert!(r.is_err());
}

#[tokio::test]
async fn kill_handler_rejects_unknown_job_id() {
    let err = functions::kill::handle(typed::<KillRequest>(
        json!({"job_id": "fn-kill-never-existed"}),
    ))
    .await
    .unwrap_err();
    assert!(err.contains("no such job"));
}

#[tokio::test]
async fn kill_handler_returns_killed_false_when_job_already_terminal() {
    let id = "fn-kill-handler-finished";
    seed(JobHandle {
        record: JobRecord {
            id: id.into(),
            argv: vec!["echo".into()],
            started_at_ms: now_ms(),
            finished_at_ms: Some(now_ms()),
            status: JobStatus::Finished,
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        },
        child: None,
    })
    .await;
    let r = resp(
        functions::kill::handle(typed::<KillRequest>(json!({"job_id": id})))
            .await
            .unwrap(),
    );
    assert_eq!(r["killed"], false);
    assert_eq!(r["status"], "finished");
    assert!(r["reason"].is_string());
}

#[tokio::test]
async fn list_handler_returns_jobs_array_and_count() {
    let id = "fn-list-handler-marker";
    seed(JobHandle {
        record: JobRecord {
            id: id.into(),
            argv: vec!["sleep".into(), "0.01".into()],
            started_at_ms: now_ms(),
            finished_at_ms: None,
            status: JobStatus::Running,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        },
        child: None,
    })
    .await;
    let r = resp(
        functions::list::handle(cfg_with_allow(&["sleep"]))
            .await
            .unwrap(),
    );
    let jobs_arr = r["jobs"].as_array().unwrap();
    assert!(
        jobs_arr.iter().any(|j| j["id"] == id),
        "list missing {id}: {jobs_arr:?}",
    );
    assert!(r["count"].is_number());
}

fn fs_host_backend() -> Arc<dyn shell::fs::FsBackend> {
    use std::fmt;

    #[derive(Debug)]
    struct StubChan;
    #[async_trait::async_trait]
    impl shell::fs::host::ChannelMaker for StubChan {
        async fn create_channel(&self, _: usize) -> Result<iii_sdk::Channel, iii_sdk::IIIError> {
            Err(iii_sdk::IIIError::Handler("stub channel".into()))
        }
        fn engine_address(&self) -> String {
            "ws://stub:0".into()
        }
    }
    impl fmt::Display for StubChan {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "StubChan")
        }
    }

    Arc::new(shell::fs::host::HostFsBackend::new(
        Arc::new(shell::fs::host::HostFsConfig::default()),
        Arc::new(StubChan),
    ))
}

#[tokio::test]
async fn fs_ls_handler_lists_directory_entries() {
    let root = tmpdir("fn-fs-ls");
    std::fs::write(root.join("a.txt"), b"hi").unwrap();
    let r = resp(
        functions::fs_ls::handle(
            fs_host_backend(),
            fresh_iii(),
            true,
            json!({"path": root.to_string_lossy()}),
        )
        .await
        .unwrap(),
    );
    let entries = r["entries"].as_array().unwrap();
    assert!(entries.iter().any(|e| e["name"] == "a.txt"));
}

#[tokio::test]
async fn fs_stat_handler_returns_entry_shape() {
    let root = tmpdir("fn-fs-stat");
    let f = root.join("a.txt");
    std::fs::write(&f, b"hello").unwrap();
    let r = resp(
        functions::fs_stat::handle(
            fs_host_backend(),
            fresh_iii(),
            true,
            json!({"path": f.to_string_lossy()}),
        )
        .await
        .unwrap(),
    );
    assert_eq!(r["size"], 5);
    assert_eq!(r["is_dir"], false);
}

#[tokio::test]
async fn fs_mkdir_handler_creates_directory() {
    let root = tmpdir("fn-fs-mkdir");
    let d = root.join("new");
    let r = resp(
        functions::fs_mkdir::handle(
            fs_host_backend(),
            fresh_iii(),
            true,
            json!({"path": d.to_string_lossy(), "mode": "0755"}),
        )
        .await
        .unwrap(),
    );
    assert_eq!(r["created"], true);
    assert!(d.is_dir());
}

#[tokio::test]
async fn fs_rm_handler_removes_file() {
    let root = tmpdir("fn-fs-rm");
    let f = root.join("doomed.txt");
    std::fs::write(&f, b"x").unwrap();
    let r = resp(
        functions::fs_rm::handle(
            fs_host_backend(),
            fresh_iii(),
            true,
            json!({"path": f.to_string_lossy()}),
        )
        .await
        .unwrap(),
    );
    assert_eq!(r["removed"], true);
    assert!(!f.exists());
}

#[tokio::test]
async fn fs_chmod_handler_sets_mode() {
    use std::os::unix::fs::PermissionsExt;
    let root = tmpdir("fn-fs-chmod");
    let f = root.join("a.txt");
    std::fs::write(&f, b"x").unwrap();
    let r = resp(
        functions::fs_chmod::handle(
            fs_host_backend(),
            fresh_iii(),
            true,
            json!({"path": f.to_string_lossy(), "mode": "0640"}),
        )
        .await
        .unwrap(),
    );
    assert!(r["updated"].as_u64().unwrap() >= 1);
    let mode = std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640);
}

#[tokio::test]
async fn fs_mv_handler_renames() {
    let root = tmpdir("fn-fs-mv");
    let src = root.join("src.txt");
    let dst = root.join("dst.txt");
    std::fs::write(&src, b"x").unwrap();
    let r = resp(
        functions::fs_mv::handle(
            fs_host_backend(),
            fresh_iii(),
            true,
            json!({
                "src": src.to_string_lossy(),
                "dst": dst.to_string_lossy(),
            }),
        )
        .await
        .unwrap(),
    );
    assert_eq!(r["moved"], true);
    assert!(dst.exists() && !src.exists());
}

#[tokio::test]
async fn fs_grep_handler_finds_matches() {
    let root = tmpdir("fn-fs-grep");
    std::fs::write(root.join("a.txt"), b"line one\nfind me\nline three\n").unwrap();
    let r = resp(
        functions::fs_grep::handle(
            fs_host_backend(),
            fresh_iii(),
            true,
            json!({
                "path": root.to_string_lossy(),
                "pattern": "find me",
                "recursive": true,
                "ignore_case": false,
                "include_glob": [],
                "exclude_glob": [],
                "max_matches": 10,
                "max_line_bytes": 4096,
            }),
        )
        .await
        .unwrap(),
    );
    let matches = r["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert!(matches[0]["content"].as_str().unwrap().contains("find me"));
}

#[tokio::test]
async fn fs_sed_handler_replaces_in_files() {
    let root = tmpdir("fn-fs-sed");
    let f = root.join("a.txt");
    std::fs::write(&f, b"foo bar foo\n").unwrap();
    let r = resp(
        functions::fs_sed::handle(
            fs_host_backend(),
            fresh_iii(),
            true,
            json!({
                "files": [f.to_string_lossy()],
                "recursive": false,
                "include_glob": [],
                "exclude_glob": [],
                "pattern": "foo",
                "replacement": "qux",
                "regex": false,
                "first_only": false,
                "ignore_case": false,
            }),
        )
        .await
        .unwrap(),
    );
    assert_eq!(r["total_replacements"], 2);
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "qux bar qux\n");
}

#[tokio::test]
async fn fs_dispatch_split_target_rejects_unknown_kind() {
    let err = functions::fs_ls::handle(
        fs_host_backend(),
        fresh_iii(),
        true,
        json!({"target": {"kind": "not_real"}, "path": "/tmp"}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("S210"), "got: {err}");
}

#[tokio::test]
async fn fs_handler_rejects_bad_payload_shape() {
    // path must be a string. Hits the S210 mapping in fs_ls::handle.
    let err = functions::fs_ls::handle(fs_host_backend(), fresh_iii(), true, json!({"path": 42}))
        .await
        .unwrap_err();
    assert!(err.contains("S210"), "got: {err}");
}

#[allow(dead_code)]
fn _unused_value_marker(_: Value) {}
