use std::sync::{Arc, OnceLock};

use iii_python_core::runner::{ExitKind, RunSpec, Runner};

fn spec(code: &str) -> RunSpec {
    RunSpec {
        work_dir: None,
        bridge: None,
        namespace: None,
        code: code.as_bytes().to_vec(),
        payload_json: None,
        timeout_ms: 10_000,
        memory_mb: 128,
    }
}

/// One process-wide runner, matching production (`main.rs` boots exactly one)
/// and Task 5's tests. Every per-run knob lives on `RunSpec`, so no test needs
/// its own `Engine`, `InstancePre`, or epoch ticker thread.
fn boot() -> Arc<Runner> {
    static R: OnceLock<Arc<Runner>> = OnceLock::new();
    R.get_or_init(|| Runner::boot().expect("runner boots"))
        .clone()
}

#[tokio::test]
async fn happy_path_produces_ok_envelope_and_captures_stdout() {
    let r = boot();
    let out = r
        .run(&spec("print(\"working\")\nresult = 2 + 2"))
        .await
        .unwrap();
    assert!(matches!(out.exit, ExitKind::Clean(0)));
    let env: serde_json::Value = serde_json::from_slice(&out.envelope.unwrap()).unwrap();
    assert_eq!(env["result"], 4);
    assert_eq!(String::from_utf8_lossy(&out.stdout.bytes), "working\n");
    assert!(!out.timed_out);
    assert!(!out.memory_denied);
}

#[tokio::test]
async fn payload_json_reaches_the_guest() {
    let r = boot();
    let mut s = spec("result = payload[\"n\"] * 3");
    s.payload_json = Some("{\"n\": 14}".to_string());
    let out = r.run(&s).await.unwrap();
    let env: serde_json::Value = serde_json::from_slice(&out.envelope.unwrap()).unwrap();
    assert_eq!(env["result"], 42);
}

#[tokio::test]
async fn tight_loop_hits_the_epoch_deadline_and_the_runner_survives() {
    let r = boot();
    let mut s = spec("while True:\n    pass");
    // 2000, not 500. A guest spends its first ~250 ms in CPython startup,
    // *parked* in filesystem calls where the epoch cannot see it — so under
    // parallel-suite contention a 500 ms budget could expire while it was
    // still parked, the backstop would fire instead, and the assertion below
    // (which is this test's whole job) would fail for a scheduling reason.
    // Observed once. 2000 ms lands the deadline deep in the pure-spin phase.
    s.timeout_ms = 2_000;
    let started = std::time::Instant::now();
    let out = r.run(&s).await.unwrap();
    assert!(out.timed_out, "epoch flag must be set");
    // Spinning wasm must be caught by the epoch, not by the wall-clock
    // backstop: a fiber that never returns `Pending` never lets the timeout be
    // polled, so this is structural rather than a race. Without the assertion
    // the epoch path could rot unnoticed behind the backstop.
    match &out.exit {
        ExitKind::Trap(msg) => assert!(
            msg.contains("wasm trap: interrupt"),
            "expected an epoch trap, got: {msg}"
        ),
        other => panic!("expected a trap, got {other:?}"),
    }
    assert!(started.elapsed().as_millis() < 5_000, "kill must be prompt");
    assert!(out.envelope.is_none(), "a killed run leaves no envelope");

    // The shared Engine/Module must be unharmed.
    let again = r.run(&spec("result = 1")).await.unwrap();
    assert!(matches!(again.exit, ExitKind::Clean(0)));
}

/// A spinning guest must hand the worker back on every epoch slice, not hold
/// it for the whole budget. This runs on the default current-thread flavor on
/// purpose: with one worker, a guest that fails to yield starves the runtime
/// outright, so the test is strictly more demanding than a multi-thread one
/// where spare workers would mask the bug.
#[tokio::test]
async fn spinning_guests_do_not_starve_the_runtime() {
    let r = boot();
    let spins: Vec<_> = (0..3)
        .map(|_| {
            let r = r.clone();
            tokio::spawn(async move {
                let mut s = spec("while True:\n    pass");
                s.timeout_ms = 3_000;
                r.run(&s).await.unwrap()
            })
        })
        .collect();

    // Unrelated timer work on the same runtime must keep its schedule while
    // those three spin. A single short sleep would prove nothing: for its
    // first ~250 ms each guest is still in CPython startup, whose filesystem
    // calls suspend the fiber anyway. So run a full second of timer work,
    // which necessarily extends into the pure-spin phase. Held hostage, it
    // does not finish until the guests' budget expires.
    let started = std::time::Instant::now();
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let slept = started.elapsed();
    // 3s, not 2s. The fixed path measures 1.07 s and the unfixed one 2.76 s,
    // so 3 s still discriminates — but 0.93 s of headroom is not enough for a
    // 2-core CI runner with three guests each burning a core.
    assert!(
        slept < std::time::Duration::from_millis(3_000),
        "1s of timer work took {slept:?} — spinning guests are starving the runtime"
    );

    for h in spins {
        assert!(h.await.unwrap().timed_out);
    }
}

