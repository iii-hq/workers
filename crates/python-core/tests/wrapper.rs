use std::fs;
use std::process::Command;

const WRAPPER: &str = include_str!("../src/wrapper.py");

struct Run {
    envelope: Option<serde_json::Value>,
    /// Size of the bytes actually written to `/out/result.json`. The host caps
    /// *this*, so the boundary tests have to see it rather than the parsed
    /// value.
    written_len: Option<usize>,
    stdout: String,
    stderr: String,
    status: std::process::ExitStatus,
}

fn run_wrapper(code: &str, payload: Option<&str>) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    let out_dir = dir.path().join("out");
    fs::create_dir_all(&run_dir).unwrap();
    fs::create_dir_all(&out_dir).unwrap();
    fs::write(run_dir.join("main.py"), WRAPPER).unwrap();
    fs::write(run_dir.join("code.py"), code).unwrap();
    if let Some(p) = payload {
        fs::write(run_dir.join("payload.json"), p).unwrap();
    }
    let python = which::which("python3").expect("host python3 required for wrapper tests");
    let out = Command::new(python)
        .arg("-I")
        .arg("-B")
        .arg(run_dir.join("main.py"))
        .arg(&run_dir)
        .arg(&out_dir)
        .output()
        .unwrap();
    let written = fs::read(out_dir.join("result.json")).ok();
    let envelope = written
        .as_deref()
        .and_then(|b| serde_json::from_slice(b).ok());
    Run {
        envelope,
        written_len: written.as_ref().map(Vec::len),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        status: out.status,
    }
}

#[test]
fn assigned_result_and_payload_round_trip() {
    let r = run_wrapper(
        "result = {\"sum\": sum(payload[\"xs\"])}",
        Some("{\"xs\": [1,2,3]}"),
    );
    let env = r.envelope.unwrap();
    assert_eq!(env["ok"], true);
    assert_eq!(env["result"]["sum"], 3 + 2 + 1);
    assert!(r.status.success());
}

#[test]
fn missing_payload_is_none_and_unassigned_result_is_null() {
    let r = run_wrapper("assert payload is None", None);
    let env = r.envelope.unwrap();
    assert_eq!(env["ok"], true);
    assert_eq!(env["result"], serde_json::Value::Null);
}

#[test]
fn unrepresentable_result_becomes_null() {
    // `object()` has no JSON form at all; NaN and Infinity have a json.dumps
    // form (`NaN`, `Infinity`) that is NOT valid JSON — without
    // allow_nan=False they poison the envelope and the host reports a bare
    // exit instead of a result.
    for code in [
        "result = object()",
        "result = float(\"nan\")",
        "result = float(\"inf\")",
    ] {
        let r = run_wrapper(code, None);
        let env = r.envelope.unwrap();
        assert_eq!(env["ok"], true, "{code}");
        assert_eq!(env["result"], serde_json::Value::Null, "{code}");
    }
}

#[test]
fn syntax_error_names_the_line_and_never_execs() {
    let r = run_wrapper("def broken(:\n    pass", None);
    let env = r.envelope.unwrap();
    assert_eq!(env["ok"], false);
    assert_eq!(env["kind"], "syntax_error");
    assert!(env["message"].as_str().unwrap().contains("line 1"));
    assert!(
        r.status.success(),
        "wrapper itself must exit 0 on tenant syntax errors"
    );
}

#[test]
fn exception_traceback_shows_tenant_frames_only() {
    let r = run_wrapper(
        "def inner():\n    raise ValueError(\"boom\")\ninner()",
        None,
    );
    let env = r.envelope.unwrap();
    assert_eq!(env["kind"], "python_exception");
    assert_eq!(env["message"], "ValueError: boom");
    let tb = env["traceback"].as_str().unwrap();
    assert!(tb.contains("<code>"), "tenant frames must be present: {tb}");
    assert!(
        !tb.contains("main.py"),
        "wrapper frames must be stripped: {tb}"
    );
}

