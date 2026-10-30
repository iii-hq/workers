# Local Mesh delivery evidence

This records the local delivery preparation before the controller's final whole-branch review. Publication and that review remain separate gates. The user explicitly chose local tests and left native acceptance unverified.

The branch was clean at `99f65195eb6d467538d7c6318973e75501ec2b08`. Fetch advanced `origin/main` to `60dc4b4ad7efdc4215d6c4f0ccadc7b3447d85e3`; all 48 branch commits rebased without conflict to `2a01cdf5a7448b2af56a6a358b5d44cb7d78963f`. The old merge base was `e466d21d659297c877127f9055ac8a282944562d`; the new merge base is the fetched main commit. A local recovery ref retains the original tip. The final compatibility correction is `9d194cef7`.

## Phase and commit map

Ranges below are inclusive and refer to the rebased history.

| Phase | Commits | Delivered behavior |
| --- | --- | --- |
| Design and plan | `f92d45bdf`–`75af0ebaf` | Design, adversarial corrections and ordered single-PR implementation plan |
| 0: prerequisites | `f98b964d4`–`b50ccba3f` | Harness/fp filesystem scope confinement; proxy trigger prefixing, worker identity and condition-call confinement |
| 1: core | `1fbc1660f`–`40f0cfa6e` | Upstream Tailscale hub, coordinator, relay, CPace join, shim, ticket gateway, proxy ownership, signed node control, verified Resolve/cache/update, operator supervisor, Linux/macOS installation code, permissions, skill, actual-engine adversarial suites and review fixes |
| 2: reach | `201ada531`–`801c79672` | Windows nodes and review corrections; diagnosed restart upgrade retry and gateway test transport readiness; shared drives with ownership correction; Console and opaque-value preservation |
| 3: automation | `8c0f1fe61`–`2a01cdf5a` | Single-use/ephemeral tokens and accepted-key correction; node-change events and stale-unregister correction; observed confined defaults, browser/computer/tmux integrations, tmux UTF-8 mode and connection-scoped observation evidence |
| Delivery compatibility | `9d194cef7` | Remove tmux's obsolete confirmation `details` option after rebasing onto the current Console API |

## Rebase compatibility diagnosis

The frozen frontend preparation initially exited 2 at tmux's TypeScript check: `TS2353`, because `details` is absent from `ConfirmOptions`. Upstream `c15a1990f` explicitly removed that property from both `ConfirmDialogProps` and `ConfirmOptions`, defining a title, short description and actions. The correction removes the obsolete entry; the existing named session title, consequence description, danger tone and confirmation action remain. The affected `pnpm --dir tmux/ui build` then exited 0: typecheck, bundle and strict UI lint all passed, with zero lint errors or warnings. No timeout, dependency version or shared UI contract was changed.

## Pinned local gates

Go **1.27.1**, Rust/cargo **1.97.1**, Node **22.23.4** and pnpm **11.13.1** were used. Go commands set `CGO_ENABLED=0 GOWORK=off GOFLAGS=-mod=readonly GOTOOLCHAIN=local TS_NO_LOGS_NO_SUPPORT=true`. Installer tests also had portable PowerShell **7.4.13** available. The frozen frontend install selected 12 workspace projects without changing the lockfile; all eight required frontend builds completed, with the diagnosed tmux correction above. Generated Mesh assets stayed byte-identical.

```sh
test -z "$(gofmt -l mesh)"
go -C mesh vet ./...
go -C mesh test ./... -count=1
make -C mesh build
go -C mesh vet -tags=e2e ./tests/e2e
for target in windows/amd64 windows/arm64 darwin/amd64 darwin/arm64 linux/arm64; do
  GOOS=${target%/*} GOARCH=${target#*/} go -C mesh build ./...
done
```

All commands exited 0. **17 tested packages** passed; assets has no tests. Hub completed in **76.936s**, integration in **18.731s** and meshnet in **26.511s**. No gate deadline changed.

