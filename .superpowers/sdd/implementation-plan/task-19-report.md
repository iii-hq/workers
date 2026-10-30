# Task 19 implementation report

Status: DONE_WITH_CONCERNS. Base: bb6ecbbea. All required local gates passed on the final run; the previously observed intermittent gateway first-dial failure recurred once and remains unexplained. Native acceptance remains explicitly unverified by user scope.

## Scope and fixture

Local-only user verification scope. No privileged TUN, system service, hosts, mount, native Ubuntu/macOS/Windows, other-network, packet-capture or named Cloudflare tunnel acceptance has been performed or claimed.

The e2e suite uses real engine `/home/anderson/.local/bin/iii` (0.25.0-rc.6), current repository rbac-proxy, IDE, fp, harness, state, queue and session-manager binaries, production hub.Run, real in-process tsnet nodes and Go SDK fake echo workers. No engine/proxy/IDE protocol implementation is mocked. Each child connects to a test-only local WebSocket forwarder; every upstream connection is dialed through its own tsnet node, preserving the actual overlay source address used by gateway ownership and ticket authentication. Forwarding records actual frames for assertions and bootstrap evidence.

Signed IDE bootstrap runs the actual Node.Register signed handlers and supervisor. A test-only WorkerController wrapper translates only the authenticated runtime URL into that node's forwarder URL. The test prepopulates the supervisor cache with the current IDE binary under its actual SHA256 and mode 0555, retaining production digest/owner/mode checks. Only an exact registry /resolve request for ide@0.12.36 is served by a controlled HTTP transport response for that digest; artifact URL remains HTTPS and is never downloaded because cache verification succeeds. Production registry parsing, Hub signed reconciliation, default control namespace, supervised project namespace, process environment and operator execution remain in the path. The original test-global HTTP client is restored. Tests run sequentially. Node-owned IDE configuration is seeded through the actual default-namespace configuration worker, with confined temporary filesystem roots and turn storage.

## Concrete owning defect and TDD evidence

Project hub startup invoked `engine::functions::list` in the project namespace. Actual engine registered the builtin only in default, so hub.Run failed before registering mesh functions:

```
list engine function prefixes: iii: invocation of "engine::functions::list" failed: function_not_found: Function engine::functions::list not found in namespace project. It is registered in namespace(s): default.
```

RED: `PATH=/home/anderson/go/pkg/mod/golang.org/toolchain@v0.0.1-go1.27.1.linux-amd64/bin:$PATH III_BIN=/home/anderson/.local/bin/iii mesh/tests/e2e/run-tests.sh -run TestE2EBootstrapIDEProjectNamespace`, exit 1, `/tmp/mesh-task19-project-red.log` (1.007s).

Read-only engine source `/home/anderson/project/iii/iii/engine/src/workers/engine_fn/mod.rs` defines engine::functions::list as the global catalog with no namespace filter. Fixed hub.refreshNames RPC namespace to default while preserving project registration/control routing. Added assertion to existing hub.Run wire regression.

Initial GREEN: same runner `-run TestE2EBootstrapIDE` with production signed supervisor: default and project both PASS, package 17.177s, `/tmp/mesh-task19-supervised-bootstrap-green.log`. This initial pass used trusted engine invocation of remote shell; later final cases additionally run through the real harness path (results below).

## Real IDE observations

Both namespaces observe `configuration::ensure` and `configuration::get` for `laptop__ide` explicitly in default. SDK emits engine::workers::register with worker metadata namespace default/project. Configuration binding is `configuration` -> shell::on-config-change, configuration_id laptop__ide, event_types [configuration:updated]; binding target namespace follows the hub namespace. Hub signed mesh-node::workers::apply targets default (outbound wire regression) and carries the hub namespace in authenticated payload. Actual engine delivery omits the already-routed namespace, so the e2e asserts the delivered signature/payload and successful default-only registered control handler rather than inventing an inbound namespace field.

Optional UI bindings observed: console:module path ide/xterm.js, console:script path ide/page.js, console:style path ide/styles.css, all targeting ide::ui-content in the worker namespace. Optional configuration::get shell-ui raw:true remains refused as outside node-owned configuration. These console bindings remain refused pending Phase 3 confinement; no broad grant is added.

Observed request details retained for Task 25: `configuration::ensure` carries `id:laptop__ide`, `name:IDE`, `description:"Command denylist, timeout & output caps, and the fs jail."`, `metadata:{ui_form:ide}`, plus the current real IDE schema/default initial value (full payload in frame artifact). The two owned `get` calls carry `{id:laptop__ide}`; the optional UI reads carry `{id:shell-ui,raw:true}`. `engine::workers::register` has no envelope namespace and carries metadata `{name:mesh-node,namespace:default,runtime:go}` for control and `{name:ide,namespace:default|project,runtime:rust}` for the supervised IDE. Every observed configuration/Console trigger target follows the worker namespace.

Phase 1 default allow additions observed necessary: none. Existing confined default namespace configuration::{get,ensure,register,set} and own trigger types plus confined configuration binding suffice for remote shell bootstrap. Task 25 must use the exact observed console paths and preserve node-owned configuration confinement. Raw recorded frames go to ignored reports/ide-default-frames.json and ide-project-frames.json; they are runtime artifacts, not committed fixtures.

