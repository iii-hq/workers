# Sandbox Consumers on Engine 0.25 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the workers-repo sandbox consumers, their test stacks, CI pins and docs onto engine 0.25's sandbox router and the `sandbox-builtin` adapter, changing consumer code only where spec 11 allows.

**Architecture:** Consumers keep calling `sandbox::*` in `default`, and the engine router forwards each call to the bound adapter. sandbox-code-runner's startup probe reads `sandbox::adapter::info` and falls back to today's probe on older engines. ide accepts `exit_code: null`. Every stack that configured `engine.workers.iii-sandbox` or required the `iii-worker`/`iii-init` companions moves to an engine `sandbox:` binding (`sandbox-mock` in the ide e2e, `sandbox-builtin` elsewhere) or to a declared `sandbox-builtin` container.

**Tech Stack:** Rust (iii-sdk 0.24.6, tokio, serde_json), TypeScript (iii-sdk node, tsx, tsc), Bash, GitHub Actions, Python (pytest, unittest).

**Spec:** docs/superpowers/specs/2026-10-20-sandbox-worker-design.md, sections 2, 5.3, 6.3, 7.1, 7.2, 9 (Consumers, workers-repo docs), 10 (step 5), 11 (regression checks), 12 (ide e2e risk). Triage decisions D5, D11, D13. Shared names: docs/superpowers/plans/2026-10-20-sandbox-shared-interfaces.md

**Repo and base:** Workers repo. Citations are at `8773952be`; `origin/main` is ahead by one harness-only commit that touches none of these files. Branch: `sandbox/p5-consumers`, based on `main` after P1 merges. Prerequisites: P1 is merged and `sandbox-builtin` 1.0.0 is in the registry; P2a, P2b and P3 have shipped as engine `iii/v0.25.0` (rollout step 4).

## Global Constraints
- "Consumers keep calling `sandbox::*` in namespace `default`, unchanged." (2, goal 4)
- Anti-goal: "Changing any `sandbox::*` request or response shape, or the S-code error wire format."
- "Consumer code is unchanged, apart from the ide `exit_code` fix and the sandbox-code-runner probe (9)."
- Consumers "declare nothing new (7.1)": no new crate or package dependency, so no consumer depends on `iii-sandbox-contract`.
- "`exec.exit_code` stays `Option<i32>`; it is `null` on signal death"
- Probe: "moves to `sandbox::adapter::info` and checks `info != null`. On engines older than 0.25 it falls back to today's probe." Its remediation text (`compose::add worker=iii-sandbox`) stays.
- "`ide/tests/e2e/*` stays standalone: its mocks register `sandbox-mock::*` instead of `sandbox::*`, and its `config.yaml` replaces the `iii-sandbox` entry with `sandbox: {adapter: sandbox-mock, namespace: ide-e2e}`. `EXEC_SANDBOX_CASES`, `III_SANDBOX_FS_NAMESPACE` and the ide S-code table stay unchanged." (9, D13)
- D13: only text that tells users to configure `engine.workers.iii-sandbox` changes; mere name mentions stay. At the base, `README.md:127` and `ide/{README.md, ARCHITECTURE.md, skills/SKILL.md, config.yaml}` hold only name mentions and still-valid `compose::add worker=iii-sandbox` commands (6.4), so they stay. Harness launchers that call `iii worker` or `worker::add` stay out of scope.
- Step 5 "Needs 0.25": every `III_RELEASE_TAG` pin becomes `'iii/v0.25.0'`.
- Router texts that consumers see (registry §4): S301 `adapter {adapter} not connected in namespace {namespace}; compose::add iii-sandbox in that project installs it`. S303 `no sandbox adapter configured: this engine is not managed by a Compose file; set sandbox.adapter in its engine config (Compose-managed engines default to sandbox-builtin)`.
- Commits follow conventional commits. Never add an AI co-author trailer or a "Generated with" line.