For each worker below, ran `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, and `cargo test --locked --all-features -- --nocapture`, using its manifest. All formatting, clippy and test commands passed.

| Worker | Tests passed | Ignored |
| --- | ---: | ---: |
| harness | 900 | 0 |
| fp | 72 | 1 |
| rbac-proxy | 129 | 0 |
| tmux | 96 | 1 |
| approval-gate | 201 | 0 |

The fp/tmux ignored entries are existing `configuration_identity_child` subprocess fixtures invoked by passing parent tests. No `skipping:` output occurred. Actual engine-backed proxy tests, approval state bootstrap, three real harness integrations and all **18 repository permission tests** ran. Approval library tests completed in **119.70s**. The supplied-engine failure regressions also passed; a supplied broken engine cannot silently skip integration bodies.

All ten actual workers were rebuilt sequentially with `cargo build --locked --manifest-path <worker>/Cargo.toml --bin <worker>`: IDE, browser, computer, tmux, rbac-proxy, harness, fp, state, queue and session-manager.

The unfiltered real Mesh runner `mesh/tests/e2e/run-tests.sh` passed **20/20 top-level tests in 167.729s**, with no failures or skips. It uses the actual iii binary, the rebuilt workers and Chromium, and invokes `go test -tags=e2e ./tests/e2e -count=1 -timeout=15m -v`. This includes eight observation-correlation subcases and four refusal-deadline subcases, as well as real network, signed-node, supervisor, default/project discovery and worker operations.

The independent proxy runner also passed **10/10**, using a byte-identical isolated copy of its runner/config/harness inputs, fresh state and the rebuilt proxy. Command environment: `III_BIN=<actual iii> WORKER_SRC=<repo>/rbac-proxy WORKER_BIN_TARGET=<repo>/rbac-proxy/target/debug/rbac-proxy ENGINE_PORT=49834 PROXY_PORT=49871 <isolated suite>/run-tests.sh --no-build`. Every case passed, including independent condition-callback refusal with one allowed callback/delivery and zero forbidden callbacks. The unrelated service on port 49134 was not used.

## Runtime observations and confined defaults

The real engine is iii **0.25.0-rc.6**. Actual built workers are IDE **0.12.36**, browser **0.2.33**, computer **0.1.24**, tmux **0.1.3**, and rbac-proxy **1.0.18**. Rust SDK versions are **0.24.6**, except tmux **0.25.0**. Browser operations use Chromium **154.0.8037.97**; tmux is **3.8**; the supplemental desktop uses Xvfb **21.1.25-1**.

Actual local artifact SHA256 values (built binaries, not published release artifacts):

| Artifact | SHA256 |
| --- | --- |
| iii | `691152530a4df1f8d1fd549abb38750dc69ec223b88e8bdafa5476f500ec8ff0` |
| ide | `941f4e180438973d2ebbb64d6195f2df9418d7dfa0ddd941ba1ddb5ec1e26dae` |
| browser | `69c962fe1684ebea7db0d90c794f402bac510a4c5596a4a8835ad2c28b1d2ab4` |
| computer | `c8e521ed6b38e79003132ec2fd4d578e8f674b6bf56b52f86f9f15ee288ce1e0` |
| tmux | `526e52aae4ff7aadc2b61d9ac559850441e578dfaadf6020d9b4376d585e54b2` |
| rbac-proxy | `cfd63a159c4867917b46ea4e8bac693ccff024aa5268553fe3fe695e056e5cef` |

Signed node apply, the production supervisor, digest-verified controlled Resolve/cache fixtures, real overlay gateway, real Rust proxy and real engine are exercised. Authoritative engine worker info confirms `rbac-proxy:laptop::<worker>` ownership and node-prefixed functions; incoming worker metadata is not authority. Controlled fixtures do not establish live Registry publication.

`DefaultAllow()` and `DefaultTriggerTypes()` both return empty **additional** lists. Existing confined configuration grants, owned trigger types and operator grants remain effective. Runtime observations did not justify a broader shared grant.

| Observed outgoing request or binding | Namespace and relevant payload | Result and feature consequence |
| --- | --- | --- |
| All workers: `configuration::ensure` | `default`; `id:"laptop__<worker>"`, name/description, schema object, `metadata:{ui_form:<worker>}`, initial value object | Success; owned configuration bootstrap |
| All workers: `configuration::get` | `default`; `{"id":"laptop__<worker>"}` | Success; reads stay within the node-owned identity |
| All workers: `configuration` binding | Worker namespace; `{"configuration_id":"laptop__<worker>","event_types":["configuration:updated"]}`; target `<worker>::on-config-change` or IDE `shell::on-config-change` | Actual configuration changes delivered to each supervised worker. Supplemental computer registration has no acknowledgement observed; no subsequent update was sent |
| IDE: optional `configuration::get` | `default`; `{"id":"shell-ui","raw":true}` | `invocation_failed`, outside node scope; shared UI settings unavailable |
| IDE: `console:module`, `console:script`, `console:style` | Worker namespace; paths `ide/xterm.js`, `ide/page.js`, `ide/styles.css`; target `ide::ui-content` | Each `REGISTRATION_DENIED`; remote assets unavailable |
| Browser: `harness::hook::pre-generate` | Worker namespace; `{"on_error":"fail_open"}`, target `browser::inject-guidance`, `inject_prompt` metadata | `REGISTRATION_DENIED`; automatic shared harness guidance unavailable |
| Browser: `console:script`, `console:style` | Worker namespace; paths `browser/page.js`, `browser/styles.css`; target `browser::ui-content` | Each `REGISTRATION_DENIED`; remote assets unavailable |
| Computer: `state::list` | `project`; `{"scope":"computer_sessions"}` | `FORBIDDEN`; restart restoration unavailable |
| Computer: `state::set` | `project`; `{"scope":"computer_sessions","key":"c1","value":{"session_id":"c1","endpoint":"native","os":"linux","width":800,"height":600,"created_ms":<runtime timestamp>}}` | `FORBIDDEN`; session durability unavailable |
| Computer: `state::delete` | `project`; `{"scope":"computer_sessions","key":"c1"}` | `FORBIDDEN`; shared session cleanup unavailable |
| Computer: `console:script`, `console:style` | `project`; paths `computer/page.js`, `computer/styles.css`; target `computer::ui-content` | Each `REGISTRATION_DENIED`; remote assets unavailable |
| Tmux: `console:module`, `console:script`, `console:style` | `project`; paths `tmux/xterm.js`, `tmux/page.js`, `tmux/styles.css`; target `tmux::ui-content` | Each `REGISTRATION_DENIED`; remote assets unavailable |

SDK worker registration uses the proxy registration path, without a new ordinary RPC grant. Own trigger types receive the node prefix. Unknown node-owned binding configs and condition callbacks remain rejected. A worker-prefixed asset path cannot confine arbitrary code injected into a shared UI.

Real IDE direct/harness shell and scoped coder paths, browser start/snapshot/stop in both namespaces, and tmux create/capture/kill are exercised. Supervised computer bootstraps and serves but its clean environment has no display: native start returns the observed missing-display error. After that process is stopped, the same binary is launched separately through the gateway with an owned 800×600 Xvfb display and completes start/screenshot/stop. Supplemental capture does not prove supervised desktop capture or native acceptance. Browser profiles, tmux sockets, displays and processes are isolated and owned by the fixtures.

The final runner regenerated six observation artifacts. Counts below are outgoing RPCs / bindings / type registrations. Hashes identify the complete private payload/response records, with no enrollment values or operation image responses in those observation files.

| Observation artifact | Counts | SHA256 |
| --- | --- | --- |
| `TestE2EBootstrapIDEDefaultNamespace-observations.json` | 8 / 4 / 0 | `f096e83eaaf8518b6ffdc8a1735c8a7d932b0de588037ee7f58764132513666d` |
| `TestE2EBootstrapIDEProjectNamespace-observations.json` | 8 / 4 / 0 | `2fc2d59d22cb1e0b5b1f43f38111e2700b8ed91c5afd79f1f117b210d472aaab` |
| `TestE2EBrowserOnNode-default-observations.json` | 5 / 4 / 13 | `c3d80d1bfbb960363efe28f6a554bdd7e35cf2a946e6d1bd9f7468aedaa44982` |
| `TestE2EBrowserOnNode-project-observations.json` | 5 / 4 / 13 | `2d3618475b895c104042782dbfe305a6eee96caa5d23a548ed59ac30dcc623c6` |
| `TestE2EComputerOnNode-observations.json` | 12 / 6 / 8 | `e1569945a3ea943e6e334a1da39f52a5df9175bf95fa2edebcbed5c6597804c9` |
| `TestE2ETmuxOnNode-observations.json` | 6 / 4 / 1 | `7e17dee629d77cab2221aad60cf8a0bcdcdb9a86e4173481366d1b439c44d3ca` |

Computer has 26 entries: 12 no-acknowledgement records, five successes, four `FORBIDDEN`, four `REGISTRATION_DENIED`, and one confirmed configuration delivery. Its supervised binding on connection 2 delivered; its supplemental binding on connection 5 has no acknowledgement observed. Connections are forwarding socket epochs, not process IDs. Correlation requires later frames on the same known connection, the owned configuration identity and matching receiver; earlier process history cannot prove a later binding.

## Evidence boundaries and remaining review items

Tasks 19–25 retain their original commands and failures, regression RED/GREEN records, actual IDE frames, final Console render captures, restart/gateway diagnoses, current observation payloads and process logs in the private delivery archive. This document publishes selected secret-free evidence; raw frames and process logs are not public attachments. Task20's first full gate output was truncated before a complete file was saved; its complete focused current/base reproductions and later corrective evidence are retained.

Exact remote configuration identity/schema lookup works in default and project namespaces. Node-only deployment still cannot deliver the refused Console form assets. An already-loaded local IDE form can match `ui_form=ide`; identity lookup alone does not prove asset delivery.

Node-change events have bounded queues, publication-time eligible binding generations, connection-bound cancellation and no worker history/replay. The SDK can buffer an already-admitted invocation across a narrow disconnect/send race; strict transport cancellation or no-late-delivery is not claimed.

Reviewed restart and gateway corrections have scoped diagnoses and regression evidence. The original natural WireGuard packet-loss location and production cold-first-request latency remain unproved. Historical failures are retained rather than erased by later successful gates.

The Console rapid post-save edit/reset visibility observation has no established cause or fix. Prior independent reset/save checks and the actual-host render matrix passed at their recorded source states. Final review still owns the deferred Console accessibility/deep-link items and tmux regression cleanup with a nondefault `TMUX_TMPDIR`; no closure is claimed here. Retained Mesa/XKB and networking diagnostics remain visible in private logs.

Native Ubuntu 26.04/macOS/Windows 11 installation, privileged services/UAC/TUN/routes/hosts, named tunnel and cross-network paths, drive mounts/locking, packet-capture acceptance, live Registry artifact publication, installer resolution and remote CI remain **unverified**. Cross-builds establish compilation only. The unrelated engine on port 49134 was left untouched.

The implementation plan explicitly requests a Linear ticket. Local workflow inspection found no title/ticket enforcement; the observed main ruleset is disabled, and the CLI reported no branch protection. Organization/external checks remain unverified. No ticket ID has been invented, and no issue, PR or push occurred during local preparation.