## Approval-gate current-engine fixture

Added supplied_engine_serves_real_state regression. Before repair:

`III_ENGINE_BIN=/home/anderson/.local/bin/iii cargo test --manifest-path approval-gate/Cargo.toml --locked --test engine_bootstrap -- --nocapture`

Exit 101, `/tmp/mesh-task19-testkit-red.log`: emitted `skipping: no iii engine...`, then `supplied engine must boot`; 0 passed, 1 failed, 32.04s. Task 18 captured the root cause: `[UNSUPPORTED_CONFIG_WORKERS] ... iii-state -> move to worker-compose.yaml as state`. Replaced unsupported in-engine state configuration with current external real state worker, captured child logs, early process-exit detection, explicit readiness assertions, process-group cleanup and unique per-engine directories. Optional absent engine alone can skip; supplied failed engine cannot. Added isolated subprocess regression using /bin/false. Real harness test fixture additionally starts actual queue, session-manager and harness workers with explicit readiness, seeds actual sessions, and retains all children for the whole async test stack lifetime. The intentionally isolated approval tests retain their existing fake harness/session dependencies; the real-harness branch has no replacement session implementation.

## Validation

All final local gates/results appear below. Heavy builds/tests ran sequentially.

Prerequisite builds completed: `cargo build --manifest-path ide/Cargo.toml --locked --bin ide` exit 0 (21.17s), corresponding state build exit 0 (12.26s), queue build exit 0 (19.55s). Build logs `/tmp/mesh-task19-build-{ide,state,queue}.log`.

First e2e iteration includes fixture corrections: ensure request requires description; NodeConfig Allow must be an empty array rather than null under inferred schema; proxy authentication sends an error frame then closes after WebSocket upgrade (HTTP101 alone is not authorization); fp::batch readiness requires a nonempty calls array. These are test expectations/fixture corrections, not production defects.


## Additional testkit evidence

Approval state bootstrap GREEN: `III_ENGINE_BIN=/home/anderson/.local/bin/iii III_STATE_BIN=$PWD/state/target/debug/state cargo test --manifest-path approval-gate/Cargo.toml --locked --test engine_bootstrap -- --nocapture`, exit 0, 2/2 tests, 2.16s, `/tmp/mesh-task19-testkit-green.log`. Includes actual external-state write/read and isolated supplied `/bin/false` failure assertion.

Enabling the formerly skipped cross-worker harness bodies exposed stale fixture assumptions. `/tmp/mesh-task19-approval-harness.log` failed 2/3 because Rust async disjoint field capture dropped the real worker-owner field before execution; making TestStack a Drop owner retains children through the future. Next run reached current turn parsing and rejected missing created_at (now seeds both created_at/updated_at). Third run passed 2/3 and reached real deny delivery, which required session::append; the real-harness branch now starts real session-manager and seeds via session::ensure. Final focused command:

```
III_ENGINE_BIN=/home/anderson/.local/bin/iii III_STATE_BIN=$PWD/state/target/debug/state III_QUEUE_BIN=$PWD/queue/target/debug/queue III_HARNESS_BIN=$PWD/harness/target/debug/harness III_SESSION_MANAGER_BIN=$PWD/session-manager/target/debug/session-manager cargo test --manifest-path approval-gate/Cargo.toml --locked --test harness_integration -- --nocapture
```

Exit 0, 3/3 tests, 7.63s, `/tmp/mesh-task19-approval-harness4.log`: held call survives sweep, pre-trigger hold then allow resolve, deny delivery through real harness. These are fixture compatibility/lifetime fixes; product harness/approval semantics are unchanged.

`rbac-proxy/tests/integration.rs` had the same confirmed supplied-engine false-skip. RED: `III_BIN=/home/anderson/.local/bin/iii cargo test --manifest-path rbac-proxy/Cargo.toml --locked --test integration supplied_failed_engine_cannot_skip -- --nocapture`, exit 101, 15.12s, `/tmp/mesh-task19-proxy-testkit-red.log`: an isolated child with III_BIN=/bin/false reported the integration successful, and the new regression failed `failed engine passed integration`. Repair limits skip to absent optional engine; supplied engine/proxy startup failure and busy test ports fail with captured logs. Test config lives within the cleaned isolated directory. Negative regression requires the supplied-engine diagnostic, not arbitrary subprocess failure. GREEN is recorded with the final proxy gate below.

Harness compact/read_only existing helpers were inspected for the named current-engine boot failure risk: their configs do not use retired iii-state; supplied-engine readiness failure already panics. They were left unchanged. Final gates supply the actual engine and use --nocapture so skip output is visible.

## E2E fixture corrections during RED iterations

The first full iteration (`/tmp/mesh-task19-e2e-first.log`, exit 1, 155.275s) passed 10/14 and exposed four fixture issues: ticketless proxy uses HTTP101 plus AUTH error frame/close; fp batch readiness must be a valid nonempty request; pipe refusal is an RPC error while batch carries item error; actual harness ancestry requires session-manager. Further focused runs corrected TurnOptions.max_turns, captured only a nonempty signed apply envelope for replay, and verified configuration acceptance through the actual registered-trigger catalog/event delivery because successful binding has no success acknowledgement. `/tmp/mesh-task19-e2e-harness2.log` proves fs_scope direct/pipe/batch, actual hook refusal, signed replay and callback cases; both harness bootstrap calls succeeded before the invalid inbound-namespace assertion was corrected. No production policy was weakened to satisfy these tests. Config writes in the final suite use configuration::set and real configuration event delivery, without invoking the reload handler manually.