/// `run()`'s future must be `Send` so the manager can hold it across a spawn.
/// `TypedFunc::call_async` requires `Data: Send`, but nothing compiled that
/// proof until now, and `#[tokio::test]` defaults to current-thread — Task 5
/// would otherwise have been the first to find out.
#[tokio::test]
async fn run_future_is_send() {
    fn assert_send<T: Send>(_: T) {}
    let r = boot();
    assert_send(r.run(&spec("result = 1")));
}

/// Epoch interruption is only observed at wasm back-edges and function
/// entries, so a guest parked in a host call is invisible to it. Before the
/// async rewrite this run held its thread for the full 5 s (measured), i.e.
/// `timeout_ms` bounded nothing that mattered.
#[tokio::test]
async fn guest_parked_in_a_host_call_is_still_killed_on_time() {
    let r = boot();
    let mut s = spec("import time\ntime.sleep(30)\nresult = 'escaped'");
    s.timeout_ms = 500;
    let started = std::time::Instant::now();
    let out = r.run(&s).await.unwrap();
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "a sleeping guest must not outlive its budget; took {elapsed:?}"
    );
    assert!(out.timed_out, "the kill must be reported as a timeout");
    assert!(matches!(out.exit, ExitKind::Trap(_)));
    assert!(out.envelope.is_none());

    // The point is not that the caller got an answer — it is that the thread
    // and the store came back. A fix that reported the timeout while leaking
    // the blocked fiber would pass every assertion above and still be the DoS.
    let again = r.run(&spec("result = 'alive'")).await.unwrap();
    assert!(matches!(again.exit, ExitKind::Clean(0)));
    let env: serde_json::Value = serde_json::from_slice(&again.envelope.unwrap()).unwrap();
    assert_eq!(env["result"], "alive");
}

#[tokio::test]
async fn allocation_loop_trips_the_limiter_and_the_runner_survives() {
    let r = boot();
    let mut s = spec("xs = []\nwhile True:\n    xs.append(\"a\" * 1_000_000)");
    s.memory_mb = 64;
    let out = r.run(&s).await.unwrap();
    assert!(out.memory_denied, "limiter flag must be set");
    // Either CPython surfaced MemoryError and died with a traceback envelope,
    // or growth failure trapped — both acceptable; the flag is the signal.
    let again = r.run(&spec("result = 1")).await.unwrap();
    assert!(matches!(again.exit, ExitKind::Clean(0)));
}

#[tokio::test]
async fn tenant_catching_memory_error_completes_with_flag_set() {
    let r = boot();
    let mut s = spec(
        "xs = []\ntry:\n    while True:\n        xs.append(\"a\" * 1_000_000)\nexcept MemoryError:\n    xs = None\nresult = \"recovered\"",
    );
    s.memory_mb = 64;
    let out = r.run(&s).await.unwrap();
    let env: serde_json::Value = serde_json::from_slice(&out.envelope.unwrap()).unwrap();
    assert_eq!(env["result"], "recovered");
    assert!(
        out.memory_denied,
        "flag stays true even though the tenant recovered"
    );
}

#[tokio::test]
async fn stdout_flood_is_capped_without_erroring_the_guest() {
    let r = boot();
    let out = r
        .run(&spec(
            "for _ in range(200_000):\n    print(\"x\" * 100)\nresult = \"done\"",
        ))
        .await
        .unwrap();
    let env: serde_json::Value = serde_json::from_slice(&out.envelope.unwrap()).unwrap();
    assert_eq!(
        env["result"], "done",
        "prints beyond the cap must not raise in the guest"
    );
    assert!(out.stdout.bytes.len() <= iii_python_core::config::MAX_LOG_BYTES);
    assert!(out.stdout.dropped > 0);
}