## Review Focus
1. At sandbox-code-runner boot, `sandbox::adapter::info` is denied by RBAC or times out. Expected: "could not verify", never "detected" from the legacy probe, because the router always registers `sandbox::create`. Pinned by Task 2 `probe_falls_back_only_when_the_router_is_absent`.
2. A sandbox `shell::exec_bg` job's process dies by signal, so `exit_code` is `null`. Expected: the job is `failed`, `exit_code` is `null`, and stdout is kept. Pinned by Task 1 `sandbox_job_null_exit_code_marks_failed`.
3. An ide call with a sandbox target arrives while no adapter is connected (router S301). Expected: the caller gets S216, and its message still carries the router's `compose::add iii-sandbox` fix. Pinned by Task 1 `remote_s301_keeps_router_fix_text`.
4. A CI job still requires `iii-worker` or `iii-init`, but a 0.25 install gets only `iii` (0.25+ releases have no such assets; `install.sh` skips them). Expected: no job requires them, otherwise every run fails. Pinned by Task 3 `test_engine_installs_require_only_iii`.
5. The template's `sandbox-builtin` sits under the `# added by compose::add` marker. It then belongs to Compose's sandbox step (P2b Task 4): an update or remove whose edit takes `needs(file)` from non-empty to empty deletes it (D11), yet computer needs it for `sandbox::*` and declares no need, so computer loses its sandboxes. Expected: the container is operator-declared, with no marker, which Compose never re-resolves or removes (spec 7.2). Pinned by Task 7 `test_sandbox_adapter_is_operator_declared`.

---

### Task 1: ide accepts `exit_code: null`

**Files:**
- Modify: `ide/src/exec/sandbox.rs:19-28, 112-125` (tests at `:140-458`)
- Modify: `ide/src/functions/exec_bg.rs:394-408` (tests in `mod sandbox_path_tests`, `:705+`)
- Test: the same files' test modules

**Interfaces:**
- Consumes: `ExecResponse.exit_code: Option<i32>` (registry §2), which the router returns unchanged (P2a).
- Produces: `pub(crate) struct SandboxExecResponse { pub stdout: String, pub stderr: String, pub exit_code: Option<i32>, pub duration_ms: u64, pub timed_out: bool }`

- [ ] **Step 1: Write the failing tests**
```rust
// ide/src/exec/sandbox.rs
#[tokio::test] async fn signal_death_null_exit_code_round_trips_as_none() {
    // StubFwd::ok(json!({"stdout":"","stderr":"Killed\n","exit_code":null,"duration_ms":3,"timed_out":false,"success":false}))
    assert_eq!(out.exit_code, None); assert!(!out.timed_out); assert_eq!(out.stderr, "Killed\n");
}
#[tokio::test] async fn remote_s301_keeps_router_fix_text() {   // Review Focus 3
    // StubFwd::remote_err("invocation_failed", r#"{"type":"transient","code":"S301","message":"adapter sandbox-builtin not connected in namespace my-proj; compose::add iii-sandbox in that project installs it","retryable":true}"#)
    assert_eq!(err.code, "S216");
    assert!(err.message.contains("compose::add iii-sandbox in that project installs it"));
}
// ide/src/functions/exec_bg.rs, mod sandbox_path_tests
#[tokio::test] async fn sandbox_job_null_exit_code_marks_failed() {   // Review Focus 2
    // ImmediateOk(json!({"stdout":"partial\n","stderr":"","exit_code":null,"duration_ms":5,"timed_out":false}))
    assert_eq!(r.record.status, JobStatus::Failed); assert_eq!(r.record.exit_code, None);
    assert_eq!(r.record.stdout, "partial\n");
}
```
- [ ] **Step 2: Run them to verify they fail.** Run `cargo test --locked --all-features --manifest-path ide/Cargo.toml -- null_exit_code remote_s301`. Expect `signal_death_…` to panic with `bad engine response: invalid type: null, expected i32` and `sandbox_job_…` to fail on stdout `"" != "partial\n"`. `remote_s301_…` already passes, because it pins existing behaviour (`ide/src/scode.rs:123-129`).
- [ ] **Step 3: Implement `exit_code: Option<i32>` on `SandboxExecResponse`.** Change `sandbox.rs:118` to `exit_code: parsed.exit_code`, `exec_bg.rs:399` to `h.record.exit_code = p.exit_code`, and `exec_bg.rs:403` to `p.exit_code == Some(0)`.
- [ ] **Step 4: Run the tests to verify they pass.** Run `cargo test --locked --all-features --manifest-path ide/Cargo.toml` and `cargo clippy --locked --all-targets --all-features --manifest-path ide/Cargo.toml -- -D warnings`. Expect all tests to pass and clippy to report no warnings.
- [ ] **Step 5: Commit.** Run `git add ide/src/exec/sandbox.rs ide/src/functions/exec_bg.rs`, then `git commit -m "fix(ide): accept a null sandbox exit_code on signal death"`.

