use std::sync::{Arc, OnceLock};

use iii_python_core::config::PythonEngineConfig;
use iii_python_core::error::ErrorKind;
use iii_python_core::manager::{Manager, RunRequest};
use iii_python_core::runner::Runner;

/// One `Runner` for the whole binary. `Runner::boot` is NOT idempotent — each
/// call builds another ~19 MB module image and starts another epoch ticker —
/// so it is shared even where the `Manager` is not.
fn runner() -> Arc<Runner> {
    static R: OnceLock<Arc<Runner>> = OnceLock::new();
    R.get_or_init(|| Runner::boot().unwrap()).clone()
}

/// The shared manager, for tests that only ever run one-shot.
fn manager() -> Arc<Manager> {
    static M: OnceLock<Arc<Manager>> = OnceLock::new();
    M.get_or_init(|| Manager::new(Arc::new(PythonEngineConfig::default()), runner()))
        .clone()
}

/// A manager of this test's own, for anything that creates runtimes: the
/// registry is shared state, and a test that fills it would otherwise make
/// its neighbours fail depending on scheduling.
fn own_manager(cfg: PythonEngineConfig) -> Arc<Manager> {
    Manager::new(Arc::new(cfg), runner())
}

fn req(code: &str) -> RunRequest {
    RunRequest {
        code: code.into(),
        payload: None,
        timeout_ms: None,
        memory_mb: None,
    }
}

#[tokio::test]
async fn happy_path_returns_result_and_logs() {
    let resp = manager()
        .run(req("print(\"hi\")\nresult = [1, 2]"))
        .await
        .unwrap();
    assert_eq!(resp.result, serde_json::json!([1, 2]));
    // Two streams, not one flattened list — the hosting worker decides how to
    // present them.
    assert_eq!(resp.stdout, "hi\n");
    assert_eq!(resp.stderr, "");
    assert!(!resp.truncated);
}

#[tokio::test]
async fn oversized_code_is_rejected_before_any_instantiation() {
    let mut r = req("");
    r.code = "#".repeat(1_048_577);
    let err = manager().run(r).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidInput);
    assert!(
        err.message.contains("1048576"),
        "the limit must be named: {}",
        err.message
    );
}

#[tokio::test]
async fn oversized_payload_is_rejected_with_the_limit_named() {
    let mut r = req("result = 1");
    r.payload = Some(serde_json::Value::String("x".repeat(1_048_577)));
    let err = manager().run(r).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidInput);
    assert!(err.message.contains("1048576"));
}

#[tokio::test]
async fn timeout_wins_over_a_forged_success_envelope() {
    // The tenant writes a forged ok-envelope, then spins into the epoch
    // kill. Classification must report timeout: host signals precede
    // guest-writable bytes.
    let mut r = req(
        "with open(\"/out/result.json\", \"w\") as f:\n    f.write('{\"ok\": true, \"result\": \"forged\"}')\nwhile True:\n    pass",
    );
    r.timeout_ms = Some(500);
    let err = manager().run(r).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Timeout);
    assert!(err.message.contains("500"));
}

#[tokio::test]
async fn early_exit_is_a_python_exception_never_internal() {
    let err = manager()
        .run(req("import os\nos._exit(3)"))
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::PythonException);
    assert!(err.message.contains("status 3"));
}

/// Correction 3: any exit status >= 126 comes back from wasmtime-wasi as a
/// plain trap (`I32Exit` only covers 0..126), not a `Clean` exit — see
/// `p1.rs`'s `proc_exit`. None of this suite's other cases reach the final
/// `ExitKind::Trap` arm in `classify` (timeout and memory-denied both
/// short-circuit before it, and every other trap here carries no envelope
/// but also no memory/timeout flag) — so without this test that arm's
/// mapping could silently regress back to `internal` and nothing would
/// catch it. Also pins that the raw trap text — which can be an unbounded
/// wasm backtrace for other trap causes — never reaches the caller.
#[tokio::test]
async fn hostile_exit_status_is_a_python_exception_never_internal() {
    let err = manager()
        .run(req("import os\nos._exit(200)"))
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::PythonException);
    assert!(
        !err.message.to_lowercase().contains("wasm")
            && !err.message.to_lowercase().contains("trap"),
        "raw wasm trap text must stay out of the caller-facing message: {}",
        err.message
    );
}