/// The brief's `RunOutcome` gives stderr its own cap and its own dropped
/// counter; nothing else in this suite would notice a runner that merged the
/// two streams into one pipe or dropped stderr on the floor.
#[tokio::test]
async fn stderr_is_captured_independently_of_stdout() {
    let r = boot();
    let out = r
        .run(&spec(
            "import sys\nprint(\"to-out\")\nprint(\"to-err\", file=sys.stderr)\nresult = 1",
        ))
        .await
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout.bytes), "to-out\n");
    assert_eq!(String::from_utf8_lossy(&out.stderr.bytes), "to-err\n");
    assert_eq!(out.stdout.dropped, 0);
    assert_eq!(out.stderr.dropped, 0);
}

#[tokio::test]
async fn runs_are_isolated_from_each_other() {
    let r = boot();
    let a = r.run(&spec("import os\nopen(\"/out/marker\", \"w\").write(\"a\")\nresult = os.path.exists(\"/out/result.json\")")).await.unwrap();
    let env: serde_json::Value = serde_json::from_slice(&a.envelope.unwrap()).unwrap();
    assert_eq!(
        env["result"], false,
        "a fresh /out must be empty at exec time"
    );
    let b = r
        .run(&spec("import os\nresult = os.path.exists(\"/out/marker\")"))
        .await
        .unwrap();
    let env: serde_json::Value = serde_json::from_slice(&b.envelope.unwrap()).unwrap();
    assert_eq!(
        env["result"], false,
        "one run's /out must be invisible to the next"
    );

    // /out being writable is only half the property: `/` and `/run` must not
    // be. That holds by construction today and is one `DirPerms::all()` typo
    // away from silently inverting, so pin it here.
    let c = r
        .run(&spec(concat!(
            "checks = []\n",
            "for p in (\"/run/code.py\", \"/lib/probe\", \"/probe\"):\n",
            "    try:\n",
            "        open(p, \"w\")\n",
            "        checks.append(\"WRITABLE \" + p)\n",
            "    except OSError:\n",
            "        checks.append(\"ro\")\n",
            "result = checks",
        )))
        .await
        .unwrap();
    let env: serde_json::Value = serde_json::from_slice(&c.envelope.unwrap()).unwrap();
    assert_eq!(
        env["result"],
        serde_json::json!(["ro", "ro", "ro"]),
        "/run and / must be read-only to the guest"
    );
}

/// The host must not size an allocation from a number the tenant picked.
/// `wrapper.py`'s MAX_RESULT_BYTES only binds cooperating code — tenant code
/// shares the interpreter and can write /out/result.json itself.
#[tokio::test]
async fn oversized_guest_written_envelope_is_refused_not_read() {
    let r = boot();
    let out = r
        .run(&spec(
            "import os\nopen(\"/out/result.json\", \"w\").write(\"x\" * 8_000_000)\nos._exit(0)",
        ))
        .await
        .unwrap();
    assert!(matches!(out.exit, ExitKind::Clean(0)));
    assert!(
        out.envelope.is_none(),
        "an over-cap envelope must be refused, not truncated into something that parses"
    );

    // ...and the refusal must be a cap, not a blanket rejection of anything
    // the guest wrote directly.
    let ok = r
        .run(&spec(concat!(
            "import json, os\n",
            "open(\"/out/result.json\", \"w\").write(json.dumps({\"ok\": True, \"result\": \"y\" * 1000}))\n",
            "os._exit(0)",
        ))).await
        .unwrap();
    let env: serde_json::Value = serde_json::from_slice(&ok.envelope.unwrap()).unwrap();
    assert_eq!(env["result"], "y".repeat(1000));
}

/// `python.wasm` declares a 40 MiB minimum memory, which the limiter is asked
/// for before any code runs — and `config.rs` clamps `memory_mb` to 1..=max,
/// so a caller can legitimately ask for less. That must surface as a denied
/// run, not as an `Err` the manager would report as `internal`.
#[tokio::test]
async fn memory_below_the_interpreter_minimum_is_denied_not_an_error() {
    let r = boot();
    let mut s = spec("result = 1");
    s.memory_mb = 8;
    let out = r
        .run(&s)
        .await
        .expect("a too-small cap is a run outcome, not an Err");
    assert!(out.memory_denied);
    assert!(matches!(out.exit, ExitKind::Trap(_)));
    assert!(out.envelope.is_none());

    let again = r.run(&spec("result = 1")).await.unwrap();
    assert!(matches!(again.exit, ExitKind::Clean(0)));
}