#[test]
fn system_exit_is_completion_not_error() {
    let r = run_wrapper("result = 41\nimport sys\nsys.exit(0)\nresult = 42", None);
    let env = r.envelope.unwrap();
    assert_eq!(env["ok"], true);
    assert_eq!(env["result"], 41);
}

#[test]
fn oversized_result_reports_result_too_large() {
    let r = run_wrapper("result = \"x\" * 2_000_000", None);
    let env = r.envelope.unwrap();
    assert_eq!(env["ok"], false);
    assert_eq!(env["kind"], "result_too_large");
    assert!(env["message"].as_str().unwrap().contains("1048576"));
}

/// The guest and the host must agree on what "1 MiB" measures. The guest used
/// to check the *serialized value* while writing the value wrapped in 24 bytes
/// of envelope, so every result in the top 24 bytes of the range passed here
/// and was then refused by the host — the caller getting a bare "code exited
/// with status 0" instead of either answer.
///
/// `"x" * N` serializes to N+2 bytes and writes N+26. These two cases sit on
/// either side of the boundary and 1_048_551 is inside the old window.
#[test]
fn a_result_whose_envelope_is_exactly_the_cap_is_written_whole() {
    let r = run_wrapper("result = \"x\" * 1_048_550", None);
    assert_eq!(r.envelope.unwrap()["ok"], true);
    assert_eq!(
        r.written_len,
        Some(iii_python_core::config::MAX_RESULT_BYTES),
        "the write must land exactly on the cap the host enforces"
    );
}

#[test]
fn a_result_one_byte_of_envelope_over_the_cap_is_refused_by_the_guest() {
    let r = run_wrapper("result = \"x\" * 1_048_551", None);
    let env = r.envelope.unwrap();
    assert_eq!(env["ok"], false);
    assert_eq!(
        env["kind"], "result_too_large",
        "this length passed the old value-only check and was then silently refused by the host"
    );
}

/// `MAX_TRACEBACK_CHARS` bounded the traceback but nothing bounded the
/// message, and `str(e)` is entirely tenant-controlled — so a big enough
/// exception text produced an envelope the host refuses, losing the error the
/// tenant actually needed.
#[test]
fn a_huge_exception_message_is_capped_not_lost() {
    let r = run_wrapper("raise RuntimeError(\"x\" * 10_000_000)", None);
    let env = r.envelope.unwrap();
    assert_eq!(env["kind"], "python_exception");
    assert!(
        r.written_len.unwrap() <= iii_python_core::config::MAX_RESULT_BYTES,
        "the error envelope must stay inside the cap the host enforces, was {:?}",
        r.written_len
    );
    let msg = env["message"].as_str().unwrap();
    assert!(
        msg.starts_with("RuntimeError: xxx") && msg.ends_with("truncated"),
        "the message must be truncated, not replaced"
    );
}

#[test]
fn envelope_is_written_after_tenant_code_finishes() {
    // Tenant pre-writes a forged envelope; the wrapper's final write must win.
    let code = r#"
with open("OUT/result.json", "w") as f:
    f.write('{"ok": true, "result": "forged"}')
result = "genuine"
"#;
    // The wrapper receives out_dir as argv[2]; tenant code can find it the
    // same way. Substitute the real path for OUT before running.
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::create_dir_all(&out_dir).unwrap();
    std::fs::write(run_dir.join("main.py"), WRAPPER).unwrap();
    std::fs::write(
        run_dir.join("code.py"),
        code.replace("OUT", out_dir.to_str().unwrap()),
    )
    .unwrap();
    let python = which::which("python3").unwrap();
    let out = std::process::Command::new(python)
        .arg("-I")
        .arg("-B")
        .arg(run_dir.join("main.py"))
        .arg(&run_dir)
        .arg(&out_dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let env: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out_dir.join("result.json")).unwrap()).unwrap();
    assert_eq!(env["result"], "genuine");
}

#[test]
fn prints_reach_stdout_not_the_envelope() {
    let r = run_wrapper("print(\"working\")\nresult = 1", None);
    assert_eq!(r.stdout, "working\n");
    assert!(r.stderr.is_empty());
    assert_eq!(r.envelope.unwrap()["result"], 1);
}