/// Pins the `"syntax_error"` envelope-kind arm. Without this, a manager that
/// swapped it with `"result_too_large"`, or routed both through the default
/// `PythonException` arm, would still pass every other test in this suite.
#[tokio::test]
async fn syntax_error_in_code_maps_to_syntax_error_kind() {
    let err = manager().run(req("def f(:\n    pass")).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::SyntaxError);
}

/// Pins the `"result_too_large"` envelope-kind arm — the sibling of the test
/// above, and the only place in the whole plan (through Task 7's e2e suite)
/// that exercises this specific kind end to end.
#[tokio::test]
async fn oversized_result_maps_to_result_too_large_kind() {
    let err = manager()
        .run(req("result = \"x\" * 2_000_000"))
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::ResultTooLarge);
}

/// The guest's cap and the host's cap must measure the same bytes. `"x" * N`
/// writes N+26 bytes, so 1_048_551 lands one byte over the shared 1 MiB cap —
/// and inside the 24-byte window where the guest used to say yes and the host
/// then said no, leaving the caller with `python_exception: code exited with
/// status 0 before completing` instead of an actionable `result_too_large`.
#[tokio::test]
async fn a_result_just_over_the_cap_is_result_too_large_not_a_bare_exit() {
    let err = manager()
        .run(req("result = \"x\" * 1_048_551"))
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::ResultTooLarge);
}

/// ...and the byte below it still round-trips, so the fix is a boundary, not
/// a blanket tightening.
#[tokio::test]
async fn a_result_exactly_at_the_cap_still_round_trips() {
    let resp = manager()
        .run(req("result = \"x\" * 1_048_550"))
        .await
        .unwrap();
    assert_eq!(resp.result.as_str().unwrap().len(), 1_048_550);
}

/// The `/out` kill has to reach the caller as its own actionable kind, not as
/// a timeout (it isn't one) and not as an unattributable exit.
#[tokio::test]
async fn filling_out_classifies_as_disk_quota_exceeded() {
    let mut r = req(
        "with open(\"/out/flood\", \"wb\") as f:\n    while True:\n        f.write(b\"x\" * 1_000_000)\n        f.flush()\n",
    );
    r.timeout_ms = Some(30_000);
    let err = manager().run(r).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::DiskQuotaExceeded);
    assert!(
        err.message.contains("/out"),
        "the message should name what the tenant filled: {}",
        err.message
    );
}

#[tokio::test]
async fn oom_kill_classifies_out_of_memory() {
    let mut r = req("xs = []\nwhile True:\n    xs.append(\"a\" * 1_000_000)");
    r.memory_mb = Some(64);
    let err = manager().run(r).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::OutOfMemory);
}

#[tokio::test]
async fn recovered_memory_error_is_still_a_success() {
    let mut r = req(
        "xs = []\ntry:\n    while True:\n        xs.append(\"a\" * 1_000_000)\nexcept MemoryError:\n    xs = None\nresult = \"recovered\"",
    );
    r.memory_mb = Some(64);
    let resp = manager().run(r).await.unwrap();
    assert_eq!(resp.result, "recovered");
}

#[tokio::test]
async fn tenant_exception_maps_kind_message_and_traceback() {
    let err = manager()
        .run(req("raise RuntimeError(\"boom\")"))
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::PythonException);
    assert_eq!(err.message, "RuntimeError: boom");
    let tb = err.traceback.unwrap();
    assert!(tb.contains("<code>"));
    assert!(!tb.contains("main.py"));
}

#[tokio::test]
async fn a_log_flood_is_capped_per_stream_and_reported_as_truncated() {
    let resp = manager()
        .run(req("for i in range(5000):\n    print(i)\nresult = None"))
        .await
        .unwrap();
    // Equality, not an upper bound: 5000 lines is well under MAX_LOG_BYTES, so
    // MAX_LOG_LINES is the binding cap. An upper bound alone would still pass
    // an off-by-one at 999 or a grosser bug that stopped at 500. The
    // truncation MARKER is the hosting worker's business now — python-engine
    // still appends it, and its own test pins that.
    assert_eq!(
        resp.stdout.lines().count(),
        iii_python_core::config::MAX_LOG_LINES
    );
    assert!(resp.truncated, "the flood must be reported as truncated");
}