/// `/out` is the one resource wasmtime does not bound for us: memory has the
/// limiter, CPU the epoch, logs the capped pipes, inputs and the envelope
/// their byte caps — guest-writable disk had nothing. `$TMPDIR` is tmpfs on
/// most Linux hosts, so this is host RAM spent outside `memory_mb`.
#[tokio::test]
async fn filling_the_out_dir_is_killed_and_the_runner_survives() {
    let r = boot();
    let mut s = spec(concat!(
        "with open(\"/out/flood\", \"wb\") as f:\n",
        "    while True:\n",
        "        f.write(b\"x\" * 1_000_000)\n",
        "        f.flush()\n",
    ));
    // Long enough that finishing on the clock instead would be the bug.
    s.timeout_ms = 30_000;
    let started = std::time::Instant::now();
    let out = r.run(&s).await.unwrap();
    assert_eq!(
        out.disk_exceeded,
        Some("/out"),
        "the /out budget must be what killed it, named as such"
    );
    assert!(!out.timed_out, "a disk kill is not a timeout");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(15),
        "the kill must be prompt, not the wall-clock budget expiring"
    );
    assert!(out.envelope.is_none());

    let again = r.run(&spec("result = 'alive'")).await.unwrap();
    assert!(matches!(again.exit, ExitKind::Clean(0)));
    let env: serde_json::Value = serde_json::from_slice(&again.envelope.unwrap()).unwrap();
    assert_eq!(env["result"], "alive");
}

/// The scan is the other half of the bound: a guest that makes a million tiny
/// files makes the *walk* the attack, so entries are capped too.
#[tokio::test]
async fn flooding_out_with_tiny_files_is_killed_on_the_entry_cap() {
    let r = boot();
    let mut s = spec(concat!(
        "i = 0\n",
        "while True:\n",
        "    open(\"/out/f%d\" % i, \"w\").write(\"x\")\n",
        "    i += 1\n",
    ));
    s.timeout_ms = 30_000;
    let out = r.run(&s).await.unwrap();
    assert_eq!(
        out.disk_exceeded,
        Some("/out"),
        "the entry cap must kill it too"
    );
    assert!(!out.timed_out);

    let again = r.run(&spec("result = 1")).await.unwrap();
    assert!(matches!(again.exit, ExitKind::Clean(0)));
}

/// An ordinary run must not trip the /out budget: one small file, one
/// directory entry, nowhere near either bound.
#[tokio::test]
async fn an_honest_run_never_trips_the_out_budget() {
    let r = boot();
    let mut s = spec("import time\ntime.sleep(1.5)\nresult = 'fine'");
    s.timeout_ms = 10_000;
    let out = r.run(&s).await.unwrap();
    assert!(
        out.disk_exceeded.is_none(),
        "1.5s of honest work must survive"
    );
    let env: serde_json::Value = serde_json::from_slice(&out.envelope.unwrap()).unwrap();
    assert_eq!(env["result"], "fine");
}

/// `/out` has `DirPerms::MUTATE`, which is all `path_symlink` is gated on, and
/// only *rooted* link targets are refused — so the guest can point
/// `result.json` at an arbitrary relative path and, with a plain
/// `File::open`, make the host read it with the worker's own authority.
#[tokio::test]
async fn a_symlinked_envelope_is_not_read_through() {
    let r = boot();
    // The link target is a real file with real envelope-shaped bytes, so a
    // host that follows the link succeeds loudly rather than failing quietly.
    let bait = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(bait.path(), br#"{"ok": true, "result": "ESCAPED"}"#).unwrap();
    let depth = bait.path().components().count();
    let code = format!(
        concat!(
            "import os\n",
            "os.symlink(\"{}\", \"/out/result.json\")\n",
            "os._exit(0)\n",
        ),
        // A relative traversal: cap-primitives refuses a rooted target, but
        // not this.
        "../".repeat(depth) + bait.path().to_str().unwrap().trim_start_matches('/')
    );
    let out = r.run(&spec(&code)).await.unwrap();
    assert!(matches!(out.exit, ExitKind::Clean(0)));
    assert!(
        out.envelope.is_none(),
        "the host must refuse a non-regular-file envelope, got {:?}",
        out.envelope.as_deref().map(String::from_utf8_lossy)
    );
}

#[tokio::test]
async fn guest_early_exit_yields_clean_nonzero_status_and_no_envelope() {
    let r = boot();
    let out = r.run(&spec("import os\nos._exit(3)")).await.unwrap();
    assert!(matches!(out.exit, ExitKind::Clean(3)));
    assert!(out.envelope.is_none());
}
