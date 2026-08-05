//! The guest scripts under the REAL `node` and `python3` — the same
//! binaries the sandbox images ship. Goldens pin the bytes; this proves they
//! work: protocol framing, envelope delivery, error shape, exit codes, and
//! the `iii` global (its file-RPC channel served here by an in-test broker
//! thread standing in for `broker.rs`).
//!
//! FAILS LOUDLY by default when an interpreter is missing: these are the
//! only tests that prove the runner scripts actually work, so a silent skip
//! would let a broken PATH report as a clean, green suite. Opt out
//! explicitly with `ALLOW_MISSING_INTERPRETERS=1` if you knowingly don't
//! have one of the two interpreters installed.

mod support;

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use sandbox_code_runner::runner::{split_sentinel, Lang};

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

/// The final path segment of a `Lang` accessor's path — deriving a guest
/// file's on-disk name from `runner_path()` / `iii_lib_path()` /
/// `run_wrapper_path()` instead of hardcoding it, so this harness can't
/// drift from the plant table the way a hardcoded copy silently could.
fn basename(path: &str) -> &str {
    path.rsplit_once('/').unwrap().1
}

/// Write the runner (plus its sibling `iii` library, which it now imports
/// unconditionally) + a handler source into `dir` (caller-owned, so a fake
/// SDK can be planted beside them first), run `<interpreter> <runner>
/// <source>` with `stdin` fed to the child verbatim (no envelope wrapping —
/// used directly by the malformed-envelope tests) and `envs` in its
/// environment. Bounded: a runner kept alive by a leaked handle fails the
/// test instead of hanging the suite.
fn spawn_runner_env(
    lang: Lang,
    handler_src: &str,
    stdin: &str,
    envs: &[(&str, &str)],
    dir: &std::path::Path,
) -> RunOutcome {
    std::fs::create_dir_all(dir).unwrap();
    let runner = dir.join(basename(lang.runner_path()));
    let source = dir.join(format!("handler.{}", lang.ext()));
    std::fs::write(&runner, lang.runner_source()).unwrap();
    let lib_name = basename(lang.iii_lib_path());
    std::fs::write(dir.join(lib_name), lang.iii_lib_source()).unwrap();
    std::fs::write(&source, handler_src).unwrap();

    // Pin the interpreter's colour behaviour instead of inheriting the
    // developer's. Node >= 26 formats numbers with ANSI colour even when
    // stdout is a pipe if `FORCE_COLOR` is set — and it is, in at least one
    // shell here — so `console.log('working on', 21)` reaches us as
    // `working on \e[33m21\e[39m` and a plain `contains` assertion fails.
    // That is a property of the terminal, not of the runner under test.
    let mut cmd = Command::new(lang.interpreter());
    cmd.arg(&runner)
        .arg(&source)
        .env_remove("FORCE_COLOR")
        .env("NO_COLOR", "1");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd
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
    let out = wait_bounded(child, 15);

    let split = split_sentinel(&out.stdout, SENTINEL);
    RunOutcome {
        exit_ok: out.exit_ok,
        logs: split.logs,
        result: split
            .result
            .as_deref()
            .and_then(|r| serde_json::from_str(r).ok()),
        stderr: out.stderr,
    }
}