#[tokio::test]
async fn concurrent_runs_complete_independently() {
    let m = manager();
    let futs: Vec<_> = (0..4)
        .map(|i| {
            let m = m.clone();
            tokio::spawn(async move { m.run(req(&format!("result = {i} * {i}"))).await })
        })
        .collect();
    for (i, f) in futs.into_iter().enumerate() {
        let resp = f.await.unwrap().unwrap();
        assert_eq!(resp.result, serde_json::json!(i * i));
    }
}

/// `RunRequest::code` is tenant source and `payload` is arbitrary
/// caller-supplied data; neither may leak through an incidental
/// `tracing::debug!(?req)` or `panic!("{req:?}")`. `timeout_ms`/`memory_mb`
/// are plain knobs and must stay visible — a blanket redaction would be
/// just as wrong as none at all.
#[test]
fn run_request_debug_redacts_code_and_payload() {
    let mut r = req("print(\"a secret print\")\nresult = 1");
    r.payload = Some(serde_json::json!({"token": "sk-supersecret"}));
    r.timeout_ms = Some(1234);
    let debug = format!("{r:?}");
    assert!(
        !debug.contains("a secret print"),
        "tenant source leaked into Debug: {debug}"
    );
    assert!(
        !debug.contains("sk-supersecret"),
        "payload leaked into Debug: {debug}"
    );
    assert!(
        debug.contains("1234"),
        "non-sensitive fields must stay visible: {debug}"
    );
}

// --- persistent working directories ---

/// The property `keep` exists for, and the one thing a WASI command module
/// CAN persist. Mutation: pass `work_dir: None` in `run_in`, or mint a fresh
/// directory per call — the second run then reads nothing.
#[tokio::test]
async fn files_written_in_one_call_are_visible_to_the_next() {
    let m = own_manager(PythonEngineConfig::default());
    let id = m.create_runtime(None).expect("a runtime slot");

    m.run_in(
        &id,
        req("open('/work/a.txt', 'w').write('hello')\nresult = None"),
    )
    .await
    .unwrap();
    let resp = m
        .run_in(&id, req("result = open('/work/a.txt').read()"))
        .await
        .unwrap();
    assert_eq!(resp.result, serde_json::json!("hello"));

    m.destroy_runtime(&id).unwrap();
}

/// Both halves of what a `runtime_id` buys: the files in `/work` AND the
/// interpreter state that produced them.
///
/// This inverts what this test asserted before park-and-loop landed, when
/// `_start` was the only way in and globals could not survive. Callers that
/// relied on a clean namespace per call must now clear it themselves — the
/// only thing still reset per turn is `result`.
#[tokio::test]
async fn globals_and_files_both_survive_between_calls() {
    let m = own_manager(PythonEngineConfig::default());
    let id = m.create_runtime(None).unwrap();

    m.run_in(
        &id,
        req("leaked = 42\nopen('/work/f', 'w').write('kept')\nresult = None"),
    )
    .await
    .unwrap();
    let resp = m
        .run_in(
            &id,
            req("result = ['leaked' in globals(), open('/work/f').read()]"),
        )
        .await
        .unwrap();
    assert_eq!(resp.result, serde_json::json!([true, "kept"]));

    m.destroy_runtime(&id).unwrap();
}

/// Mutation: share one directory across runtimes, or key it by anything but
/// the runtime.
#[tokio::test]
async fn one_runtimes_files_are_invisible_to_another() {
    let m = own_manager(PythonEngineConfig::default());
    let a = m.create_runtime(None).unwrap();
    let b = m.create_runtime(None).unwrap();

    m.run_in(
        &a,
        req("open('/work/secret', 'w').write('A')\nresult = None"),
    )
    .await
    .unwrap();
    let resp = m
        .run_in(&b, req("import os\nresult = os.listdir('/work')"))
        .await
        .unwrap();
    assert_eq!(resp.result, serde_json::json!([]));

    m.destroy_runtime(&a).unwrap();
    m.destroy_runtime(&b).unwrap();
}