## CI shape

`.github/workflows/mesh-e2e.yml` runs on PR changes to mesh, rbac-proxy and the real worker dependencies, plus manual dispatch. Pinned Go 1.27.1/Rust 1.97.1; engine installed through the existing iii installer with version printed; seven real worker binaries built sequentially under --locked; runner requires all executables and invokes tagged e2e with 15 minute test timeout. No sudo, native interface/hosts/services or external tunnel. Each test uses isolated temp state and process-group cleanup. Child logs, full Go results and actual IDE frames upload as artifacts even on failure. Workflow itself has been source-reviewed and shell runner syntax checked; remote GitHub execution is not claimed.

## Final new mesh e2e result

Command: `PATH=/home/anderson/go/pkg/mod/golang.org/toolchain@v0.0.1-go1.27.1.linux-amd64/bin:$PATH III_BIN=/home/anderson/.local/bin/iii mesh/tests/e2e/run-tests.sh` (runner sets CGO_ENABLED=0 GOWORK=off GOFLAGS=-mod=readonly GOTOOLCHAIN=local TS_NO_LOGS_NO_SUPPORT=true).

Exit 0, 14/14 named cases plus all three fs_scope subcases, package 116.729s. Log `/tmp/mesh-task19-e2e-final.log`. Each case boots an actual fresh engine/proxy/hub; all test bodies executed. Required actual IDE/harness/fp paths are exercised as described, not inferred from unit successes.

| Case | Result / seconds |
|---|---|
| `TestE2ETemplatesRefused` | PASS 7.79s |
| `TestE2EMiddlewareNeedsSessionKey` | PASS 4.60s |
| `TestE2ELoopbackWithoutTicket` | PASS 4.20s |
| `TestE2ENodeRegistersMeshIDGetsPrefix` | PASS 7.38s |
| `TestE2ERevocationWithin5s` | PASS 8.24s |
| `TestE2ESupervisorUnreachable` | PASS 8.16s |
| `TestE2EConditionCallbackRefused` | PASS 7.82s |
| `TestE2ENameSquatting` | PASS 10.56s |
| `TestE2ETwoNodesSameWorkerName` | PASS 10.83s |
| `TestE2EBootstrapIDEDefaultNamespace` | PASS 9.19s |
| `TestE2EBootstrapIDEProjectNamespace` | PASS 9.28s |
| `TestE2EFsScopeOnRemoteCoder` | PASS 9.49s |
| `TestE2EHooksRefused` | PASS 7.86s |
| `TestE2ERefusedRegistrationLogged` | PASS 11.31s |

Revocation from an already-connected node was enforced in 1.179683234s after real configuration::set/event reload; repeated refusal did not increment the target counter. Condition callback test refuses both an outside-grant callback and a raw own-function callback key; a separate confined positive configuration binding receives a real update event while forbidden callback count remains zero. Mesh hook is the first integrated defense, so this is mesh-boundary evidence; independent callback logic is covered by the five Rust interceptor tests listed below; other real proxy boundaries are covered by its existing e2e. The proxy callback defense is not claimed to execute after mesh already refuses the same binding.

Remote coder allowed reads succeed within node host_roots and outside reads return C215 without exposing the outside marker in direct/pipe/batch. Actual delivered remote frames contain no model/hub fs_scope. Real harness pre/post hook refusals precede a successful target call with zero malicious hook callbacks.

SDK refusal case observes actual console triggerregistrationresult errors, reads real supervised IDE refusal logs, and observes unchanged accepted-forwarder connection count over 2.3 s (past the SDK 2 s reconnect interval), then successfully calls shell. This is a measured absence of reconnect within that interval, not an unlimited-lifetime claim.

## Prior convergence observations

Task 18 had intermittent TestLiveGatewayOwnershipLifecycle/TestHubRestartKeepsNodes failures while another heavy Rust suite ran; contention was an observation, not a proven cause. Its subsequent sequential full Go gate passed 15 packages (hub 42.759s). Those failures remain recorded in Task 18's report. This task keeps full Rust/Go/e2e suites sequential, preserves all existing deadlines, and makes no claim that a pass repairs a prior intermittent failure.

## Full Phase 1 Rust gates

Ran sequentially from worktree root under Rust 1.97.1:

```sh
export III_ENGINE_BIN=/home/anderson/.local/bin/iii III_BIN=/home/anderson/.local/bin/iii
export III_STATE_BIN=$PWD/state/target/debug/state III_QUEUE_BIN=$PWD/queue/target/debug/queue
export III_HARNESS_BIN=$PWD/harness/target/debug/harness III_SESSION_MANAGER_BIN=$PWD/session-manager/target/debug/session-manager
for task_worker in harness fp rbac-proxy approval-gate; do
  cargo fmt --manifest-path "$task_worker/Cargo.toml" --all -- --check
  cargo clippy --manifest-path "$task_worker/Cargo.toml" --locked --all-targets --all-features -- -D warnings
  cargo test --manifest-path "$task_worker/Cargo.toml" --locked --all-features -- --nocapture
done
```