### Task 2: sandbox-code-runner probes the router

**Files:**
- Modify: `sandbox-code-runner/src/engine.rs:110` (add the code after `impl Engine for IIIEngine`; tests at `:286-388`)
- Modify: `sandbox-code-runner/src/main.rs:8-9, 124-156`
- Test: `sandbox-code-runner/src/engine.rs` `mod tests` (uses `FakeEngine`)

**Interfaces:**
- Consumes: `sandbox::adapter::info` → `RouterInfo { adapter, namespace, contract, info: Option<AdapterInfo> }` (registry §2-3, P2a). It is read as `serde_json::Value`, so no contract dependency is added. Also consumes `classify_probe_error(raw, target) -> ProbeOutcome` (`src/error.rs:168`).
- Produces: `#[derive(Debug, Clone, PartialEq, Eq)] pub enum SandboxProbe { Ready, Missing, Unverified(String) }` and `pub async fn probe_sandbox(engine: &dyn Engine) -> SandboxProbe`.

- [ ] **Step 1: Write the failing tests.** Each test checks the result of `probe_sandbox(fake.as_ref())`. Here `ids()` stands for `fake.calls()` mapped to the function ids.
```rust
#[tokio::test] async fn probe_ready_when_the_router_reports_an_adapter() {
    // "sandbox::adapter::info" -> Ok({"adapter":"sandbox-builtin","namespace":"default","contract":{"major":1,"minor":0},
    //   "info":{"adapter":"sandbox-builtin","adapter_version":"1.0.0","contract":{"major":1,"minor":0}}})
    assert_eq!(probe, SandboxProbe::Ready); assert_eq!(ids(), ["sandbox::adapter::info"]);
}
#[tokio::test] async fn probe_missing_when_the_router_has_no_adapter() {   // same reply with "info": null
    assert_eq!(probe, SandboxProbe::Missing); assert_eq!(ids(), ["sandbox::adapter::info"]);
}
#[tokio::test] async fn probe_falls_back_on_engines_without_the_router() {
    // adapter::info -> Err("remote error (function_not_found): Function sandbox::adapter::info not found in namespace default.")
    // engine::functions::info -> Ok({}) gives Ready; -> Err("remote error (NOT_FOUND): Function 'sandbox::create' is not registered.") gives Missing
}
#[tokio::test] async fn probe_falls_back_only_when_the_router_is_absent() {   // Review Focus 1
    // adapter::info -> Err("remote error: FORBIDDEN: rbac denies sandbox::adapter::info"), then Err("invocation timed out")
    assert!(matches!(probe, SandboxProbe::Unverified(_))); assert!(!ids().contains(&"engine::functions::info"));
}
```
- [ ] **Step 2: Run them to verify they fail.** Run `cargo test --locked --all-features --manifest-path sandbox-code-runner/Cargo.toml probe_`. Expect `error[E0425]: cannot find function \`probe_sandbox\``.
- [ ] **Step 3: Implement `probe_sandbox` in `src/engine.rs`.** Make one `call("sandbox::adapter::info", json!({}), 5_000)`:
  - On `Ok(v)`, return `Ready` if `v.get("info").is_some_and(|i| !i.is_null())`, else `Missing`.
  - On `Err(raw)` with `classify_probe_error(&raw, "sandbox::adapter::info") == Free`, run today's `engine::functions::info` probe of `sandbox::create` (`main.rs:129-153`): `Ok` gives `Ready`, `Free` gives `Missing`, anything else gives `Unverified`.
  - Any other `Err(raw)` gives `Unverified(raw)`.

  `main.rs:124-156` becomes a `match`. `Ready` logs info `"sandbox adapter detected"`. `Missing` logs today's warning verbatim (`main.rs:148-149`). `Unverified` logs today's `"could not verify iii-sandbox presence"` with the error. `main.rs:8-9` imports `probe_sandbox` and `SandboxProbe` and drops `Engine as _`, `classify_probe_error` and `ProbeOutcome`, which nothing else there uses.
- [ ] **Step 4: Run the tests to verify they pass.** Run `cargo test --locked --all-features --manifest-path sandbox-code-runner/Cargo.toml` and clippy with the same flags as Task 1. Expect all tests to pass and no warnings.
- [ ] **Step 5: Commit.** Run `git add sandbox-code-runner/src/engine.rs sandbox-code-runner/src/main.rs`, then `git commit -m "fix(sandbox-code-runner): probe the sandbox router for a connected adapter"`.