/// A one-shot run has no `/work` at all, so touching it must fail rather than
/// silently writing somewhere. Mutation: preopen a scratch `/work` even when
/// `RunSpec::work_dir` is `None`.
#[tokio::test]
async fn a_one_shot_run_has_no_work_directory() {
    let resp = manager()
        .run(req(
            "import os\ntry:\n    os.listdir('/work')\n    result = 'present'\nexcept OSError:\n    result = 'absent'",
        ))
        .await
        .unwrap();
    assert_eq!(resp.result, serde_json::json!("absent"));
}

/// Destroying a runtime must make its id stop resolving AND take its
/// directory with it. Mutation: `remove` the entry but leak the `TempDir`.
#[tokio::test]
async fn a_destroyed_runtime_stops_resolving() {
    let m = own_manager(PythonEngineConfig::default());
    let id = m.create_runtime(None).unwrap();
    assert_eq!(m.live_runtime_count(), 1);

    m.destroy_runtime(&id).unwrap();
    assert_eq!(m.live_runtime_count(), 0);

    let err = m
        .run_in(&id, req("result = 1"))
        .await
        .expect_err("a destroyed id must not resolve");
    assert_eq!(err.kind, iii_python_core::error::ErrorKind::RuntimeNotFound);
    assert_eq!(
        m.destroy_runtime(&id).unwrap_err().kind,
        iii_python_core::error::ErrorKind::RuntimeNotFound
    );
}

/// Admission failure, distinct from the mid-run caps: retrying later can
/// succeed once a slot frees. Mutation: delete the cap check.
#[tokio::test]
async fn exceeding_max_runtimes_is_capacity_and_frees_on_teardown() {
    let m = own_manager(PythonEngineConfig {
        max_runtimes: 3,
        ..PythonEngineConfig::default()
    });
    let mut ids = Vec::new();
    while let Ok(id) = m.create_runtime(None) {
        ids.push(id);
        assert!(ids.len() < 100, "the cap must bite before this");
    }
    assert_eq!(ids.len(), 3, "exactly max_runtimes slots, no more");
    let err = m.create_runtime(None).expect_err("the cap must refuse");
    assert_eq!(err.kind, iii_python_core::error::ErrorKind::Capacity);

    m.destroy_runtime(&ids.pop().unwrap()).unwrap();
    m.create_runtime(None)
        .expect("a freed slot must be usable again");
}

/// The backstop for a caller that never tears a runtime down. Both
/// directions, because a sweeper that reaps everything passes a
/// reaps-the-idle-one test on its own. Mutation: drop the TTL comparison,
/// and the second half fails.
#[tokio::test]
async fn the_sweep_reaps_idle_runtimes_and_spares_fresh_ones() {
    let eager = own_manager(PythonEngineConfig {
        idle_ttl_secs: 0,
        ..PythonEngineConfig::default()
    });
    let id = eager.create_runtime(None).unwrap();
    assert_eq!(eager.sweep_idle(), vec![id]);
    assert_eq!(eager.live_runtime_count(), 0);

    let patient = own_manager(PythonEngineConfig {
        idle_ttl_secs: 3600,
        ..PythonEngineConfig::default()
    });
    patient.create_runtime(None).unwrap();
    assert!(
        patient.sweep_idle().is_empty(),
        "a runtime inside its TTL must survive the sweep"
    );
    assert_eq!(patient.live_runtime_count(), 1);
}

// --- the guest iii bridge ---

use futures::future::BoxFuture;
use iii_python_core::runner::GuestBridge;

/// Records what the guest asked for and answers from a script.
struct StubBridge {
    calls: std::sync::Mutex<Vec<(String, serde_json::Value, u64)>>,
    answer: Result<serde_json::Value, String>,
}

impl GuestBridge for StubBridge {
    fn call(
        &self,
        fn_id: String,
        payload: serde_json::Value,
        timeout_ms: u64,
    ) -> BoxFuture<'static, Result<serde_json::Value, String>> {
        self.calls
            .lock()
            .unwrap()
            .push((fn_id, payload, timeout_ms));
        let answer = self.answer.clone();
        Box::pin(async move { answer })
    }
}