Exit 0, all four fmt/clippy/test gates PASS. Logs `/tmp/mesh-task19-{worker}-{fmt,clippy,test}.log`, sequence `/tmp/mesh-task19-rust-gates.log`; exact executable script `/tmp/mesh-task19-rust-gates.sh`. The script redirects each stage to the listed file and aborts on any failure. Per-target counts below include all binary, integration and doc targets rather than only library tests.

### harness: 899 passed

| Target | Passed | Failed | Ignored | Time |
|---|---:|---:|---:|---|
| `unit src/lib.rs` | 814 | 0 | 0 | 0.47s |
| `unit src/main.rs` | 4 | 0 | 0 | 0.00s |
| `tests/compact_command.rs` | 6 | 0 | 0 | 7.78s |
| `tests/delete_session_tree.rs` | 53 | 0 | 0 | 0.54s |
| `tests/event_driven.rs` | 5 | 0 | 0 | 0.42s |
| `tests/manifest.rs` | 2 | 0 | 0 | 0.01s |
| `tests/pre_generate_budget.rs` | 1 | 0 | 0 | 0.10s |
| `tests/prompts.rs` | 6 | 0 | 0 | 0.00s |
| `tests/read_only.rs` | 1 | 0 | 0 | 2.08s |
| `tests/schemas.rs` | 7 | 0 | 0 | 0.02s |
| `Doc-tests harness` | 0 | 0 | 0 | 0.00s |

### fp: 72 passed

| Target | Passed | Failed | Ignored | Time |
|---|---:|---:|---:|---|
| `unit src/lib.rs` | 67 | 0 | 0 | 0.00s |
| `unit src/main.rs` | 0 | 0 | 0 | 0.00s |
| `tests/configuration_identity.rs` | 1 | 0 | 1 | 0.01s |
| `tests/contract.rs` | 4 | 0 | 0 | 0.00s |
| `Doc-tests fp` | 0 | 0 | 0 | 0.00s |

### rbac-proxy: 129 passed

| Target | Passed | Failed | Ignored | Time |
|---|---:|---:|---:|---|
| `unit src/lib.rs` | 121 | 0 | 0 | 0.25s |
| `unit src/main.rs` | 0 | 0 | 0 | 0.00s |
| `tests/integration.rs` | 2 | 0 | 0 | 16.76s |
| `tests/manifest.rs` | 1 | 0 | 0 | 0.00s |
| `tests/observe_protocol.rs` | 2 | 0 | 0 | 0.02s |
| `tests/schemas.rs` | 3 | 0 | 0 | 0.00s |
| `Doc-tests rbac_proxy` | 0 | 0 | 0 | 0.00s |

### approval-gate: 201 passed

| Target | Passed | Failed | Ignored | Time |
|---|---:|---:|---:|---|
| `unit src/lib.rs` | 164 | 0 | 0 | 116.88s |
| `unit src/main.rs` | 0 | 0 | 0 | 0.00s |
| `tests/contract_parity.rs` | 3 | 0 | 0 | 0.00s |
| `tests/engine_bootstrap.rs` | 2 | 0 | 0 | 2.13s |
| `tests/harness_integration.rs` | 3 | 0 | 0 | 7.52s |
| `tests/integration.rs` | 5 | 0 | 0 | 11.52s |
| `tests/repository_permissions.rs` | 18 | 0 | 0 | 0.04s |
| `tests/schemas.rs` | 6 | 0 | 0 | 0.00s |
| `Doc-tests approval_gate` | 0 | 0 | 0 | 0.00s |

No `skipping:` output occurred in any full Rust gate. Harness compact_command 6 tests/read_only 1 test used the actual supplied engine; they are existing tests with purpose-specific stub dependencies, not claims of a real LLM. fp's one ignored entry is the deliberately ignored configuration_identity_child subprocess fixture invoked by its passing parent test. The engine-backed subset of approval-gate’s 164 library tests now executes its engine/state bodies rather than Task 18's compatibility self-skips. Its engine_bootstrap 2 tests and real harness 3 tests also passed in the full run (2.13s/7.52s). Proxy integration 2 tests passed 16.76s: existing real engine/proxy behavioral body plus the isolated supplied `/bin/false` cannot-skip regression, providing its RED/GREEN.

Independent proxy condition-defense tests executed in the 121 library tests: `condition_function_outside_grants_is_denied`, `own_condition_function_is_prefixed`, `allowed_condition_function_passes`, `condition_function_grants_are_namespace_scoped`, `own_condition_function_is_own_only_in_declared_namespace`. Confined trigger-type grant tests also passed, including own exact/wildcard grants, refusal of unrelated/unregistered types, and registration result unprefixing. These are proxy interceptor tests; the mesh e2e exercises mesh's earlier integrated refusal.

## Existing real proxy e2e

