//! End-to-end: engine → sandbox-code-runner → iii-sandbox → microVM → runner.
//!
//! GATED: set SANDBOX_CODE_RUNNER_E2E=1 to run (needs /dev/kvm, network for the
//! first image pull, and an engine binary — same convention as
//! node-engine's integration test). Skips silently otherwise so
//! `cargo test` is green on machines without virtualization.

use std::time::Duration;

fn gated() -> bool {
    if std::env::var("SANDBOX_CODE_RUNNER_E2E").as_deref() == Ok("1") {
        return false;
    }
    eprintln!("SKIPPED: set SANDBOX_CODE_RUNNER_E2E=1 (and III_BIN) to run the e2e test");
    true
}

/// Bind :0 to have the OS pick a genuinely free port, then release it.
fn pick_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind :0")
        .local_addr()
        .unwrap()
        .port()
}

fn wait_for_listen(port: u16, deadline: Duration) {
    let start = std::time::Instant::now();
    while start.elapsed() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("nothing listening on port {port} after {deadline:?}");
}

/// Spawn a local `iii` engine the way `node-engine/tests/integration.rs`
/// does (Task 9 Step 1): resolve the binary, write a `workers:` config with
/// an `iii-worker-manager` block bound to `port`, and start it with
/// `-c <path> --no-update-check`.
///
/// Two differences from that recipe, both required by this task:
///
/// - Binary resolution also honors `III_BIN` (checked first, falling back
///   to `which::which("iii")` — node-engine's actual lookup). node-engine's
///   committed test has no `III_BIN` support at all; it is PATH-only. This
///   test adds the env override because Step 3's verification command
///   (`III_BIN=<engine binary> cargo test ...`) requires it.
/// - The config additionally carries an `iii-sandbox` block. Confirmed
///   against `iii-worker/src/sandbox_daemon/README.md`'s "Sample
///   Configuration", `docs/creating-workers/sandboxes.mdx`, and — decisively
///   — the commented-out block already living under `workers:` in the real
///   engine's own `engine/config.yaml` and the `workers:` list in
///   `sdk/fixtures/config-test.yaml`: the block is a `- name: iii-sandbox`
///   entry under the same top-level `workers:` key as `iii-worker-manager`,
///   not a separate top-level key.
fn spawn_engine(port: u16, home: &std::path::Path) -> std::process::Child {
    let iii_bin = std::env::var_os("III_BIN")
        .map(std::path::PathBuf::from)
        .or_else(|| which::which("iii").ok())
        .expect(
            "III_BIN must point at an iii engine binary, or `iii` must be on PATH \
             (same convention as node-engine/tests/integration.rs)",
        );

    let cfg_path = home.join("config.yaml");
    let cfg_body = format!(
        "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {port}\n\n  - name: iii-sandbox\n    config:\n      auto_install: true\n      image_allowlist:\n        - python\n        - node\n"
    );
    std::fs::write(&cfg_path, &cfg_body).expect("write engine config.yaml");

    std::process::Command::new(&iii_bin)
        .arg("-c")
        .arg(&cfg_path)
        .arg("--no-update-check")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn iii engine binary")
}

/// Kills whatever processes have been handed to it and removes the scratch
/// home on drop — including when a panic unwinds through this scope, not
/// just on a normal return.
///
/// `wait_for_listen` and the `sandbox-code-runner` spawn below both run BEFORE the
/// `catch_unwind` block, so a panic there (e.g. the engine hanging mid-boot)
/// would otherwise skip straight past the manual cleanup at the bottom of
/// this function. `std::process::Child` does not kill its process on drop,
/// so that window would leak the already-spawned engine (and the scratch
/// home) on exactly the failure path a developer most needs clean state on.
/// Mirrors `KillOnDrop` in `node-engine/tests/integration.rs`.
struct Cleanup {
    engine: Option<std::process::Child>,
    worker: Option<std::process::Child>,
    home: Option<std::path::PathBuf>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        for c in [&mut self.engine, &mut self.worker].into_iter().flatten() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(home) = &self.home {
            let _ = std::fs::remove_dir_all(home);
        }
    }
}