fn bridged(answer: Result<serde_json::Value, String>) -> (Arc<Manager>, Arc<StubBridge>) {
    let bridge = Arc::new(StubBridge {
        calls: std::sync::Mutex::new(Vec::new()),
        answer,
    });
    let m = Manager::with_bridge(
        Arc::new(PythonEngineConfig::default()),
        runner(),
        bridge.clone(),
    );
    (m, bridge)
}

/// The round trip: guest calls out, host answers, guest gets the value back
/// as an ordinary Python object. Mutation: drop the `bridge_pump` arm from
/// the `select!` and the guest blocks until its budget expires.
#[tokio::test]
async fn guest_code_can_call_the_bus_and_use_the_answer() {
    let (m, bridge) = bridged(Ok(serde_json::json!({ "n": 7 })));
    let resp = m
        .run(req(
            "answer = iii.trigger({'function_id': 'state::get', 'payload': {'key': 'k'}})\nresult = answer['n'] * 2",
        ))
        .await
        .expect("the bridged run must succeed");
    assert_eq!(resp.result, serde_json::json!(14));

    let calls = bridge.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "state::get");
    assert_eq!(calls[0].1, serde_json::json!({ "key": "k" }));
}

/// A bus error must surface as a catchable Python exception, not kill the
/// run — the tenant may well have a fallback.
#[tokio::test]
async fn a_bus_error_is_a_catchable_python_exception() {
    let (m, _) = bridged(Err("state::get: not found".into()));
    let resp = m
        .run(req(
            "try:\n    iii.trigger({'function_id': 'state::get'})\n    result = 'no raise'\nexcept RuntimeError as e:\n    result = str(e)",
        ))
        .await
        .unwrap();
    assert!(
        resp.result.as_str().unwrap().contains("not found"),
        "got {:?}",
        resp.result
    );
}

/// Without a bridge there is no global at all, rather than one that always
/// refuses — a tenant cannot tell those apart from inside. Mutation: install
/// `iii` unconditionally in `wrapper.py`.
#[tokio::test]
async fn there_is_no_iii_global_without_a_bridge() {
    let resp = manager()
        // Reference the name and catch NameError. `'iii' in dir(__builtins__)`
        // looks equivalent and is not: in an exec'd namespace `__builtins__`
        // is often the dict, and `dir(a_dict)` lists dict METHODS, not keys —
        // so that form answers `false` whether or not the global exists, and a
        // mutation that installed `iii` unconditionally went undetected.
        .run(req(
            "try:\n    iii\n    result = 'present'\nexcept NameError:\n    result = 'absent'",
        ))
        .await
        .unwrap();
    assert_eq!(resp.result, serde_json::json!("absent"));
}

/// The guest's own timeout must be clamped to what is LEFT of the run's
/// budget. Unclamped, this would blow the run's deadline every time and the
/// tenant would never see the bus error it could have handled. Mutation:
/// clamp only to a fixed ceiling.
#[tokio::test]
async fn a_guest_timeout_is_clamped_to_the_runs_remaining_budget() {
    let (m, bridge) = bridged(Ok(serde_json::Value::Null));
    let mut r = req("iii.trigger({'function_id': 'x::y', 'timeout_ms': 600000})\nresult = None");
    r.timeout_ms = Some(3_000);
    m.run(r).await.unwrap();

    let asked = bridge.calls.lock().unwrap()[0].2;
    assert!(
        asked <= 3_000,
        "the guest asked for 600000ms inside a 3000ms run; host passed {asked}"
    );
    assert!(asked > 0, "a clamped timeout must still be usable: {asked}");
}