The default engine port49134 belongs to an unrelated running service and was left untouched. The existing runner places state relative to its own directory, so copied its run-tests.sh/config.yaml and harness sources into `/tmp/mesh-task19-proxy-e2e.14lzeivo`, with existing node_modules linked (the runner saw dependencies present and did not install). Verified runner/config/source bytes against originals before running. Fresh data was created only under that temporary suite directory. Command from worktree root:

```sh
III_BIN=/home/anderson/.local/bin/iii WORKER_SRC=$PWD/rbac-proxy WORKER_BIN_TARGET=$PWD/rbac-proxy/target/debug/rbac-proxy ENGINE_PORT=49834 PROXY_PORT=49871 /tmp/mesh-task19-proxy-e2e.14lzeivo/run-tests.sh --no-build
```

Exit 0, `HARNESS_DONE: PASS 9/9`, `/tmp/mesh-task19-proxy-e2e.log`; full engine/proxy/harness logs and JSON report remain in the temporary suite's reports directory. Cases: exposed-call-through-middleware; forbidden-call-rejected; discovery-functions-filtered; discovery-workers-internals-stripped; prefix-applied-and-hook-ran-on-registration; prefix-self-invoke-roundtrip; trigger-to-forbidden-denied; channel-roundtrip-through-proxy; oversized-call-rejected-without-poisoning. This suite confirms generic target refusal and prefixed registration against the real current engine/proxy, but has no embedded condition_function_id case; callback-specific independent coverage is the five Rust tests listed above.

## Full mesh Go gate and recurring observation

Pinned Go 1.27.1 linux/amd64, exact script `/tmp/mesh-task19-go-gates.sh`:

```sh
export PATH=/home/anderson/go/pkg/mod/golang.org/toolchain@v0.0.1-go1.27.1.linux-amd64/bin:$PATH
export CGO_ENABLED=0 GOWORK=off GOFLAGS=-mod=readonly GOTOOLCHAIN=local
test -z "$(gofmt -l mesh)"
go -C mesh vet ./...
go -C mesh test ./... -count=1
make -C mesh build
```

First run exit 1, `/tmp/mesh-task19-go-gates.log`: fmt/vet passed; TestLiveGatewayOwnershipLifecycle failed in 9.03s at gateway_test.go:76 with `unowned connection: <nil> failed to WebSocket dial: failed to send handshake request: Get "http://[fdc2:6491:3904::1]:50331/": context deadline exceeded`. Hub package 42.286s; all other 14 packages passed; build did not execute after test failure. Here “unowned” means the intentionally inactive gateway should return 503, not an unknown node identity. This is the same named intermittent observation from Task 18, now reproduced with heavy suites sequential. Thus heavy-suite overlap is not an established explanation.

Systematic investigation: read the pinned tailscale.com v1.102.4 tsnet Up/Dial implementation (Up waits for Running+assigned IP; Dial also only awaits Running), existing gateway fixture, coordinator map updates and hub userspace startup. First overlay HTTP request follows node.Up immediately with 3s client timeout; data-path readiness is a plausible risk, not a demonstrated cause. Focused unchanged reproduction:

```sh
PATH=/home/anderson/go/pkg/mod/golang.org/toolchain@v0.0.1-go1.27.1.linux-amd64/bin:$PATH CGO_ENABLED=0 GOWORK=off GOFLAGS=-mod=readonly GOTOOLCHAIN=local go -C mesh test ./internal/hub -run '^TestLiveGatewayOwnershipLifecycle$' -count=3 -v
```

Exit 0, 3/3 PASS, 28.112s, `/tmp/mesh-task19-gateway-repro.log`. Added only failure-visible tsnet transport logs, initial dial elapsed time and peer online/active/Rx/Tx/last-handshake status to that test. No readiness retry, timing allowance, revocation deadline or product network behavior was changed. No root cause was established or claimed fixed.

Final complete gate with diagnostics: exit 0, `/tmp/mesh-task19-go-diagnostic.log`. fmt/vet silent; all 15 packages passed and make build completed:

```
ok  	github.com/iii-hq/workers/mesh	0.005s
ok  	github.com/iii-hq/workers/mesh/install	0.103s
ok  	github.com/iii-hq/workers/mesh/internal/cli	0.015s
ok  	github.com/iii-hq/workers/mesh/internal/coordinator	12.625s
ok  	github.com/iii-hq/workers/mesh/internal/elevate	0.003s
ok  	github.com/iii-hq/workers/mesh/internal/gateway	0.008s
ok  	github.com/iii-hq/workers/mesh/internal/hub	42.670s
ok  	github.com/iii-hq/workers/mesh/internal/integration	18.748s
ok  	github.com/iii-hq/workers/mesh/internal/join	0.024s
ok  	github.com/iii-hq/workers/mesh/internal/meshnet	6.843s
ok  	github.com/iii-hq/workers/mesh/internal/node	0.111s
ok  	github.com/iii-hq/workers/mesh/internal/registry	0.007s
ok  	github.com/iii-hq/workers/mesh/internal/relay	0.023s
ok  	github.com/iii-hq/workers/mesh/internal/service	0.003s
ok  	github.com/iii-hq/workers/mesh/internal/supervisor	3.431s
make: Entering directory '/home/anderson/project/iii/workers/.claude/worktrees/mesh-worker-spec/mesh'
mkdir -p bin/
CGO_ENABLED=0 GOWORK=off go build -mod=readonly -trimpath -ldflags="-s -w -X main.version=0.1.0" -o bin/mesh .
make: Leaving directory '/home/anderson/project/iii/workers/.claude/worktrees/mesh-worker-spec/mesh'
```