### Task 3: CI on engine 0.25.0, and the ide e2e on `sandbox-mock`

These changes ship as one commit. `test_other_engine_installers_use_the_atomic_configuration_release` (`.github/scripts/tests/test_rust_ci_workflows.py:197-226`) forces every pin to the same tag. The ide e2e config is refused by 0.25 before this change and by 0.24 after it.

**Files:**
- Modify the `III_RELEASE_TAG` pins: `.github/workflows/{ci.yml:15, build.yml:39, database-e2e.yml:29, browser-scrapling-e2e.yml:19, storage-e2e.yml:28, rbac-proxy-e2e.yml:28, ide-e2e.yml:28, judge-e2e.yml:41, _worker-e2e.yml:240}` and `.github/scripts/tests/test_rust_ci_workflows.py:212`
- Modify: `.github/workflows/_worker-e2e.yml:251-253` (the companion loop becomes `test -x "$III_INSTALL_BIN_DIR/iii"`) and `:385-386` (delete the `iii-init` and `iii-worker` paths)
- Modify: `harness/tests/e2e/run-ci.sh:15-20` (check only `$iii_bin_dir/iii`). The stale `iii-worker` comments at `:34, 41, 46` and the code under them stay (D13).
- Modify: `ide/tests/e2e/config.yaml:12-26`. The `iii-sandbox` entry becomes top-level `sandbox: {adapter: sandbox-mock, namespace: ide-e2e}`.
- Modify: `ide/tests/e2e/workers/harness/src/cases-fs-sandbox.ts:18-133, 152`, `cases-sandbox-break.ts:145`, and the header comment `cases-exec-sandbox.ts:22-25`
- Modify: `ide/tests/e2e/run-tests.sh:50-55, 207-212` and `ide/tests/e2e/README.md:20-27, 80-85`
- Test: `.github/scripts/tests/test_rust_ci_workflows.py`, `ide/tests/e2e/run-tests.sh`

**Interfaces:**
- Consumes from P2a: `EngineConfig.sandbox: Option<SandboxRouterConfig { adapter, namespace = "default" }>`, the router handshake on `<adapter>::adapter::info` (major 1, else S302), and S301 for missing adapter ids.
- Consumes from P3 (Task 2): `engine/install.sh` downloads `iii-worker` and `iii-init` only when the resolved release lists them, and stays silent otherwise. 0.25+ releases have no such assets, so a 0.25.0 install gets only `iii`; the 0.24 pin keeps both until this task bumps it.
- Produces: the `sandbox-mock` test double (registry §7): `sandbox-mock::fs::{ls,stat,mkdir,rm,chmod,mv,grep,sed,write,read}` plus `sandbox-mock::adapter::info` → `{adapter:"sandbox-mock", adapter_version:"0.0.0", contract:{major:1,minor:0}}`, registered once by the harness worker in its namespace `ide-e2e`.
- Unchanged (D13): `EXEC_SANDBOX_CASES`, `runner.ts`, ide's `III_SANDBOX_FS_NAMESPACE` support (`ide/src/triggers.rs:26-49`) and its S-code table (`ide/src/scode.rs:40-69`). Only `run-tests.sh` stops setting `III_SANDBOX_FS_NAMESPACE=ide-e2e`: after the rename nothing serves `sandbox::fs::*` in `ide-e2e`, and the engine has no fallback to `default` (`iii:engine/src/function.rs:256`), so ide must reach the router in `default`.

