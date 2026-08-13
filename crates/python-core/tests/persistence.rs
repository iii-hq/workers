//! Park-and-loop interpreter persistence.
//!
//! Every containment test here names the mutation that makes it fail. A test
//! whose named mutation still passes is worthless and gets deleted — this
//! crate has deleted two already.

use std::sync::{Arc, OnceLock};

use iii_python_core::runner::{ExitKind, PersistentSpec, Runner};

fn boot() -> Arc<Runner> {
    static R: OnceLock<Arc<Runner>> = OnceLock::new();
    R.get_or_init(|| Runner::boot().expect("runner boots"))
        .clone()
}

/// A runtime plus the working directory it owns, kept alive together.
struct Live {
    rt: iii_python_core::runner::PersistentRuntime,
    _work: tempfile::TempDir,
}

async fn spawn(memory_mb: u64) -> Live {
    let work = tempfile::tempdir().unwrap();
    let rt = boot()
        .spawn_persistent(PersistentSpec {
            work_dir: work.path().to_path_buf(),
            memory_mb,
            bridge: None,
            namespace: None,
        })
        .await
        .unwrap_or_else(|e| panic!("spawn: {}", e.error));
    Live { rt, _work: work }
}

fn result_of(out: &iii_python_core::runner::RunOutcome) -> serde_json::Value {
    let bytes = out.envelope.as_ref().unwrap_or_else(|| {
        panic!(
            "no envelope; stderr={}",
            String::from_utf8_lossy(&out.stderr.bytes)
        )
    });
    serde_json::from_slice(bytes).unwrap()
}

/// The whole point: a name bound in one turn is still bound in the next.
#[tokio::test]
async fn globals_survive_between_turns() {
    let live = spawn(128).await;
    let first = live
        .rt
        .run("counter = 41".into(), None, 10_000)
        .await
        .unwrap();
    assert!(matches!(first.exit, ExitKind::Clean(0)), "{:?}", first.exit);

    let second = live
        .rt
        .run("counter += 1\nresult = counter".into(), None, 10_000)
        .await
        .unwrap();
    assert_eq!(result_of(&second)["result"], 42);
}

/// Imports are the expensive half of interpreter startup and the reason to
/// keep one warm at all.
#[tokio::test]
async fn imported_modules_survive_between_turns() {
    let live = spawn(128).await;
    live.rt
        .run("import json as _j".into(), None, 10_000)
        .await
        .unwrap();
    let out = live
        .rt
        .run("result = _j.dumps([1, 2])".into(), None, 10_000)
        .await
        .unwrap();
    assert_eq!(result_of(&out)["result"], "[1, 2]");
}

/// Files written to `/work` outlive the turn that wrote them — this is the
/// parity feature with sandbox-code-runner, which persists files and not
/// interpreters.
#[tokio::test]
async fn work_directory_survives_between_turns() {
    let live = spawn(128).await;
    live.rt
        .run(
            "open('/work/note.txt', 'w').write('hello')".into(),
            None,
            10_000,
        )
        .await
        .unwrap();
    let out = live
        .rt
        .run(
            "result = open('/work/note.txt').read()".into(),
            None,
            10_000,
        )
        .await
        .unwrap();
    assert_eq!(result_of(&out)["result"], "hello");
}

/// Each turn is budgeted on its own `timeout_ms`, not on whatever the runtime
/// was created with or the previous caller asked for.
///
/// **Mutation: cache the first turn's budget and reuse it** (arm the deadline
/// from a value captured at spawn rather than from `turn.timeout_ms`). The
/// second turn is then killed at 3 s despite asking for ten seconds.
///
/// The margins are load-tolerant on purpose: the first turn's budget (3 s)
/// must comfortably hold a trivial turn on a slow shared CI runner — 600 ms
/// did not — while staying under the second turn's sleep (4 s), which is what
/// makes a leaked first-turn budget observable.
///
/// Note on what is NOT claimed here: a turn has two independent kills — the
/// epoch deadline for executing wasm and the host-side `sleep_until` arm for a
/// guest parked in a host call — and they are armed to the same instant. Either
/// one alone produces `timed_out`, so no test through this API can show which
/// fired. That redundancy is deliberate; it is not something a test can pin.
#[tokio::test]
async fn each_turn_is_budgeted_on_its_own_timeout() {
    let live = spawn(128).await;
    let first = live.rt.run("result = 1".into(), None, 3_000).await.unwrap();
    assert!(!first.timed_out);

    let second = live
        .rt
        .run(
            "import time\ntime.sleep(4)\nresult = 2".into(),
            None,
            10_000,
        )
        .await
        .unwrap();
    assert!(
        !second.timed_out,
        "a 10s turn was killed on the previous turn's 3s budget"
    );
    assert_eq!(result_of(&second)["result"], 2);
}