fn spawn_runner(lang: Lang, handler_src: &str, stdin: &str) -> RunOutcome {
    let dir = scratch("runner");
    let out = spawn_runner_env(lang, handler_src, stdin, &[], &dir);
    std::fs::remove_dir_all(&dir).ok();
    out
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
fn goldens_pin_every_guest_script() {
    let mut failures = Vec::new();
    for (name, contents) in [
        (
            basename(Lang::Node.runner_path()),
            Lang::Node.runner_source(),
        ),
        (
            basename(Lang::Python.runner_path()),
            Lang::Python.runner_source(),
        ),
        (
            basename(Lang::Node.iii_lib_path()),
            Lang::Node.iii_lib_source(),
        ),
        (
            basename(Lang::Python.iii_lib_path()),
            Lang::Python.iii_lib_source(),
        ),
        (
            basename(Lang::Node.run_wrapper_path()),
            Lang::Node.run_wrapper_source(),
        ),
        (
            basename(Lang::Python.run_wrapper_path()),
            Lang::Python.run_wrapper_source(),
        ),
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

// ---------------------------------------------------------------------
// The `iii` global. Hermetic: a FAKE iii-sdk package is planted where the
// real one would be (scratch node_modules for Node, the script directory —
// sys.path[0] — for Python), recording registerWorker/shutdown calls to a
// log file the tests read back. The REAL SDK's round trip is the e2e
// integration test's job; these prove the wrappers' contract with any SDK:
// lazy connect, worker-name passthrough, shutdown-after-run, and clear
// errors when the SDK or III_URL is missing.
// ---------------------------------------------------------------------

use std::path::{Path, PathBuf};

const FAKE_SDK_MJS: &str = r#"import { appendFileSync } from 'node:fs';
const log = (entry) => appendFileSync(process.env.FAKE_SDK_LOG, JSON.stringify(entry) + '\n');
export function registerWorker(url, options) {
  log({ event: 'register', url, workerName: options && options.workerName });
  return {
    async trigger(req) { return { echoed: req }; },
    async shutdown() { log({ event: 'shutdown' }); },
  };
}
"#;

const FAKE_SDK_PY: &str = r#"import json
import os


class InitOptions:
    def __init__(self, worker_name=None, **kwargs):
        self.worker_name = worker_name


def _log(entry):
    with open(os.environ["FAKE_SDK_LOG"], "a", encoding="utf-8") as f:
        f.write(json.dumps(entry) + "\n")


class _Client:
    def trigger(self, req):
        return {"echoed": req}

    def shutdown(self):
        _log({"event": "shutdown"})


def register_worker(url, options=None):
    _log({"event": "register", "url": url,
          "worker_name": getattr(options, "worker_name", None)})
    return _Client()
"#;

fn scratch(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ce-iii-{label}-{}", uuid::Uuid::new_v4()))
}

/// Plant the fake SDK where the guest scripts in `dir` will resolve the
/// real one: `node_modules/iii-sdk` for Node, an `iii/` package in the
/// directory itself (= sys.path[0] of a script run from there) for Python.
fn plant_fake_sdk(lang: Lang, dir: &Path) {
    match lang {
        Lang::Node => {
            let pkg = dir.join("node_modules/iii-sdk");
            std::fs::create_dir_all(&pkg).unwrap();
            std::fs::write(
                pkg.join("package.json"),
                r#"{"name":"iii-sdk","type":"module","main":"index.mjs","exports":{".":"./index.mjs"}}"#,
            )
            .unwrap();
            std::fs::write(pkg.join("index.mjs"), FAKE_SDK_MJS).unwrap();
        }
        Lang::Python => {
            let pkg = dir.join("iii");
            std::fs::create_dir_all(&pkg).unwrap();
            std::fs::write(pkg.join("__init__.py"), FAKE_SDK_PY).unwrap();
        }
    }
}

/// What the fake SDK recorded, parsed back from the log file.
fn sdk_log(path: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).expect("log line is JSON"))
        .collect()
}

struct WrapperOutcome {
    exit_ok: bool,
    stdout: String,
    stderr: String,
}

/// Run `child` to completion with a hard cap: readers drain stdout/stderr
/// on threads (a full pipe would block the child and `try_wait` forever),
/// and a child still alive at the deadline is killed and FAILS the test —
/// a wrapper whose SDK connection kept the process alive must never
/// surface as a silently hung suite.
fn wait_bounded(mut child: std::process::Child, secs: u64) -> WrapperOutcome {
    let drain = |stream: Option<Box<dyn std::io::Read + Send>>| {
        let mut stream = stream;
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(s) = stream.as_mut() {
                use std::io::Read as _;
                let _ = s.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out_t = drain(
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
    );
    let err_t = drain(
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
    );

    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    let status = loop {
        match child.try_wait().unwrap() {
            Some(status) => break status,
            None if std::time::Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("guest process still alive after {secs}s — a leaked handle (SDK socket?) is keeping it up");
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    WrapperOutcome {
        exit_ok: status.success(),
        stdout: String::from_utf8_lossy(&out_t.join().unwrap()).to_string(),
        stderr: String::from_utf8_lossy(&err_t.join().unwrap()).to_string(),
    }
}

/// Plant the run wrapper + iii library + the code file into `dir` and run
/// `<interpreter> run.<ext> code.<ext>` with the given env — exactly the
/// argv/env shape `run` execs inside the VM.
fn run_eval_wrapper(lang: Lang, code: &str, envs: &[(&str, &str)], dir: &Path) -> WrapperOutcome {
    std::fs::create_dir_all(dir).unwrap();
    let wrapper = dir.join(basename(lang.run_wrapper_path()));
    std::fs::write(&wrapper, lang.run_wrapper_source()).unwrap();
    let lib_name = basename(lang.iii_lib_path());
    std::fs::write(dir.join(lib_name), lang.iii_lib_source()).unwrap();
    let code_file = dir.join(format!("code.{}", lang.ext()));
    std::fs::write(&code_file, code).unwrap();

    let mut cmd = Command::new(lang.interpreter());
    cmd.arg(&wrapper)
        .arg(&code_file)
        .env_remove("FORCE_COLOR")
        .env("NO_COLOR", "1");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_bounded(child, 15)
}

/// A run that never touches `iii` must not connect (the log stays empty) —
/// the lazy global is what keeps plain evals from paying an SDK handshake.
#[test]
fn node_eval_wrapper_is_lazy_and_connects_only_on_use() {
    if require(Lang::Node) {
        return;
    }
    let dir = scratch("node-lazy");
    std::fs::create_dir_all(&dir).unwrap();
    plant_fake_sdk(Lang::Node, &dir);
    let log = dir.join("sdk.log");
    let envs: &[(&str, &str)] = &[
        ("III_URL", "ws://localhost:49134"),
        ("III_WORKER_NAME", "sandbox-code-runner:run"),
        ("FAKE_SDK_LOG", log.to_str().unwrap()),
    ];

    let out = run_eval_wrapper(Lang::Node, "console.log('plain');\n", envs, &dir);
    assert!(out.exit_ok, "stderr: {}", out.stderr);
    assert!(out.stdout.contains("plain"), "stdout: {}", out.stdout);
    assert!(
        sdk_log(&log).is_empty(),
        "an eval that never used iii must not have connected"
    );

    let out = run_eval_wrapper(
        Lang::Node,
        "const r = await iii.trigger({ function_id: 'x::y', payload: { n: 1 } });\n\
         console.log(JSON.stringify(r));\n",
        envs,
        &dir,
    );
    assert!(out.exit_ok, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains(r#"{"echoed""#),
        "stdout: {}",
        out.stdout
    );
    let log_entries = sdk_log(&log);
    assert_eq!(log_entries.len(), 2, "{log_entries:?}");
    assert_eq!(log_entries[0]["event"], "register");
    assert_eq!(log_entries[0]["url"], "ws://localhost:49134");
    assert_eq!(
        log_entries[0]["workerName"], "sandbox-code-runner:run",
        "III_WORKER_NAME must reach the SDK"
    );
    assert_eq!(
        log_entries[1]["event"], "shutdown",
        "the wrapper must shut a used client down after the run"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn python_eval_wrapper_is_lazy_and_connects_only_on_use() {
    if require(Lang::Python) {
        return;
    }
    let dir = scratch("py-lazy");
    std::fs::create_dir_all(&dir).unwrap();
    plant_fake_sdk(Lang::Python, &dir);
    let log = dir.join("sdk.log");
    let envs: &[(&str, &str)] = &[
        ("III_URL", "ws://localhost:49134"),
        ("III_WORKER_NAME", "sandbox-code-runner:run"),
        ("FAKE_SDK_LOG", log.to_str().unwrap()),
    ];

    let out = run_eval_wrapper(Lang::Python, "print('plain')\n", envs, &dir);
    assert!(out.exit_ok, "stderr: {}", out.stderr);
    assert!(
        sdk_log(&log).is_empty(),
        "an eval that never used iii must not have connected"
    );

    let out = run_eval_wrapper(
        Lang::Python,
        "import json\n\
         r = iii.trigger({'function_id': 'x::y', 'payload': {'n': 1}})\n\
         print(json.dumps(r))\n",
        envs,
        &dir,
    );
    assert!(out.exit_ok, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains(r#"{"echoed""#),
        "stdout: {}",
        out.stdout
    );
    let log_entries = sdk_log(&log);
    assert_eq!(log_entries.len(), 2, "{log_entries:?}");
    assert_eq!(log_entries[0]["event"], "register");
    assert_eq!(log_entries[0]["worker_name"], "sandbox-code-runner:run");
    assert_eq!(log_entries[1]["event"], "shutdown");
    std::fs::remove_dir_all(&dir).ok();
}

/// Probing the global — printing it, listing keys, reading its prototype —
/// must answer something useful and must NEVER connect. (A live session
/// spent six blind evals against an opaque `{}` whose prototype was null;
/// see console-a2795be8.) After first use, `Object.keys(iii)` lists the
/// client's callable surface.
#[test]
fn node_iii_is_introspectable_without_connecting() {
    if require(Lang::Node) {
        return;
    }
    let dir = scratch("node-introspect");
    std::fs::create_dir_all(&dir).unwrap();
    plant_fake_sdk(Lang::Node, &dir);
    let log = dir.join("sdk.log");
    let envs: &[(&str, &str)] = &[
        ("III_URL", "ws://localhost:49134"),
        ("FAKE_SDK_LOG", log.to_str().unwrap()),
    ];

    let out = run_eval_wrapper(
        Lang::Node,
        "console.log('hint:', String(iii));\n\
         console.log('inspect:', iii);\n\
         console.log('keys-before:', JSON.stringify(Object.keys(iii)));\n\
         console.log('proto-ok:', Object.getPrototypeOf(iii) !== null);\n",
        envs,
        &dir,
    );
    assert!(out.exit_ok, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("lazy iii-sdk client"),
        "String(iii) and console.log(iii) must explain themselves: {}",
        out.stdout
    );
    assert!(out.stdout.contains("keys-before: []"), "{}", out.stdout);
    assert!(out.stdout.contains("proto-ok: true"), "{}", out.stdout);
    assert!(
        sdk_log(&log).is_empty(),
        "introspection must never connect: {:?}",
        sdk_log(&log)
    );

    let out = run_eval_wrapper(
        Lang::Node,
        "await iii.trigger({ function_id: 'x::y', payload: {} });\n\
         console.log('keys-after:', JSON.stringify(Object.keys(iii).sort()));\n\
         console.log('has-after:', 'trigger' in iii);\n",
        envs,
        &dir,
    );
    assert!(out.exit_ok, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains(r#"keys-after: ["shutdown","trigger"]"#),
        "after first use, keys must list the client surface: {}",
        out.stdout
    );
    assert!(out.stdout.contains("has-after: true"), "{}", out.stdout);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn python_iii_is_introspectable_without_connecting() {
    if require(Lang::Python) {
        return;
    }
    let dir = scratch("py-introspect");
    std::fs::create_dir_all(&dir).unwrap();
    plant_fake_sdk(Lang::Python, &dir);
    let log = dir.join("sdk.log");
    let envs: &[(&str, &str)] = &[
        ("III_URL", "ws://localhost:49134"),
        ("FAKE_SDK_LOG", log.to_str().unwrap()),
    ];

    let out = run_eval_wrapper(
        Lang::Python,
        "print('hint:', repr(iii))\n\
         print('dir-before:', dir(iii))\n",
        envs,
        &dir,
    );
    assert!(out.exit_ok, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("lazy iii-sdk client"),
        "repr(iii) must explain itself: {}",
        out.stdout
    );
    assert!(out.stdout.contains("dir-before: []"), "{}", out.stdout);
    assert!(
        sdk_log(&log).is_empty(),
        "introspection must never connect: {:?}",
        sdk_log(&log)
    );

    let out = run_eval_wrapper(
        Lang::Python,
        "iii.trigger({'function_id': 'x::y', 'payload': {}})\n\
         print('has-trigger:', 'trigger' in dir(iii))\n",
        envs,
        &dir,
    );
    assert!(out.exit_ok, "stderr: {}", out.stderr);
    assert!(out.stdout.contains("has-trigger: True"), "{}", out.stdout);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn node_iii_without_a_url_fails_fast_with_a_clear_error() {
    if require(Lang::Node) {
        return;
    }
    let dir = scratch("node-nourl");
    std::fs::create_dir_all(&dir).unwrap();
    plant_fake_sdk(Lang::Node, &dir);
    let log = dir.join("sdk.log");
    let out = run_eval_wrapper(
        Lang::Node,
        "try { await iii.trigger({ function_id: 'x::y', payload: {} }); } \
         catch (e) { console.log('caught:', e.message); }\n",
        &[("FAKE_SDK_LOG", log.to_str().unwrap())],
        &dir,
    );
    assert!(out.exit_ok, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("III_URL is not set"),
        "stdout: {}",
        out.stdout
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn node_iii_without_the_sdk_reports_it_clearly() {
    if require(Lang::Node) {
        return;
    }
    let dir = scratch("node-nosdk");
    std::fs::create_dir_all(&dir).unwrap();
    // No fake SDK planted; the bare import must fail and the error must say so.
    let out = run_eval_wrapper(
        Lang::Node,
        "try { await iii.trigger({ function_id: 'x::y', payload: {} }); } \
         catch (e) { console.log('caught:', e.message); }\n",
        &[("III_URL", "ws://localhost:49134")],
        &dir,
    );
    assert!(out.exit_ok, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("could not be loaded"),
        "stdout: {}",
        out.stdout
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn python_iii_without_the_sdk_reports_it_clearly() {
    if require(Lang::Python) {
        return;
    }
    let dir = scratch("py-nosdk");
    std::fs::create_dir_all(&dir).unwrap();
    let out = run_eval_wrapper(
        Lang::Python,
        "try:\n    iii.trigger({'function_id': 'x::y', 'payload': {}})\n\
         except RuntimeError as e:\n    print('caught:', e)\n",
        &[("III_URL", "ws://localhost:49134")],
        &dir,
    );
    assert!(out.exit_ok, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("not installed in this runtime"),
        "stdout: {}",
        out.stdout
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Handlers get the same lazy global THROUGH THE RUNNER: a handler that
/// calls `iii.trigger` mid-invocation gets its answer, the client is shut
/// down before the frame, and the runner protocol (sentinel framing,
/// result envelope) stays intact.
#[test]
fn node_handlers_get_iii_through_the_runner() {
    if require(Lang::Node) {
        return;
    }
    let dir = scratch("node-runner-sdk");
    std::fs::create_dir_all(&dir).unwrap();
    plant_fake_sdk(Lang::Node, &dir);
    let log = dir.join("sdk.log");
    let envelope = serde_json::json!({ "sentinel": SENTINEL, "payload": { "n": 1 } }).to_string();
    let out = spawn_runner_env(
        Lang::Node,
        "export async function handler(p) { return await iii.trigger({ function_id: 'a::b', payload: p }); }",
        &envelope,
        &[
            ("III_URL", "ws://localhost:49134"),
            ("FAKE_SDK_LOG", log.to_str().unwrap()),
        ],
        &dir,
    );
    assert!(out.exit_ok, "stderr: {}", out.stderr);
    assert_eq!(
        out.result,
        Some(serde_json::json!({ "echoed": { "function_id": "a::b", "payload": { "n": 1 } } }))
    );
    let log_entries = sdk_log(&log);
    assert_eq!(log_entries.len(), 2, "register + shutdown: {log_entries:?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn python_handlers_get_iii_through_the_runner() {
    if require(Lang::Python) {
        return;
    }
    let dir = scratch("py-runner-sdk");
    std::fs::create_dir_all(&dir).unwrap();
    plant_fake_sdk(Lang::Python, &dir);
    let log = dir.join("sdk.log");
    let envelope = serde_json::json!({ "sentinel": SENTINEL, "payload": { "n": 1 } }).to_string();
    let out = spawn_runner_env(
        Lang::Python,
        "def handler(p):\n    return iii.trigger({'function_id': 'a::b', 'payload': p})\n",
        &envelope,
        &[
            ("III_URL", "ws://localhost:49134"),
            ("FAKE_SDK_LOG", log.to_str().unwrap()),
        ],
        &dir,
    );
    assert!(out.exit_ok, "stderr: {}", out.stderr);
    assert_eq!(
        out.result,
        Some(serde_json::json!({ "echoed": { "function_id": "a::b", "payload": { "n": 1 } } }))
    );
    assert_eq!(sdk_log(&log).len(), 2, "register + shutdown");
    std::fs::remove_dir_all(&dir).ok();
}

/// Regression for the round-2 finding: `sentinel` used to be bound at
/// MODULE scope in `invoke.py`, and Python always registers the running
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