- [ ] **Step 1: Write the failing test.** Change `:212` to expect `"iii/v0.25.0"` and add:
```python
def test_engine_installs_require_only_iii() -> None:   # Review Focus 4
    steps = workflow("_worker-e2e.yml")["jobs"]["build"]["steps"]
    run_ci = (REPOSITORY / "harness/tests/e2e/run-ci.sh").read_text().splitlines()
    texts = [named_step(steps, "Install published iii release")["run"],
             named_step(steps, "Package E2E binaries")["run"],
             "\n".join(line for line in run_ci if not line.lstrip().startswith("#"))]
    for text in texts:
        assert not re.search(r"iii-(init|worker)(?![-\w])", text), text[:120]
```
- [ ] **Step 2: Run it to verify it fails.** Run `pytest .github/scripts/tests/test_rust_ci_workflows.py -v`. Expect `test_engine_installs_require_only_iii` to FAIL on the `_worker-e2e.yml` install step, and `test_other_engine_installers_…` to FAIL with `('_worker-e2e.yml', 'build', 'Install published iii release', 'iii/v0.24.0-rc.2')`. Then, with `iii` 0.25.0 on PATH, run `ide/tests/e2e/run-tests.sh`. Expect `FATAL: engine exited before binding port`, with `[UNSUPPORTED_CONFIG_WORKERS]` naming `iii-sandbox` in the engine log tail.
- [ ] **Step 3: Implement.** Bump every pin to `'iii/v0.25.0'` and edit the two companion checks as listed above. Then change the ide e2e:
  - `cases-fs-sandbox.ts`: `setupSandboxMocks` registers the `sandbox-mock::fs::*` functions and `sandbox-mock::adapter::info` once. Delete `refreshSandboxMocks`, `mockRefs` and the unregistering: they only re-owned ids that the real `iii-sandbox` also registered. `captured[].function_id` records the `sandbox-mock::` id.
  - `:152` and `cases-sandbox-break.ts:145` expect `sandbox-mock::fs::ls` and `sandbox-mock::fs::write`, which also proves the router's forward target.
  - `run-tests.sh:212` drops `III_SANDBOX_FS_NAMESPACE=ide-e2e`.
  - The `config.yaml` comment, `cases-exec-sandbox.ts:22-25`, `run-tests.sh:50-55, 207-210` and the README say: the suite binds `sandbox-mock`, so the fs cases go through the router, and `sandbox::create` gets S301, which the existing catch in `ensureSandbox` (`cases-exec-sandbox.ts:56-61`) turns into SKIP; a real VM run binds `sandbox-builtin` instead.
- [ ] **Step 4: Run the tests to verify they pass.** Run `pytest .github/scripts/tests/ -v` (all pass) and `actionlint -color` (no output). Run `(cd ide/tests/e2e/workers/harness && npm ci && npm run build)`; expect tsc to exit 0. Run `ide/tests/e2e/run-tests.sh`; expect `HARNESS_DONE: PASS n/n`, every `FS_SANDBOX_CASES` and `SANDBOX_BREAK_CASES` case as PASS, and `[harness] SKIP ::` lines only for `EXEC_SANDBOX_CASES`. Run `ide/tests/e2e/run-tests.sh --suite=jailed --no-build`; expect `HARNESS_DONE: PASS 2/2`.
- [ ] **Step 5: Commit.** Run `git add .github/workflows .github/scripts/tests/test_rust_ci_workflows.py harness/tests/e2e/run-ci.sh ide/tests/e2e`, then `git commit -m "ci: pin engine 0.25.0 and bind the ide e2e to sandbox-mock"`.

### Task 4: Isolation e2e through `sandbox::*`

**Files:**
- Modify: `sandbox-builtin/scripts/sandbox-isolation-e2e.sh`, the engine `config.yaml` heredoc. P1 creates the file; at the base the heredoc is iii `scripts/sandbox-isolation-e2e.sh:145-151`.
- Modify: `.github/workflows/ci.yml`, job `sandbox-builtin-vm-linux` (P1), the isolation e2e step's `env`

**Interfaces:**
- Consumes (registry §6, P1): `SANDBOX_ID_PREFIX` (default `sandbox-builtin`), `ADAPTER_BIN`, `III_BIN`. The script starts the adapter in namespace `default`, on its own engine, not under Compose (D13). The engine comes from `III_RELEASE_TAG` (Task 3); the binding is P2a's `EngineConfig.sandbox`.
- Produces: none.

- [ ] **Step 1: Write the failing test.** Set `SANDBOX_ID_PREFIX: sandbox` on the vm-linux isolation step.
- [ ] **Step 2: Run it to verify it fails.** On a KVM host with engine 0.25.0, build the adapter as P1 Task 9 Step 4 does, then run `SANDBOX_ID_PREFIX=sandbox III_BIN="$(command -v iii)" ADAPTER_BIN=target/debug/sandbox-builtin bash scripts/sandbox-isolation-e2e.sh` from `sandbox-builtin/`. Expect `FAIL: sandbox daemon did not register`: on the unbound engine the readiness probe `sandbox::list` gets S303 `no sandbox adapter configured`.
- [ ] **Step 3: Implement the binding.** Add top-level `sandbox:` / `  adapter: sandbox-builtin` to the heredoc. It is unconditional, because the direct `sandbox-builtin::*` ids keep working on a bound engine.
- [ ] **Step 4: Run the tests to verify they pass.** Run the same command, then again with `SANDBOX_ID_PREFIX` unset. Expect the last line `===== result: 0 failure(s)` and exit code 0 both times.
- [ ] **Step 5: Commit.** Run `git add sandbox-builtin/scripts/sandbox-isolation-e2e.sh .github/workflows/ci.yml`, then `git commit -m "test(sandbox-builtin): run the isolation e2e through the sandbox router"`.