The refreshed engine discovery namespace assertion is covered by this complete hub gate. This passing run satisfies the local gate while preserving the unresolved intermittent observation for follow-up.

## Self-review and final limits

Reviewed every changed production/test/CI file. The only product behavior change is global engine catalog discovery routed to default; project-owned mesh/proxy calls and worker registrations remain in project. Testkit changes fix actual current-engine compatibility and silent success on supplied-engine failure. New e2e keeps real overlay identity, CPace enrollment, gateway tickets, Rust proxy, signed node handlers, supervisor/cache digest checks, actual IDE/harness/fp/state/queue/session-manager and fake Go echo workers in their intended roles. All test-global seams are restored and child processes/temp configuration are owned by fixture cleanup; no protocol mock replaces the required real paths. No broad default permission grant was added. Production mirrored filesystem predicates were untouched. Final whitespace/shell checks passed; a final JSON-macro indentation cleanup was formatting-only and cargo fmt --check was rerun.

Native Ubuntu 26.04 VM/macOS/Windows, cross-network, privileged TUN/hosts/services/mounts, packet captures, named Cloudflare tunnel and live registry artifact publication/installation remain unverified. The cache/resolve fixture does not prove live registry publication. Remote GitHub CI has not run. The measured SDK no-reconnect interval is 2.3 s only. The recurring initial gateway handshake timeout remains a concern, with diagnostics retained and no claimed repair.

No push, native setup, unrelated-service change or controller-ledger edit performed.

Commit subject: `test(mesh): end-to-end and adversarial suite`.

## Fix round 1 — review base 9914b0c13

Addressed the four Important findings in task-19-fix-1.md. Changes are confined to CI and test fixtures/assertions; no additional product or Rust source change was needed. The receiving-code-review, TDD and verification workflows were applied. The condition-callback ruling is corrected: mesh.md:722 requires actual Mesh **and** independent proxy refusal, so the earlier five proxy unit cases plus integrated Mesh refusal did not complete that requirement. The new real proxy case below supplements them and leaves the integrated Mesh hook intact.

### Worker identity and absolute revocation completion

`mesh/tests/e2e/adversarial_test.go`: each fake worker's handler now returns its fixture-owned node name, ignoring caller data. Both `TestE2ENameSquatting` and `TestE2ETwoNodesSameWorkerName` call both final routes with identical `{"probe":"same-payload"}` data after both workers exist, decode the reply, and require the respective node identity. Metadata checks remain. RED ran these new assertions against the old payload-echo handler:

```sh
PATH=/home/anderson/go/pkg/mod/golang.org/toolchain@v0.0.1-go1.27.1.linux-amd64/bin:$PATH III_BIN=/home/anderson/.local/bin/iii mesh/tests/e2e/run-tests.sh -run 'TestE2E(NameSquatting|TwoNodesSameWorkerName)$'
```

Exit 1, both cases failed, package 21.068s; `/tmp/mesh-task19-fix1-identity-red.log`. Exact failures include `displaced evil: {"_caller_worker_id":"9254d3a1-20c9-49a4-b42a-94c71993bd89","probe":"same-payload"}` and `echo displaced: {"_caller_worker_id":"1e9ebd4b-3811-4ad3-90f0-20a32248bd42","probe":"same-payload"}`. Thus the new checks cannot pass by reflecting the caller's claimed identity.

`TestE2ERevocationWithin5s` now starts the same five-second clock before the policy update, applies its absolute deadline to each RPC, and checks refusal completion before that deadline. `mesh/tests/e2e/revocation_test.go` contains the bounded polling helper and four focused cases. Only an actual `*iii.InvocationError` with code `FORBIDDEN` counts; elapsed deadlines, context cancellation, and transport messages containing “forbidden” do not. The post-refusal target counter assertion remains.

The extracted old before-call/string-matching behavior was tested before fixing it:

```sh
PATH=/home/anderson/go/pkg/mod/golang.org/toolchain@v0.0.1-go1.27.1.linux-amd64/bin:$PATH CGO_ENABLED=0 GOWORK=off GOFLAGS=-mod=readonly GOTOOLCHAIN=local go -C mesh test -tags=e2e ./tests/e2e -run '^TestPolicyRefusalDeadline$' -count=1 -v
```

RED exit 1, 0.013s, `/tmp/mesh-task19-fix1-deadline-red.log`: `before-deadline` passed; `late-policy-response`, `cancellation-is-not-policy`, and `message-is-not-policy-code` each failed because the helper incorrectly returned `<nil>`. After the fix, this combined focused command passed:

```sh
PATH=/home/anderson/go/pkg/mod/golang.org/toolchain@v0.0.1-go1.27.1.linux-amd64/bin:$PATH III_BIN=/home/anderson/.local/bin/iii mesh/tests/e2e/run-tests.sh -run 'TestPolicyRefusalDeadline$|TestE2E(NameSquatting|TwoNodesSameWorkerName|RevocationWithin5s|ConditionCallbackRefused)$'
```

