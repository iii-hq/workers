//! The runner scripts under the REAL `node` and `python3` — the same
//! binaries the sandbox images ship. Goldens pin the bytes; this proves they
//! work: protocol framing, envelope delivery, error shape, exit codes.
//!
//! FAILS LOUDLY by default when an interpreter is missing: these are the
//! only tests that prove the runner scripts actually work, so a silent skip
//! would let a broken PATH report as a clean, green suite. Opt out
//! explicitly with `ALLOW_MISSING_INTERPRETERS=1` if you knowingly don't
//! have one of the two interpreters installed.

mod support;

use std::io::Write;
use std::process::{Command, Stdio};

use code_runner::runner::{split_sentinel, Lang};

const SENTINEL: &str = "0f7f37e2-golden-sentinel";

struct RunOutcome {
    exit_ok: bool,
    logs: String,
    result: Option<serde_json::Value>,
    stderr: String,
}

fn interpreter_available(bin: &str) -> bool {
    Command::new(bin)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Returns `true` when the caller should skip — only on the explicit
/// `ALLOW_MISSING_INTERPRETERS=1` opt-out. Otherwise a missing interpreter
/// FAILS the test: these tests are the only proof the runner scripts work,
/// so silence here would let a broken PATH masquerade as a passing suite.
fn require(lang: Lang) -> bool {
    if interpreter_available(lang.interpreter()) {
        return false;
    }
    if std::env::var("ALLOW_MISSING_INTERPRETERS").as_deref() == Ok("1") {
        eprintln!(
            "SKIPPED (ALLOW_MISSING_INTERPRETERS=1): {} not on PATH — runner untested for {:?}",
            lang.interpreter(),
            lang
        );
        return true;
    }
    panic!(
        "{bin} is not on PATH, so this test cannot prove the {lang:?} runner works. \
         Install {bin}, or set ALLOW_MISSING_INTERPRETERS=1 to explicitly accept that gap.",
        bin = lang.interpreter(),
    );
}

/// Write the runner + a handler source into a scratch dir, run
/// `<interpreter> <runner> <source>` with `stdin` fed to the child verbatim
/// (no envelope wrapping — used directly by the malformed-envelope tests).
fn spawn_runner(lang: Lang, handler_src: &str, stdin: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("ce-runner-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let runner = dir.join(format!("run.{}", lang.ext()));
    let source = dir.join(format!("handler.{}", lang.ext()));
    std::fs::write(&runner, lang.runner_source()).unwrap();
    std::fs::write(&source, handler_src).unwrap();

    // Pin the interpreter's colour behaviour instead of inheriting the
    // developer's. Node >= 26 formats numbers with ANSI colour even when
    // stdout is a pipe if `FORCE_COLOR` is set — and it is, in at least one
    // shell here — so `console.log('working on', 21)` reaches us as
    // `working on \e[33m21\e[39m` and a plain `contains` assertion fails.
    // That is a property of the terminal, not of the runner under test.
    let mut child = Command::new(lang.interpreter())
        .arg(&runner)
        .arg(&source)
        .env_remove("FORCE_COLOR")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    std::fs::remove_dir_all(&dir).ok();

    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let split = split_sentinel(&stdout, SENTINEL);
    RunOutcome {
        exit_ok: out.status.success(),
        logs: split.logs,
        result: split
            .result
            .as_deref()
            .and_then(|r| serde_json::from_str(r).ok()),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    }
}

/// The normal case: wrap `payload` (a JSON literal) into the
/// `{"sentinel": ..., "payload": ...}` envelope the runner expects on
/// stdin.
fn run_runner(lang: Lang, handler_src: &str, payload: &str) -> RunOutcome {
    let payload_value: serde_json::Value =
        serde_json::from_str(payload).expect("test payload must be valid JSON");
    let envelope =
        serde_json::json!({ "sentinel": SENTINEL, "payload": payload_value }).to_string();
    spawn_runner(lang, handler_src, &envelope)
}

#[test]
fn goldens_pin_both_runner_scripts() {
    let mut failures = Vec::new();
    for (name, contents) in [
        ("run.mjs", Lang::Node.runner_source()),
        ("run.py", Lang::Python.runner_source()),
    ] {
        if let Err(msg) = support::check_golden(&format!("runners/{name}"), contents) {
            failures.push(msg);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn node_happy_path_returns_result_and_logs() {
    if require(Lang::Node) {
        return;
    }
    let out = run_runner(
        Lang::Node,
        "export function handler(p) { console.log('working on', p.n); return { doubled: p.n * 2 }; }",
        r#"{"n": 21}"#,
    );
    assert!(out.exit_ok);
    assert_eq!(out.result, Some(serde_json::json!({ "doubled": 42 })));
    assert!(out.logs.contains("working on 21"), "logs: {}", out.logs);
}

#[test]
fn node_async_handler_is_awaited() {
    if require(Lang::Node) {
        return;
    }
    let out = run_runner(
        Lang::Node,
        "export async function handler(p) { return await Promise.resolve(p.n + 1); }",
        r#"{"n": 1}"#,
    );
    assert!(out.exit_ok);
    assert_eq!(out.result, Some(serde_json::json!(2)));
}

#[test]
fn node_throwing_handler_reports_the_error_and_fails() {
    if require(Lang::Node) {
        return;
    }
    let out = run_runner(
        Lang::Node,
        "export function handler() { throw new Error('boom-7'); }",
        "{}",
    );
    assert!(!out.exit_ok);
    let err = out.result.expect("error result after the sentinel");
    assert!(err["error"].as_str().unwrap().contains("boom-7"));
}

#[test]
fn node_missing_handler_names_the_convention() {
    if require(Lang::Node) {
        return;
    }
    let out = run_runner(Lang::Node, "export const notHandler = 1;", "{}");
    assert!(!out.exit_ok);
    let err = out.result.expect("error result");
    assert!(err["error"].as_str().unwrap().contains("handler(payload)"));
}

#[test]
fn python_happy_path_returns_result_and_logs() {
    if require(Lang::Python) {
        return;
    }
    let out = run_runner(
        Lang::Python,
        "def handler(p):\n    print('working on', p['n'])\n    return {'doubled': p['n'] * 2}\n",
        r#"{"n": 21}"#,
    );
    assert!(out.exit_ok);
    assert_eq!(out.result, Some(serde_json::json!({ "doubled": 42 })));
    assert!(out.logs.contains("working on 21"), "logs: {}", out.logs);
}

#[test]
fn python_raising_handler_reports_the_error_and_fails() {
    if require(Lang::Python) {
        return;
    }
    let out = run_runner(
        Lang::Python,
        "def handler(p):\n    raise ValueError('boom-9')\n",
        "{}",
    );
    assert!(!out.exit_ok);
    let err = out.result.expect("error result");
    let msg = err["error"].as_str().unwrap();
    assert!(
        msg.contains("ValueError") && msg.contains("boom-9"),
        "{msg}"
    );
}

#[test]
fn python_async_def_is_refused_with_a_clear_message() {
    if require(Lang::Python) {
        return;
    }
    let out = run_runner(Lang::Python, "async def handler(p):\n    return 1\n", "{}");
    assert!(!out.exit_ok);
    let err = out.result.expect("error result");
    assert!(err["error"].as_str().unwrap().contains("async def"));
}

#[test]
fn python_unserializable_result_is_a_handler_error() {
    if require(Lang::Python) {
        return;
    }
    let out = run_runner(Lang::Python, "def handler(p):\n    return object()\n", "{}");
    assert!(!out.exit_ok);
    let err = out.result.expect("error result");
    assert!(err["error"].as_str().unwrap().contains("TypeError"));
}

#[test]
fn node_null_payload_reaches_handler_as_null() {
    if require(Lang::Node) {
        return;
    }
    let out = run_runner(
        Lang::Node,
        "export function handler(p) { return p === null; }",
        "null",
    );
    assert!(out.exit_ok);
    assert_eq!(out.result, Some(serde_json::json!(true)));
}

#[test]
fn node_malformed_envelope_exits_nonzero_with_a_diagnostic_and_no_frame() {
    if require(Lang::Node) {
        return;
    }
    let out = spawn_runner(
        Lang::Node,
        "export function handler(p) { return p; }",
        "not json",
    );
    assert!(!out.exit_ok);
    assert_eq!(
        out.result, None,
        "no sentinel was ever established, so there can be no frame"
    );
    assert!(!out.stderr.is_empty(), "expected a diagnostic on stderr");
}

#[test]
fn python_malformed_envelope_exits_nonzero_with_a_diagnostic_and_no_frame() {
    if require(Lang::Python) {
        return;
    }
    // Completely missing envelope (empty stdin) rather than invalid JSON —
    // covers a different real trigger (child stdin closed with no bytes)
    // than the Node test above (bytes present but not valid JSON).
    let out = spawn_runner(Lang::Python, "def handler(p):\n    return p\n", "");
    assert!(!out.exit_ok);
    assert_eq!(
        out.result, None,
        "no sentinel was ever established, so there can be no frame"
    );
    assert!(!out.stderr.is_empty(), "expected a diagnostic on stderr");
}

#[test]
fn node_dangling_timer_output_after_the_frame_does_not_corrupt_the_result() {
    if require(Lang::Node) {
        return;
    }
    let out = run_runner(
        Lang::Node,
        "export function handler(p) { setTimeout(() => console.log('late'), 50); return { ok: true }; }",
        "{}",
    );
    assert!(out.exit_ok);
    assert_eq!(out.result, Some(serde_json::json!({ "ok": true })));
}

#[test]
fn python_dangling_thread_output_after_the_frame_does_not_corrupt_the_result() {
    if require(Lang::Python) {
        return;
    }
    let out = run_runner(
        Lang::Python,
        "import threading\nimport time\n\n\ndef handler(p):\n    def late():\n        time.sleep(0.05)\n        print('late')\n    threading.Thread(target=late).start()\n    return {'ok': True}\n",
        "{}",
    );
    assert!(out.exit_ok);
    assert_eq!(out.result, Some(serde_json::json!({ "ok": true })));
}

#[test]
fn node_handler_calling_process_exit_leaves_no_sentinel_but_exits_clean() {
    if require(Lang::Node) {
        return;
    }
    let out = run_runner(
        Lang::Node,
        "export function handler() { process.exit(0); }",
        "{}",
    );
    assert!(out.exit_ok);
    assert_eq!(
        out.result, None,
        "the runner never reached its own final write"
    );
}

#[test]
fn python_handler_calling_os_exit_leaves_no_sentinel_but_exits_clean() {
    if require(Lang::Python) {
        return;
    }
    let out = run_runner(
        Lang::Python,
        "import os\n\n\ndef handler(p):\n    os._exit(0)\n",
        "{}",
    );
    assert!(out.exit_ok);
    assert_eq!(
        out.result, None,
        "the runner never reached its own final write"
    );
}

/// Regression for the round-2 finding: `sentinel` used to be bound at
/// MODULE scope in `run.py`, and Python always registers the running
/// script as `sys.modules['__main__']` — so a handler could read it
/// straight off that module by name and forge a winning frame before the
/// runner ever wrote its own. `sentinel` now lives inside `main()`, so the
/// same lookup must come back empty AND the handler's genuine return value
/// must come back untouched.
#[test]
fn python_handler_cannot_steal_the_sentinel_via_dunder_main() {
    if require(Lang::Python) {
        return;
    }
    let out = run_runner(
        Lang::Python,
        "import sys\n\n\ndef handler(p):\n    stolen = getattr(sys.modules['__main__'], 'sentinel', None)\n    return {'stolen': stolen, 'real': p['n']}\n",
        r#"{"n": 7}"#,
    );
    assert!(out.exit_ok);
    assert_eq!(
        out.result,
        Some(serde_json::json!({ "stolen": null, "real": 7 })),
        "sentinel must not be reachable off sys.modules['__main__'], and the \
         handler's real result must come back intact"
    );
}