### Task 5: sandbox-code-runner gated e2e through the router

**Files:**
- Modify: `sandbox-code-runner/tests/integration.rs:1-16` (module doc and the `gated()` message), `:38-81` (`spawn_engine`), `:94-110` (`Cleanup`), `:491-516` (start order)

**Interfaces:**
- Consumes from P2a: the router binding.
- Consumes from P1 (registry §6): the adapter reads `III_URL`, and reads config id `sandbox-builtin` once at startup, with serde defaults that deny every image.
- Consumes: `configuration::set {id, value, flush: false}`, which iii documents as "may precede registration" (`iii:engine/src/workers/configuration/configuration.rs:564`).
- Produces: none. The adapter binary comes from env `ADAPTER_BIN`, as in the isolation e2e (registry §6).

- [ ] **Step 1: Write the failing test.** The existing `full_loop_eval_register_trigger_teardown` is the test: python and node runs, plus the `iii` global dialing back through `localhost`.
- [ ] **Step 2: Run it to verify it fails.** On a KVM host with iii 0.25.0, run `SANDBOX_CODE_RUNNER_E2E=1 III_BIN="$(command -v iii)" ADAPTER_BIN=<sandbox-builtin binary> cargo test --locked --manifest-path sandbox-code-runner/Cargo.toml --test integration -- --nocapture`. Expect `nothing listening on port … after 30s`, because the engine refuses the `iii-sandbox` entry at `:69`.
- [ ] **Step 3: Implement the router setup.**
  - The config at `:69` becomes the `iii-worker-manager` entry plus top-level `sandbox:\n  adapter: sandbox-builtin\n`; the doc at `:50-57` says so.
  - Add `fn spawn_adapter(iii_bin: &Path, port: u16) -> Child` (factor `iii_bin()` out of `spawn_engine`). It first runs `iii trigger --port {port} configuration::set --json '{"id":"sandbox-builtin","value":{"auto_install":true,"image_allowlist":["python","node"]},"flush":false}'`, which must exit 0. It then spawns `$ADAPTER_BIN` with `III_URL=ws://127.0.0.1:{port}`.
  - `Cleanup` kills the adapter too.
  - The existing 50-attempt retry covers the adapter's connect window, while calls still get S301.
- [ ] **Step 4: Run the tests to verify they pass.** Run the same command; expect `test full_loop_eval_register_trigger_teardown ... ok`. Then run `cargo test --locked --all-features --manifest-path sandbox-code-runner/Cargo.toml`; expect it to pass with the gated test skipped.
- [ ] **Step 5: Commit.** Run `git add sandbox-code-runner/tests/integration.rs`, then `git commit -m "test(sandbox-code-runner): drive the gated e2e through the sandbox router"`.

### Task 6: Harness `shell_coder_sandbox` recognizes the adapter

**Files:**
- Modify: `harness/tests/e2e/src/scenarios/shell_coder_sandbox.rs:68, 129, 360` (test at `:828-885`)

**Interfaces:**
- Consumes from P2b (registry §5): the `compose::add worker=iii-sandbox` reply has `details.container == "sandbox-builtin"`, and `add` keeps that adapter in the file with no VM need (it passes `allow_remove: false`, P2b Task 4).
- Consumes from P2a: `sandbox::adapter::info` → `RouterInfo` JSON.
- Produces: `ScenarioSpec.version` 5.

- [ ] **Step 1: Write the failing test.** In `registry_adds_require_successful_results_in_sequence`, every sandbox result becomes `add_result("call-sandbox", "sandbox-builtin", "ok")`. The calls stay `add_call(.., "iii-sandbox")`, and the expected `RegistryAddEvidence` values are unchanged.
- [ ] **Step 2: Run it to verify it fails.** Run `cargo test --locked --manifest-path harness/Cargo.toml -p harness-e2e registry_adds`. Expect `left: RegistryAddEvidence { shell_succeeded: true, sandbox_succeeded: false, ordered: false }`.
- [ ] **Step 3: Implement the adapter checks.**
  - `:360` becomes `compose_add_succeeded(message, "sandbox-builtin")`.
  - `:129` becomes `context.trigger_value("sandbox::adapter::info", json!({})).await?["info"].is_object()`, because the router always lists `sandbox::create`.
  - `:68` becomes `version: 5`, as the README's versioning rule requires (`harness/tests/e2e/README.md:279-281`).