/// **Mutation: drop the `denied.store(false, ...)` at turn start.** A
/// `MemoryError` the tenant caught and recovered from sets the limiter's flag,
/// and without the reset every later turn on this interpreter reports
/// `memory_denied` — classifying a perfectly good run as out-of-memory.
#[tokio::test]
async fn a_recovered_memory_error_does_not_poison_later_turns() {
    let live = spawn(64).await;
    let first = live
        .rt
        .run(
            "try:\n    b = bytearray(200 * 1024 * 1024)\nexcept MemoryError:\n    pass\nresult = 'survived'"
                .into(),
            None,
            20_000,
        )
        .await
        .unwrap();
    assert!(
        first.memory_denied,
        "the allocation should have been refused: {:?}",
        first.exit
    );

    let second = live
        .rt
        .run("result = 'clean'".into(), None, 10_000)
        .await
        .unwrap();
    assert!(
        !second.memory_denied,
        "the limiter's denied flag leaked across the turn boundary"
    );
    assert_eq!(result_of(&second)["result"], "clean");
}

/// **Mutation: drop the `wipe_dir(&out_dir)` at turn start.** The previous
/// turn's envelope is then still on disk when a turn that writes none — one
/// killed by its deadline — is read, and the caller is handed the *previous*
/// turn's result as this turn's.
#[tokio::test]
async fn a_turn_never_inherits_the_previous_turns_envelope() {
    let live = spawn(128).await;
    let first = live
        .rt
        .run("result = 'first'".into(), None, 10_000)
        .await
        .unwrap();
    assert_eq!(result_of(&first)["result"], "first");

    // A turn that cannot write an envelope: it is killed mid-spin.
    let second = live
        .rt
        .run("while True:\n    pass".into(), None, 700)
        .await
        .unwrap();
    assert!(second.timed_out, "{:?}", second.exit);
    assert!(
        second.envelope.is_none(),
        "read a stale envelope: {}",
        String::from_utf8_lossy(second.envelope.as_deref().unwrap_or(b""))
    );
}

/// A timed-out turn poisons the interpreter but keeps the directory.
///
/// The only kill that reaches a guest parked in a host call unwinds `_start`,
/// so there is no resumable state left; resuming at a tenant-chosen point
/// would hand half-written state to the next caller. The recovery is a fresh
/// interpreter on the same directory, which is why the directory must outlive
/// the interpreter that was using it.
///
/// **Mutation: let the task clean up `work_dir` on exit** (treat it as the
/// runtime's own scratch, the way `/run` and `/out` are). The respawned
/// interpreter then cannot read the file the first one wrote, and the tenant
/// silently loses everything a timed-out call had produced.
#[tokio::test]
async fn a_timed_out_turn_kills_the_interpreter_but_not_the_directory() {
    let work = tempfile::tempdir().unwrap();
    let rt = boot()
        .spawn_persistent(PersistentSpec {
            work_dir: work.path().to_path_buf(),
            memory_mb: 128,
            bridge: None,
            namespace: None,
        })
        .await
        .unwrap_or_else(|e| panic!("spawn: {}", e.error));

    rt.run(
        "open('/work/kept.txt','w').write('yes')".into(),
        None,
        10_000,
    )
    .await
    .unwrap();

    let killed = rt
        .run("while True:\n    pass".into(), None, 700)
        .await
        .unwrap();
    assert!(killed.timed_out);

    // The interpreter is gone...
    assert!(!rt.is_live(), "the interpreter survived its own timeout");
    assert!(
        rt.run("result = 1".into(), None, 5_000).await.is_err(),
        "a poisoned interpreter accepted another turn"
    );

    // ...but the directory it was working in is untouched, so a fresh
    // interpreter on the same directory picks up where the tenant left off.
    let fresh = boot()
        .spawn_persistent(PersistentSpec {
            work_dir: work.path().to_path_buf(),
            memory_mb: 128,
            bridge: None,
            namespace: None,
        })
        .await
        .unwrap_or_else(|e| panic!("respawn: {}", e.error));
    let out = fresh
        .run(
            "result = open('/work/kept.txt').read()".into(),
            None,
            10_000,
        )
        .await
        .unwrap();
    assert_eq!(result_of(&out)["result"], "yes");
}