/// Tenant output and request frames share one pipe, so they must not be able
/// to become each other. Mutation: drop the rolling tail in `FramingSink`
/// and a marker split across two writes is silently logged as output.
#[tokio::test]
async fn tenant_output_and_request_frames_do_not_contaminate_each_other() {
    let (m, bridge) = bridged(Ok(serde_json::json!("answered")));
    let resp = m
        .run(req(
            "print('before')\nresult = iii.trigger({'function_id': 'x::y'})\nprint('after')",
        ))
        .await
        .unwrap();
    assert_eq!(resp.result, serde_json::json!("answered"));
    assert_eq!(
        resp.stdout, "before\nafter\n",
        "the frame must not appear in tenant output"
    );
    assert_eq!(bridge.calls.lock().unwrap().len(), 1);
}

/// `memory_mb` binds a runtime for its whole life, so a per-call value against
/// an existing runtime is refused rather than silently ignored.
///
/// Mutation: drop the guard in `run_in` and let the field through. Wasm linear
/// memory only ever grows, so the second caller would inherit whatever the
/// first grew to — the number they asked for would be a promise the sandbox
/// cannot keep, and silently ignoring it is how a caller learns that too late.
#[tokio::test]
async fn a_per_call_memory_mb_against_a_runtime_is_refused() {
    let m = own_manager(PythonEngineConfig::default());
    let id = m.create_runtime(None).unwrap();

    let mut r = req("result = 1");
    r.memory_mb = Some(256);
    let err = m.run_in(&id, r).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidInput, "{}", err.message);
    assert!(err.message.contains("memory_mb"), "{}", err.message);

    // The same request without the field runs.
    assert_eq!(
        m.run_in(&id, req("result = 1")).await.unwrap().result,
        serde_json::json!(1)
    );
    m.destroy_runtime(&id).unwrap();
}

/// A runtime backing a registered function outlives its own idleness.
///
/// Mutation: drop the `pinned` filter from `sweep_idle`. Nothing bumps a
/// registration's activity between invocations, so an unpopular registered
/// function would be reaped and its next caller would get `RuntimeNotFound`
/// for a function the catalog still advertises.
#[tokio::test]
async fn a_pinned_runtime_is_never_swept() {
    let m = own_manager(PythonEngineConfig {
        idle_ttl_secs: 0,
        ..PythonEngineConfig::default()
    });
    let pinned = m.create_runtime(None).unwrap();
    let ordinary = m.create_runtime(None).unwrap();
    m.pin_runtime(&pinned).unwrap();

    let reaped = m.sweep_idle();
    assert_eq!(reaped, vec![ordinary.clone()], "swept the wrong runtimes");
    assert_eq!(m.live_runtime_count(), 1);

    // Still usable, and still the same interpreter.
    m.run_in(&pinned, req("kept = 7")).await.unwrap();
    assert_eq!(
        m.run_in(&pinned, req("result = kept"))
            .await
            .unwrap()
            .result,
        serde_json::json!(7)
    );

    // Teardown is the only way out.
    m.destroy_runtime(&pinned).unwrap();
    assert_eq!(m.live_runtime_count(), 0);
}

/// Pinning something that is not there is an error, not a silent no-op — a
/// caller that pins a stale id must learn its registration has no runtime.
#[tokio::test]
async fn pinning_an_unknown_runtime_is_an_error() {
    let m = own_manager(PythonEngineConfig::default());
    let err = m.pin_runtime("rt-nope").unwrap_err();
    assert_eq!(err.kind, ErrorKind::RuntimeNotFound);
}

/// The host's ANSWER to `iii.trigger` is length-prefixed the same way a turn
/// frame is, so it carries the same hazard.
///
/// Mutation: read `sys.stdin` instead of `sys.stdin.buffer` in `_read_frame`.
/// The prefix counts bytes and text-mode `read`/`len` count characters, so a
/// host function returning any non-ASCII value blocks the guest until its
/// deadline — a hang that only ever shows up in production data.
#[tokio::test]
async fn a_non_ascii_bridge_answer_does_not_hang_the_guest() {
    let (m, _b) = bridged(Ok(serde_json::json!({"city": "東京", "note": "±3°"})));
    let out = m
        .run(req(
            "a = iii.trigger({'function_id': 'geo::lookup'})\nresult = a['city'] + a['note']",
        ))
        .await
        .expect("the bridge answer must arrive intact");
    assert_eq!(out.result, serde_json::json!("東京±3°"));
}