- [ ] **Step 4: Run the tests to verify they pass.** Run the same command without a filter, then `cargo clippy --locked --manifest-path harness/Cargo.toml -p harness-e2e --all-targets -- -D warnings`. Expect all tests to pass and clippy to be clean.
- [ ] **Step 5: Commit.** Run `git add harness/tests/e2e/src/scenarios/shell_coder_sandbox.rs`, then `git commit -m "test(harness): count the sandbox-builtin adapter in shell_coder_sandbox"`.

### Task 7: Template declares `sandbox-builtin`

**Files:**
- Modify: `template/worker-compose.yaml:6-17, 230-248` and `template/tests/test_template.py:36-56, 72-77`
- Modify: `template/scripts/sync_template.py:118-121` (the importer `sync.sh` runs), `template/tests/test_sync.py:177-196`, `template/README.md:55-59`
- Modify: `computer/README.md:36-63` (the replacement snippet is the template block; drop the standalone `iii -c` snippet), `computer/images/desktop/Dockerfile:22-26`, `computer/ui/src/page/Launcher.tsx:198-199`, `computer/src/config.rs:50-52` and `ade/ui/src/configuration-forms/specs/scalar.ts:149` (both say the OCI ref is set in iii-sandbox's own configuration; it is now `sandbox-builtin`'s `custom_images`)

**Interfaces:**
- Consumes: package `sandbox-builtin` 1.0.0 and its config keys (P1; spec 7.6), the unmarked, operator-declared `version: '^1'` block that `SANDBOX_CONFIG_MOVED` prints (P2b Task 2, spec 7.9), `edit::MARKER` (registry §5, P2b), and update/remove deleting a marked adapter when their edit takes `needs(file)` from non-empty to empty (D11; `allow_remove`, P2b Task 4).
- Produces: none.

- [ ] **Step 1: Write the failing tests**
```python
ADAPTER = "sandbox-builtin"
def test_sandbox_adapter_is_operator_declared(self):   # Review Focus 5
    self.assertNotIn("workers", self.compose["engine"])
    adapter = self.containers[ADAPTER]
    self.assertEqual((adapter["worker"], adapter["version"]), ("package://sandbox-builtin", "^1"))
    self.assertEqual(adapter["config_override"]["image_allowlist"], ["desktop", "anthropic"])
    self.assertEqual(adapter["config_override"]["custom_images"]["desktop"], "ghcr.io/trycua/linux:24.04-slim-20261013-36bc520")
    lines = (TEMPLATE / "worker-compose.yaml").read_text().splitlines()
    self.assertNotEqual(lines[lines.index(f"  {ADAPTER}:") - 1].strip(), "# added by compose::add")
```
Update the existing tests as well:
- Add `ADAPTER` to the expected container set at `:39`, and delete the `assertIn("iii-sandbox", …)` at `:41`.
- The `path://` loop at `:43` and the `env_file` loop at `:72` skip `ADAPTER`. `test_dependencies_are_declared_before_consumers` (`:58-68`) stays, so `harness.start_after` must list `ADAPTER`.
- In `test_sync.py` `test_local_workers_support_inline_compose_and_existing_start_commands`, add `self.local_worker("sandbox-builtin")`, the entry `adapter: {worker: package://sandbox-builtin}`, and `self.assertEqual(containers["adapter"]["worker"], "package://sandbox-builtin")`.
- [ ] **Step 2: Run them to verify they fail.** Run `python -m unittest discover -s template/tests -p test_template.py -v` and the same with `-p test_sync.py`. Expect `test_sandbox_adapter_is_operator_declared` to fail with `'workers' unexpectedly found`, and the sync test with `'path://../../sandbox-builtin' != 'package://sandbox-builtin'`.
- [ ] **Step 3: Implement.**
  - Move the `:8-17` block into a first container, `sandbox-builtin`, with `worker: package://sandbox-builtin`, `version: '^1'` and `config_override:` holding the old keys. Reword the comment (operator-declared, no marker), give it no `env_file`, and add `sandbox-builtin` to `harness.start_after`.
  - `sync_template.py:118-121`: skip `package://sandbox-builtin`, with a one-line comment: its release build embeds init and firmware, which `cargo run` cannot produce.
  - `template/README.md:55-59` says that sync keeps `sandbox-builtin` as a package. It also says the baseline `worker-compose.yaml` declares it, so `up` on a host with no adapter build fails with `UNSUPPORTED_PLATFORM`; remove that container there.
  - Computer: the README and Dockerfile point at `sandbox-builtin`'s `config_override`. Launcher `:198` says no sandbox adapter is serving `sandbox::*`: run `iii trigger compose::add worker=iii-sandbox`, then add a desktop image to sandbox-builtin's `custom_images`. `:199` names sandbox-builtin's `custom_images`. The `:194` headline stays (D13). `computer/src/config.rs:50-52` and `scalar.ts:149` say the OCI ref is set in sandbox-builtin's configuration instead of iii-sandbox's.