/// Every LIVE (never-stopped) sandbox_id the daemon currently knows about.
/// `sandbox::stop` only marks a registry entry `stopped: true` — it does not
/// remove it (`SandboxRegistry::remove` is a separate, reaper-driven path) —
/// so a raw id-set comparison across `sandbox::list` calls would see a
/// STOPPED sandbox as still "there" and falsely fail a leak check. Filtering
/// to `stopped == false` is what actually answers "is a VM still running and
/// holding a daemon slot", which is the thing a leak means.
async fn live_sandbox_ids(iii: &iii_sdk::IIIClient) -> std::collections::HashSet<String> {
    let resp = iii
        .trigger(iii_sdk::protocol::TriggerRequest {
            function_id: "sandbox::list".into(),
            payload: serde_json::json!({}),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
        .expect("sandbox::list");
    resp["sandboxes"]
        .as_array()
        .expect("sandboxes is an array")
        .iter()
        .filter(|s| s["stopped"] == false)
        .filter_map(|s| s["sandbox_id"].as_str().map(str::to_string))
        .collect()
}

/// Proves the redesigned execution model against REAL VMs — the one thing no
/// unit test (which only ever hands the manager a `FakeEngine`) can:
///
/// - a one-shot eval returns a result and leaves NO live sandbox behind
///   (checked via `sandbox::list`, not just the response shape);
/// - `keep: true` DOES leave exactly one live sandbox, and its `runtime_id`
///   addresses that same VM (same filesystem) for a later eval;
/// - `register_function` with NO `runtime_id` works, and the function it
///   publishes answers real calls on the real bus;
/// - `teardown` by `namespace` unregisters it and stops its runtime.
async fn one_shot_keep_and_namespace_over_the_real_chain(iii: &iii_sdk::IIIClient) {
    let eval = |payload: serde_json::Value| iii_sdk::protocol::TriggerRequest {
        function_id: "sandbox-code-runner::run".into(),
        payload,
        action: None,
        timeout_ms: Some(35_000),
    };

    let before = live_sandbox_ids(iii).await;

    // One-shot (default): a result, no runtime_id, and no live sandbox left.
    let ephemeral = iii
        .trigger(eval(
            serde_json::json!({ "lang": "python", "code": "print(1+1)" }),
        ))
        .await
        .expect("one-shot eval");
    assert_eq!(ephemeral["stdout"], "2\n", "{ephemeral}");
    assert_eq!(ephemeral["exit_code"], 0);
    assert!(
        ephemeral.get("runtime_id").is_none(),
        "a one-shot eval's response must carry no runtime_id: {ephemeral}"
    );
    let after_ephemeral = live_sandbox_ids(iii).await;
    assert_eq!(
        after_ephemeral, before,
        "a one-shot eval left a live sandbox behind — before {before:?}, after {after_ephemeral:?}"
    );

    // `keep: true`: DOES leave exactly one live sandbox, addressable by the
    // minted runtime_id.
    let kept = iii
        .trigger(eval(serde_json::json!({
            "lang": "python",
            "code": "open('/tmp/kept-probe','w').write('kept')",
            "keep": true,
        })))
        .await
        .expect("kept eval");
    assert_eq!(kept["exit_code"], 0, "{kept}");
    let runtime_id = kept["runtime_id"]
        .as_str()
        .expect("keep: true mints a runtime_id")
        .to_string();
    let after_keep = live_sandbox_ids(iii).await;
    assert_eq!(
        after_keep.len(),
        before.len() + 1,
        "keep: true must leave exactly one new live sandbox — before {before:?}, after {after_keep:?}"
    );

    // The minted runtime_id really does address that same VM: its
    // filesystem is still there.
    let reused = iii
        .trigger(eval(serde_json::json!({
            "runtime_id": runtime_id,
            "code": "print(open('/tmp/kept-probe').read())",
        })))
        .await
        .expect("reuse of the kept runtime");
    assert_eq!(
        reused["stdout"], "kept\n",
        "the kept runtime's filesystem must persist across evals: {reused}"
    );
    assert_eq!(reused["runtime_id"], runtime_id);

    // register_function with NO runtime_id: sandbox-code-runner resolves its own
    // namespace runtime, and the function answers real calls on the bus.
    let reg = iii
        .trigger(iii_sdk::protocol::TriggerRequest {
            function_id: "sandbox-code-runner::register_function".into(),
            payload: serde_json::json!({
                "function_id": "ce-e2e-ns::double",
                "lang": "python",
                "source": "def handler(p):\n    return {'doubled': p['n'] * 2}\n",
                "description": "e2e namespace probe",
            }),
            action: None,
            timeout_ms: Some(35_000),
        })
        .await
        .expect("register_function with no runtime_id");
    assert_eq!(reg["registered"], true, "{reg}");

    let call = iii
        .trigger(iii_sdk::protocol::TriggerRequest {
            function_id: "ce-e2e-ns::double".into(),
            payload: serde_json::json!({ "n": 21 }),
            action: None,
            timeout_ms: Some(35_000),
        })
        .await
        .expect("the namespace-registered function answers");
    assert_eq!(call["doubled"], 42, "{call}");

    // teardown by namespace unregisters it and stops its runtime.
    let td = iii
        .trigger(iii_sdk::protocol::TriggerRequest {
            function_id: "sandbox-code-runner::teardown".into(),
            payload: serde_json::json!({ "namespace": "ce-e2e-ns" }),
            action: None,
            timeout_ms: Some(35_000),
        })
        .await
        .expect("teardown by namespace");
    assert_eq!(td["torn_down"], true, "{td}");
    assert_eq!(td["namespace"], "ce-e2e-ns::");
    assert_eq!(td["unregistered"][0], "ce-e2e-ns::double");

    let dead = iii
        .trigger(iii_sdk::protocol::TriggerRequest {
            function_id: "ce-e2e-ns::double".into(),
            payload: serde_json::json!({ "n": 1 }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
    assert!(
        dead.is_err(),
        "ce-e2e-ns::double should be unregistered after teardown, but it answered: {dead:?}"
    );

    // Clean up the kept runtime, and confirm the live-sandbox count returns
    // exactly to baseline — no leaks anywhere across this whole sequence.
    iii.trigger(iii_sdk::protocol::TriggerRequest {
        function_id: "sandbox-code-runner::teardown".into(),
        payload: serde_json::json!({ "runtime_id": runtime_id }),
        action: None,
        timeout_ms: Some(35_000),
    })
    .await
    .expect("teardown the kept runtime");

    let after_all = live_sandbox_ids(iii).await;
    assert_eq!(
        after_all, before,
        "live sandboxes must return exactly to baseline after cleanup — before {before:?}, \
         after {after_all:?}"
    );
}

/// Proves the guest `iii` global against REAL VMs — the planted Node SDK
/// bundle, the pip-installed Python SDK, the eval wrapper, and the
/// runner-side install, none of which a `FakeEngine` test can execute for
/// real:
///
/// - evaluated NODE code triggers a bus function mid-run through the
///   embedded SDK and gets the answer;
/// - evaluated PYTHON code does the same through the pip-installed SDK;
/// - `iii.registerFunction` from an eval is live WHILE the process runs
///   (provable by self-trigger through the engine) and gone after it
///   exits — the documented ephemeral semantics;
/// - persistence goes through `sandbox-code-runner::register_function`, callable
///   via `iii.trigger` from inside an eval;
/// - a registered handler itself uses `iii` (cross-runtime call).
async fn iii_global_over_the_real_chain(iii: &iii_sdk::IIIClient) {
    let eval = |payload: serde_json::Value| iii_sdk::protocol::TriggerRequest {
        function_id: "sandbox-code-runner::run".into(),
        payload,
        action: None,
        timeout_ms: Some(60_000),
    };
    let trigger =
        |function_id: &str, payload: serde_json::Value| iii_sdk::protocol::TriggerRequest {
            function_id: function_id.into(),
            payload,
            action: None,
            timeout_ms: Some(35_000),
        };

    let before = live_sandbox_ids(iii).await;

    // A plain registered function to be the CALLEE of guest code — its
    // runtime (ce-e2e-iii, python) is distinct from any eval's VM.
    iii.trigger(trigger(
        "sandbox-code-runner::register_function",
        serde_json::json!({
            "function_id": "ce-e2e-iii::double",
            "lang": "python",
            "source": "def handler(p):\n    return {'doubled': p['n'] * 2}\n",
            "description": "e2e iii callee",
        }),
    ))
    .await
    .expect("register the callee");

    // The embedded Node SDK: an eval triggers the callee mid-run.
    let called = iii
        .trigger(eval(serde_json::json!({
            "lang": "node",
            "code": "const r = await iii.trigger({ function_id: 'ce-e2e-iii::double', payload: { n: 21 } });\nconsole.log('got', r.doubled);",
            "timeout_ms": 30_000,
        })))
        .await
        .expect("node eval that uses iii.trigger");
    assert_eq!(called["stdout"], "got 42\n", "{called}");
    assert_eq!(called["exit_code"], 0, "{called}");

    // The pip-installed Python SDK: same round trip (this is the only
    // place the create-time `pip install iii-sdk` path is proven live).
    let called_py = iii
        .trigger(eval(serde_json::json!({
            "lang": "python",
            "code": "r = iii.trigger({'function_id': 'ce-e2e-iii::double', 'payload': {'n': 21}})\nprint('got', r['doubled'])",
            "timeout_ms": 30_000,
        })))
        .await
        .expect("python eval that uses iii.trigger");
    assert_eq!(called_py["stdout"], "got 42\n", "{called_py}");

    // iii.registerFunction is the SDK's own, so it is LIVE only while the
    // eval process runs: the eval proves liveness by triggering its own
    // registration THROUGH THE ENGINE, and the bus proves the ephemerality
    // right after the process exits.
    let ephemeral = iii
        .trigger(eval(serde_json::json!({
            "lang": "node",
            "code": "iii.registerFunction('ce-e2e-ephemeral-echo', async (data) => ({ pong: data.ping }), { description: 'dies with the eval process' });\nconst r = await iii.trigger({ function_id: 'ce-e2e-ephemeral-echo', payload: { ping: 7 } });\nconsole.log('pong', r.pong);",
            "timeout_ms": 30_000,
        })))
        .await
        .expect("eval that self-registers and self-triggers");
    assert_eq!(ephemeral["stdout"], "pong 7\n", "{ephemeral}");
    let dead = iii
        .trigger(iii_sdk::protocol::TriggerRequest {
            function_id: "ce-e2e-ephemeral-echo".into(),
            payload: serde_json::json!({ "ping": 1 }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
    assert!(
        dead.is_err(),
        "an iii.registerFunction registration must die with the eval process: {dead:?}"
    );

    // The error UX regression guard (console-a2795be8): a wrong-signature
    // SDK call must die with a SHORT, readable error naming the method —
    // with a minified bundle, node's code-frame print was hundreds of KB
    // of mangled source that hit the exec output cap and CUT OFF the
    // actual message. Assert the property (short + names the call site),
    // not the SDK's exact message, so SDK upgrades don't break this.
    let bad_call = iii
        .trigger(eval(serde_json::json!({
            "lang": "node",
            "code": "iii.registerFunction({ function_id: 'demo::hello', handler: async () => ({}) });",
            "timeout_ms": 30_000,
        })))
        .await
        .expect("the wrong-signature eval still settles as a response");
    assert_eq!(bad_call["exit_code"], 1, "{bad_call}");
    let stderr = bad_call["stderr"].as_str().unwrap_or_default();
    assert!(
        stderr.contains("registerFunction"),
        "the error must name the failing call: {stderr}"
    );
    assert!(
        stderr.len() < 4_000,
        "a wrong-signature error must be a readable page, not a {}-byte \
         minified-code dump",
        stderr.len()
    );

    // Persistence goes through sandbox-code-runner::register_function — callable
    // from inside an eval via iii.trigger, surviving the eval's exit.
    let registered = iii
        .trigger(eval(serde_json::json!({
            "lang": "node",
            "code": "const r = await iii.trigger({ function_id: 'sandbox-code-runner::register_function', payload: { function_id: 'ce-e2e-iii::treble', lang: 'node', source: 'export function handler(p) { return { trebled: p.n * 3 }; }', description: 'e2e persistent registration from inside an eval' }, timeout_ms: 60000 });\nconsole.log('registered', r.function_id);",
            "timeout_ms": 90_000,
        })))
        .await
        .expect("eval that registers persistently through the bus");
    assert_eq!(
        registered["stdout"], "registered ce-e2e-iii::treble\n",
        "{registered}"
    );
    let trebled = iii
        .trigger(trigger(
            "ce-e2e-iii::treble",
            serde_json::json!({ "n": 14 }),
        ))
        .await
        .expect("the persistently-registered function answers after the eval exited");
    assert_eq!(trebled["trebled"], 42, "{trebled}");

    // A handler using iii itself: cross-runtime call from the python
    // namespace runtime to the node one.
    iii.trigger(trigger(
        "sandbox-code-runner::register_function",
        serde_json::json!({
            "function_id": "ce-e2e-iii::describe",
            "lang": "python",
            "source": "def handler(p):\n    r = iii.trigger({'function_id': 'ce-e2e-iii::treble', 'payload': {'n': p['n']}})\n    return {'trebled': r['trebled']}\n",
            "description": "e2e handler-side iii probe",
        }),
    ))
    .await
    .expect("register the iii-using handler");
    let described = iii
        .trigger(trigger(
            "ce-e2e-iii::describe",
            serde_json::json!({ "n": 4 }),
        ))
        .await
        .expect("the iii-using handler answers");
    assert_eq!(described["trebled"], 12, "{described}");

    // One namespace teardown covers both language runtimes and all three
    // functions, and the live-sandbox count returns exactly to baseline.
    let td = iii
        .trigger(trigger(
            "sandbox-code-runner::teardown",
            serde_json::json!({ "namespace": "ce-e2e-iii" }),
        ))
        .await
        .expect("teardown the iii namespace");
    assert_eq!(td["torn_down"], true, "{td}");
    let mut unregistered: Vec<String> = td["unregistered"]
        .as_array()
        .expect("unregistered array")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    unregistered.sort();
    assert_eq!(
        unregistered,
        vec![
            "ce-e2e-iii::describe".to_string(),
            "ce-e2e-iii::double".to_string(),
            "ce-e2e-iii::treble".to_string(),
        ],
        "{td}"
    );

    let after = live_sandbox_ids(iii).await;
    assert_eq!(
        after, before,
        "live sandboxes must return to baseline after the iii step — before {before:?}, \
         after {after:?}"
    );
}

#[test]
fn full_loop_eval_register_trigger_teardown() {
    if gated() {
        return;
    }
    let port = pick_port();
    let home = std::env::temp_dir().join(format!("ce-e2e-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    let mut cleanup = Cleanup {
        engine: None,
        worker: None,
        home: Some(home.clone()),
    };

    cleanup.engine = Some(spawn_engine(port, &home));
    wait_for_listen(port, Duration::from_secs(30));

    cleanup.worker = Some(
        std::process::Command::new(env!("CARGO_BIN_EXE_sandbox-code-runner"))
            .arg("--url")
            .arg(format!("ws://127.0.0.1:{port}"))
            .arg("--config")
            .arg("/nonexistent-use-defaults.yaml")
            .spawn()
            .expect("sandbox-code-runner starts"),
    );

    let result = std::panic::catch_unwind(|| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let iii = iii_sdk::register_worker(
                &format!("ws://127.0.0.1:{port}"),
                iii_sdk::InitOptions::default(),
            );

            // Poll until sandbox-code-runner::run resolves (worker connect is async).
            // Every pre-success error is treated as "not booted yet" and
            // retried — correct for the function-resolution race, but it
            // means a genuine failure looks identical to "not ready" until
            // the loop expires. Keep the last error around so a real
            // failure is diagnosable instead of surfacing as a bare
            // `booted == false`.
            let mut booted = false;
            let mut last_err: Option<String> = None;
            for attempt in 0..50 {
                // `keep: true` here (not the one-shot default): this probe
                // both proves the worker is up AND needs a runtime_id back
                // to exercise the reuse + explicit-teardown paths below.
                // First call pulls the image — give it the full window.
                let out = iii
                    .trigger(iii_sdk::protocol::TriggerRequest {
                        function_id: "sandbox-code-runner::run".into(),
                        payload: serde_json::json!({
                            "lang": "python", "code": "print(6*7)", "keep": true,
                        }),
                        action: None,
                        timeout_ms: Some(300_000),
                    })
                    .await;
                match out {
                    Ok(v) => {
                        assert_eq!(v["stdout"], "42\n", "eval output: {v}");
                        assert_eq!(v["exit_code"], 0);
                        let runtime_id = v["runtime_id"]
                            .as_str()
                            .expect("keep: true mints a runtime_id")
                            .to_string();

                        // Filesystem persists between evals in one runtime.
                        let w = iii
                            .trigger(iii_sdk::protocol::TriggerRequest {
                                function_id: "sandbox-code-runner::run".into(),
                                payload: serde_json::json!({
                                    "runtime_id": runtime_id,
                                    "code": "open('/tmp/probe','w').write('kept')",
                                }),
                                action: None,
                                timeout_ms: Some(35_000),
                            })
                            .await
                            .expect("second eval");
                        assert_eq!(w["exit_code"], 0);
                        let r = iii
                            .trigger(iii_sdk::protocol::TriggerRequest {
                                function_id: "sandbox-code-runner::run".into(),
                                payload: serde_json::json!({
                                    "runtime_id": runtime_id,
                                    "code": "print(open('/tmp/probe').read())",
                                }),
                                action: None,
                                timeout_ms: Some(35_000),
                            })
                            .await
                            .expect("third eval");
                        assert_eq!(r["stdout"], "kept\n");
                        assert_eq!(r["runtime_id"], runtime_id);

                        let td = iii
                            .trigger(iii_sdk::protocol::TriggerRequest {
                                function_id: "sandbox-code-runner::teardown".into(),
                                payload: serde_json::json!({ "runtime_id": runtime_id }),
                                action: None,
                                timeout_ms: Some(35_000),
                            })
                            .await
                            .expect("teardown succeeds");
                        assert_eq!(td["torn_down"], true);
                        assert_eq!(td["runtime_id"], runtime_id);
                        assert!(td["unregistered"].as_array().unwrap().is_empty());

                        // `td` is teardown describing its own behavior —
                        // prove it against the bus instead of trusting the
                        // self-report: the runtime it lived on must
                        // actually be gone. Short timeout so a hung call
                        // (which would mean teardown didn't really finish)
                        // fails fast instead of stalling the suite.
                        let dead_eval = iii
                            .trigger(iii_sdk::protocol::TriggerRequest {
                                function_id: "sandbox-code-runner::run".into(),
                                payload: serde_json::json!({
                                    "runtime_id": runtime_id,
                                    "code": "1",
                                }),
                                action: None,
                                timeout_ms: Some(5_000),
                            })
                            .await;
                        assert!(
                            dead_eval.is_err(),
                            "eval against torn-down runtime_id {runtime_id} should fail, \
                             but got: {dead_eval:?}"
                        );

                        one_shot_keep_and_namespace_over_the_real_chain(&iii).await;
                        iii_global_over_the_real_chain(&iii).await;

                        booted = true;
                        break;
                    }
                    Err(e) => {
                        let msg = format!("attempt {attempt}: {e}");
                        eprintln!("{msg}");
                        last_err = Some(msg);
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                }
            }
            assert!(
                booted,
                "sandbox-code-runner::run never resolved on the bus; last error: {last_err:?}"
            );
        });
    });

    // `cleanup` drops here (normal return) or while `resume_unwind` below
    // unwinds through this scope (failure) — either way the engine, worker,
    // and scratch home get torn down exactly once.
    drop(cleanup);
    if let Err(p) = result {
        std::panic::resume_unwind(p);
    }
}