/// A tenant exception ends its turn without ending the interpreter — a failing
/// script is a response, not an infrastructure failure, and the next caller on
/// this runtime must still be served.
#[tokio::test]
async fn a_tenant_exception_leaves_the_interpreter_usable() {
    let live = spawn(128).await;
    let boom = live
        .rt
        .run("raise ValueError('nope')".into(), None, 10_000)
        .await
        .unwrap();
    let env = result_of(&boom);
    assert_eq!(env["ok"], false);
    assert_eq!(env["kind"], "python_exception");
    assert!(env["message"].as_str().unwrap().contains("nope"));
    assert!(live.rt.is_live());

    let after = live
        .rt
        .run("result = 'still here'".into(), None, 10_000)
        .await
        .unwrap();
    assert_eq!(result_of(&after)["result"], "still here");
}

/// **Mutation: drop the `take_captured` reset** (go back to reading the pipe
/// without draining it). Turn two then reports turn one's output as its own.
#[tokio::test]
async fn each_turn_reports_only_its_own_output() {
    let live = spawn(128).await;
    let first = live
        .rt
        .run("print('from-one')".into(), None, 10_000)
        .await
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&first.stdout.bytes), "from-one\n");

    let second = live
        .rt
        .run("print('from-two')".into(), None, 10_000)
        .await
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&second.stdout.bytes), "from-two\n");
}

/// `sys.exit()` inside tenant code is the wrapper's documented "end this run,
/// keep whatever `result` held" — on a persistent interpreter it must not take
/// the interpreter down with it.
#[tokio::test]
async fn tenant_sys_exit_ends_the_turn_not_the_interpreter() {
    let live = spawn(128).await;
    let out = live
        .rt
        .run(
            "import sys\nresult = 'before'\nsys.exit()".into(),
            None,
            10_000,
        )
        .await
        .unwrap();
    assert_eq!(result_of(&out)["result"], "before");
    assert!(live.rt.is_live());
    let after = live.rt.run("result = 'after'".into(), None, 10_000).await;
    assert_eq!(result_of(&after.unwrap())["result"], "after");
}

/// The payload reaches the turn that asked for it, not a later one.
#[tokio::test]
async fn each_turn_gets_its_own_payload() {
    let live = spawn(128).await;
    let a = live
        .rt
        .run(
            "result = payload['n']".into(),
            Some(r#"{"n":1}"#.into()),
            10_000,
        )
        .await
        .unwrap();
    assert_eq!(result_of(&a)["result"], 1);
    let b = live
        .rt
        .run(
            "result = payload['n']".into(),
            Some(r#"{"n":2}"#.into()),
            10_000,
        )
        .await
        .unwrap();
    assert_eq!(result_of(&b)["result"], 2);
}

/// **Mutation: drop the `ns.pop("result", None)` in `run_once`.** The shared
/// namespace is what makes globals survive, and `result` lives in it — so a
/// turn that sets no result would return the *previous* turn's value, and the
/// caller would read a stale answer as their own.
#[tokio::test]
async fn a_turn_that_sets_no_result_returns_null_not_the_last_one() {
    let live = spawn(128).await;
    let first = live
        .rt
        .run("result = 'mine'".into(), None, 10_000)
        .await
        .unwrap();
    assert_eq!(result_of(&first)["result"], "mine");

    let second = live
        .rt
        .run("x = 1  # sets no result".into(), None, 10_000)
        .await
        .unwrap();
    assert_eq!(
        result_of(&second)["result"],
        serde_json::Value::Null,
        "inherited the previous turn's result"
    );
}

/// A non-ASCII byte anywhere in a turn frame must not desynchronise the guest.
///
/// **Mutation: read `sys.stdin` instead of `sys.stdin.buffer`** in
/// `_read_frame`. The host's length prefix counts BYTES; text-mode `read(n)`
/// counts CHARACTERS and so does `len()`, so the guest asks for bytes the host
/// has already finished sending and blocks until its deadline. Every character
/// below is multi-byte in UTF-8, in the code and in the payload.
#[tokio::test]
async fn a_turn_frame_carrying_non_ascii_is_framed_by_bytes() {
    let live = spawn(128).await;
    let out = live
        .rt
        .run(
            "# — em dash, ± sign, 日本語\nresult = payload['greeting'] + ' — ok'".into(),
            Some(r#"{"greeting":"こんにちは"}"#.into()),
            10_000,
        )
        .await
        .unwrap();
    assert!(!out.timed_out, "the guest desynchronised: {:?}", out.exit);
    assert_eq!(result_of(&out)["result"], "こんにちは — ok");

    // Still in sync for the next turn.
    let after = live
        .rt
        .run("result = 'after'".into(), None, 10_000)
        .await
        .unwrap();
    assert_eq!(result_of(&after)["result"], "after");
}