- [ ] **Step 4: Run the tests to verify they pass.** Run both unittest commands, `pnpm --dir computer/ui build`, `pnpm --dir ade/ui build` and `cargo test --locked --manifest-path computer/Cargo.toml`. Expect all OK.
- [ ] **Step 5: Commit.** Run `git add template/worker-compose.yaml template/tests template/scripts/sync_template.py template/README.md computer/README.md computer/images/desktop/Dockerfile computer/ui/src/page/Launcher.tsx computer/src/config.rs ade/ui/src/configuration-forms/specs/scalar.ts`, then `git commit -m "fix(template): declare sandbox-builtin instead of engine.workers.iii-sandbox"`.

### Task 8: Whole-plan verification and regression checks

**Files:** none changed.

- [ ] **Step 1: Run the static checks.** For each of `ide`, `sandbox-code-runner` and `harness`, run `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings` and `cargo test --locked --all-features` from that directory (harness: add `-p harness-e2e`). Then run `pytest .github/scripts/tests/ -v`, `actionlint -color`, and both template unittest commands. Expect everything green.
- [ ] **Step 2: Run the e2e suites.** Run `ide/tests/e2e/run-tests.sh` and `run-tests.sh --suite=jailed --no-build`; expect PASS. Run Task 4's isolation e2e and Task 5's gated e2e on a KVM host; expect both to pass.
- [ ] **Step 3: Run the spec 11 regression checks.** Run these on Apple Silicon macOS and on Linux x86_64 with KVM, with engine 0.25.0 and `command -v iii-worker` printing nothing. Start the stack with `cd template && iii compose --up`.
  - Check `iii trigger sandbox::adapter::info --json '{}'`. Expect `info.adapter == "sandbox-builtin"` and `info.contract.major == 1`.
  - computer: `computer::sessions::start {"image":"desktop"}`, then `computer::screenshot` and `computer::act {"action":"click","x":640,"y":360}` with its `session_id`. Then `compose::restart container=computer`, and confirm `computer::sessions::list` re-attaches the same session.
  - sandbox-code-runner: covered by Task 5. The worker log shows `sandbox adapter detected`. With the adapter stopped, it shows today's "NOT installed" warning.
  - ide: `shell::exec {"command":"echo","args":["ok"],"target":{"kind":"sandbox","sandbox_id":<from sandbox::create {"image":"desktop"}>}}` returns `stdout:"ok\n"`. `shell::fs::ls` on `/` with the same target returns entries.
  - Pickers: `sandbox::catalog::list` lists `desktop` and `anthropic`, and both the ADE and computer console pickers show them.
  - harness: `harness/target/release/harness-e2e run --url ws://127.0.0.1:49134 --scenario shell_coder_sandbox …` against the template stack scores the `worker_setup` and `sandbox_lifecycle` gates as passed.
  - Router smoke and error JSON: `sandbox::exec {"sandbox_id":"00000000-0000-0000-0000-000000000000","cmd":"true"}` fails with S002. Its message's `type`, `retryable` and `docs_url` anchor match the S002 row of `sandbox-builtin/tests/fixtures/sandbox_error_codes.json`.
- [ ] **Step 4: Prepare the PR note; do not push.** Create the MOT ticket first. Use the title `fix(sandbox): move workers-repo consumers onto engine 0.25 (MOT-<id>)`. The body links the spec, lists Tasks 1-7, records the results of Steps 1-3, and states Task 3's reading of D13 for `III_SANDBOX_FS_NAMESPACE`.