Exit 0, 42.007s, `/tmp/mesh-task19-fix1-mesh-green.log`: four actual integration cases plus all four deadline subcases passed. Actual revocation refusal completed in **1.153067148s**. The integrated `TestE2EConditionCallbackRefused` still exercises the real Mesh registration hook and proves its forbidden callback never executes.

### Independent actual proxy condition defense

Added `condition-callback-outside-grants-denied` to `rbac-proxy/tests/e2e/workers/harness/src/cases.ts`. It asserts the live proxy configuration has no trigger-registration hook, registers real allowed/forbidden callbacks and an exposed event target on the engine, creates a real configuration entry, and sends the forbidden conditional binding through an authenticated proxy connection. It requires a `REGISTRATION_DENIED` frame naming the forbidden callback. It checks the actual engine binding registry for absence of that callback and presence of the allowed conditional binding, updates the real configuration, then requires both the allowed condition and its target to execute while the forbidden counter stays zero. It unregisters its functions and allowed binding on completion.

Used fresh temporary copies of the existing runner/config/harness to isolate its relative data directory and avoid the unrelated default-port service. Normal runner/config/source bytes were verified against the worktree; the existing harness node_modules was linked. The only controlled RED mutation, in `/tmp/mesh-task19-fix1-proxy-red.srap5dwp/run-tests.sh`, added `match("secret::condition-callback")` to that temporary seed's expose list. No production policy or repository runner was broadened. With that deliberately granted callback, the new case failed after 5012ms with `no condition binding refusal`; `HARNESS_DONE: FAIL 0/1`, exit 1, `/tmp/mesh-task19-fix1-proxy-red.log`. This is test-sensitivity evidence against an over-grant, not a newly discovered proxy product defect.

```sh
env WORKER_SRC="$PWD/rbac-proxy" WORKER_BIN_TARGET="$PWD/rbac-proxy/target/debug/rbac-proxy" III_BIN=/home/anderson/.local/bin/iii ENGINE_PORT=49834 PROXY_PORT=49871 /tmp/mesh-task19-fix1-proxy-red.srap5dwp/run-tests.sh --no-build --filter=condition-callback
env WORKER_SRC="$PWD/rbac-proxy" WORKER_BIN_TARGET="$PWD/rbac-proxy/target/debug/rbac-proxy" III_BIN=/home/anderson/.local/bin/iii ENGINE_PORT=49834 PROXY_PORT=49871 /tmp/mesh-task19-fix1-proxy-green._qby4tbs/run-tests.sh --no-build --filter=condition-callback
env WORKER_SRC="$PWD/rbac-proxy" WORKER_BIN_TARGET="$PWD/rbac-proxy/target/debug/rbac-proxy" III_BIN=/home/anderson/.local/bin/iii ENGINE_PORT=49834 PROXY_PORT=49871 /tmp/mesh-task19-fix1-proxy-full.w9j_m_3b/run-tests.sh --no-build
```

Unmodified-policy focused GREEN: exit 0, `PASS 1/1`, `/tmp/mesh-task19-fix1-proxy-green.log`. Final whole proxy suite: exit 0, **PASS 10/10**, `/tmp/mesh-task19-fix1-proxy-full.log`. It includes the original nine named cases above and the new condition case (109ms). Exact new evidence in its harness log: `[condition] independent refusal=REGISTRATION_DENIED; allowed callbacks=1; deliveries=1; forbidden callbacks=0`. Per-case JSON and actual engine/proxy/harness logs are under the temporary suite's `reports/`. Timeout waiting for a refusal fails the case; it is never interpreted as refusal.

### CI clean frontend prerequisites

`.github/workflows/mesh-e2e.yml` now uses the repository's existing pinned pnpm/setup-node action revisions, Node 22 and root `packageManager` pnpm 11.13.1, installs frozen dependencies for the three required UI projects plus their workspace dependencies, and builds IDE/harness/state assets before Rust workers. State's identical missing-bundle dependency is included. Relevant shared package/root lockfile/workspace paths now trigger the workflow.

Created a clean archive of tracked FIX_BASE sources at `/tmp/mesh-task19-fix1-clean.ux_3h7y2` with `git archive HEAD | tar -x`. Asserted no `ui/dist` or `ui/node_modules` existed in IDE, harness or state. Compiled each unchanged actual build script with `rustc --edition=2024 <clean>/<worker>/build.rs -o /tmp/mesh-task19-fix1-<worker>-build`, supplying `CARGO_MANIFEST_DIR=<clean>/<worker>` and `TARGET=x86_64-unknown-linux-gnu`. Ran each with `PATH=/usr/bin:/bin`, `PNPM` and `SKIP_UI_BUILD` unset. All three reproduced exit **101**, `pnpm not found on PATH`, in `/tmp/mesh-task19-fix1-prereq-red.log`.

Installed a temporary Node 22/pnpm toolchain only under `/tmp/mesh-task19-fix1-node22` (`npm install --prefix /tmp/mesh-task19-fix1-node22 --no-audit --no-fund node@22`, then the same with `pnpm@11.13.1`). Actual versions: **Node v22.23.4**, **pnpm 11.13.1**. From the clean archive, with that temporary `node_modules/.bin` first on PATH, executed the exact workflow preparation:

```sh
pnpm --filter '@iii-workers/shell-ui...' --filter '@iii-workers/harness-ui...' --filter '@iii-workers/state-ui...' install --frozen-lockfile
for directory in ide/ui harness/ui state/ui; do
  pnpm --dir "$directory" build
done
```

Both exited 0. Install selected 5/52 workspace projects, used the existing content-addressed pnpm cache for 173 packages, ran esbuild postinstall successfully, and did not change the root lockfile (byte comparison passed); `/tmp/mesh-task19-fix1-prereq-install.log`. Each build ran `tsc --noEmit && node build.mjs` and reported `lint: 0 errors, 0 warnings (strict: on)`. Generated asset bytes: IDE page.js 773966/styles.css 108613/xterm.js 345444; harness page.js 41866/styles.css 17357; state page.js 23783/styles.css 5515. Running the same three compiled build scripts again with pnpm absent and no bypass now returned **0/0/0**; `/tmp/mesh-task19-fix1-prereq-green.log`. This validates clean source/dependency/bundle prerequisites and the actual build-script acceptance path. It does not claim a new full clean Cargo build or remote GitHub job execution.

### Final scoped verification and self-review

Because the fake-worker helper is shared, reran all 14 actual Mesh integration cases plus the new deadline regression after the focused changes. Real engine and all seven built workers are the same validated local binaries described above. Heavy checks were sequential; no unrelated full Rust/full Go gate was repeated for these test/CI-only changes.

```sh
PATH=/home/anderson/go/pkg/mod/golang.org/toolchain@v0.0.1-go1.27.1.linux-amd64/bin:$PATH III_BIN=/home/anderson/.local/bin/iii mesh/tests/e2e/run-tests.sh
```

Exit **0**, package **115.661s**, `/tmp/mesh-task19-fix1-e2e-full.log`, no skipped cases. Complete top-level results:

```
TestE2ETemplatesRefused                    PASS  7.44s
TestE2EMiddlewareNeedsSessionKey           PASS  4.15s
TestE2ELoopbackWithoutTicket               PASS  4.63s
TestE2ENodeRegistersMeshIDGetsPrefix        PASS  7.78s
TestE2ERevocationWithin5s                  PASS  8.19s
TestE2ESupervisorUnreachable               PASS  7.92s
TestE2EConditionCallbackRefused            PASS  7.56s
TestE2ENameSquatting                       PASS 10.38s
TestE2ETwoNodesSameWorkerName              PASS 10.89s
TestE2EBootstrapIDEDefaultNamespace        PASS  9.06s
TestE2EBootstrapIDEProjectNamespace        PASS  9.26s
TestE2EFsScopeOnRemoteCoder                PASS  9.17s
TestE2EHooksRefused                        PASS  7.98s
TestE2ERefusedRegistrationLogged           PASS 11.21s
TestPolicyRefusalDeadline                  PASS  0.02s
```

The filesystem direct/pipe/batch subcases and all four deadline subcases passed. Actual policy refusal completed in **1.118041354s**, before its absolute five-second deadline. Default/project bootstrap again recorded the actual configuration RPCs in default and configuration/Console trigger registrations in the worker namespace; their complete observed frames are in this final log. No default grants were broadened. Independent real proxy coverage is the separate 10/10 suite above, while the Mesh case continues to exercise its earlier hook.

Final syntax/format verification, all exit 0:

```sh
export PATH=/home/anderson/go/pkg/mod/golang.org/toolchain@v0.0.1-go1.27.1.linux-amd64/bin:$PATH
export CGO_ENABLED=0 GOWORK=off GOFLAGS=-mod=readonly GOTOOLCHAIN=local
test -z "$(gofmt -l mesh/tests/e2e)"
go -C mesh vet -tags=e2e ./tests/e2e
npm --prefix rbac-proxy/tests/e2e/workers/harness run build
bash -n mesh/tests/e2e/run-tests.sh rbac-proxy/tests/e2e/run-tests.sh
git diff --check
```

TypeScript output (`tsc --noEmit`) is in `/tmp/mesh-task19-fix1-proxy-types-final.log`. PyYAML parsed the workflow; each embedded `run` block passed `bash -n -c`, and the frontend-before-worker step-order assertion passed. The existing rbac-proxy E2E workflow already selects `rbac-proxy/**` and invokes the runner, so the new case is also part of that workflow's next run.

Self-review checked all four findings against their covering code and output: caller-controlled identities are gone; deadline completion and typed policy refusal are enforced; the independent proxy test cannot short-circuit through Mesh; and clean assets are built with frozen dependencies before Cargo. No new product behavior or permission grants changed. The previously documented native/live-registry/remote-CI limits and unresolved initial gateway handshake timeout remain; no repair of that intermittent observation is claimed. Expected fake-node status/relay diagnostics remain visible as noted in the review's Minor finding. No push, unrelated-service modification, privileged setup or controller-ledger edit occurred.

Fix commit subject: `test(mesh): close phase gate review gaps`.
