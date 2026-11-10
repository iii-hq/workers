# Local Mesh delivery evidence

> **Removed on 2026-11-10.** The native machine node described here no longer exists. The installer (`install.sh`, `install.ps1`), the `mesh join`, `leave`, `node` and `status` commands, the system service and elevation, Windows native support, node self-update (`mesh-node::update`), the supervisor that ran registry workers on a machine (`nodes.<name>.workers`), shared drives and links were deleted. The mesh worker now runs only inside an iii engine, as a hub or as a client (see `client-mode.md` and `mesh/README.md`). The rest of this document is kept as history.

This records local implementation, the final whole-branch review corrections, and their evidence. The latest user instruction keeps the branch/worktree local: no ticket, push, PR or publication is requested. Native acceptance remains explicitly unverified; the controller owns the scoped re-review.

The branch was clean at `99f65195eb6d467538d7c6318973e75501ec2b08`. Fetch advanced `origin/main` to `60dc4b4ad7efdc4215d6c4f0ccadc7b3447d85e3`; all 48 branch commits rebased without conflict to `2a01cdf5a7448b2af56a6a358b5d44cb7d78963f`. The old merge base was `e466d21d659297c877127f9055ac8a282944562d`; the new merge base is the fetched main commit. A local recovery ref retains the original tip. The Task26 compatibility correction is `9d194cef7`.

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

## Task26 local gates before final review

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

The final-fix runner regenerated six observation artifacts, replacing the Task26 identities below; the Task26 manifest remains in its private archive. Counts below are outgoing RPCs / bindings / type registrations. Hashes identify the complete private payload/response records, with no enrollment values or operation image responses in those observation files.

| Observation artifact | Counts | SHA256 |
| --- | --- | --- |
| `TestE2EBootstrapIDEDefaultNamespace-observations.json` | 8 / 4 / 0 | `215b2f89923b1ad8c86a469899624795fcbb364e5ac155c66061fefaf666a355` |
| `TestE2EBootstrapIDEProjectNamespace-observations.json` | 8 / 4 / 0 | `d6154c08032a3f7bff8ca1747aea61578cd4770595b0ed9d318ecffbea0719ee` |
| `TestE2EBrowserOnNode-default-observations.json` | 5 / 4 / 13 | `4819132a417c49c9f64d9a4a2c7528b1327297e898df21a934b8b1918101d602` |
| `TestE2EBrowserOnNode-project-observations.json` | 5 / 4 / 13 | `b9a0fc6444b01b04c6e930ffc81832516b50edc5f2a4dd703a16c5ad43c96871` |
| `TestE2EComputerOnNode-observations.json` | 12 / 6 / 8 | `c7c3eb2018d297bc0b6acf859e8c1763dc4cc65de15cf4cc76945fd5789a9bf1` |
| `TestE2ETmuxOnNode-observations.json` | 6 / 4 / 1 | `6447240f5f041820e1c9dbab7a524a4d5b83652881545b3baa65031a65039b93` |

Computer has 26 entries: 12 no-acknowledgement records, five successes, four `FORBIDDEN`, four `REGISTRATION_DENIED`, and one confirmed configuration delivery. Its current supervised binding on connection 3 delivered; its supplemental binding on connection 4 has no acknowledgement observed. The earlier Task26 run used connections 2 and 5, respectively. Connections are forwarding socket epochs, not process IDs. Correlation requires later frames on the same known connection, the owned configuration identity and matching receiver; earlier process history cannot prove a later binding.

## Final-review local corrections and gates

The complete four-finding fix wave after `89ed9b0a0` is implemented locally:

- Registry resolutions have a **10-second child request deadline**, covering response headers, JSON decoding and final EOF. The budget matches the existing join HTTP operation; caller cancellation/deadlines remain effective and the outer reconciliation stays alive. Real hanging-header/body regressions pass through the actual signed node handlers: a valid later worker applies, the old worker is removed, the timeout remains visible per worker, and drives reach their controller. That drive test uses fixture host operations, not native mounts.
- Configuration deep links open all ancestor disclosures before focusing. The actual Console host verifies initially closed advanced fields and manually collapsed machine/drive entries at desktop/mobile sizes in both themes. Opaque draft preservation still passes; existing field accessibility metadata remains intact.
- The locale-clean tmux regression retains only `TMUX_TMPDIR` from the parent for consistent creation/teardown. Its nondefault-directory test verifies that the owned server and socket are gone. Production UTF-8 mode is unchanged.
- CI uploads only a generated **`public-report.json`**, containing test outcomes/counts and curated observation identities/statuses. Raw frames, payload/config bodies, error text and process dumps remain private. A runtime-generated credential regression and scans of both retained and fresh real reports prove the selected file excludes all six captured ephemeral engine registration credentials. No publication or production credential exposure is claimed.

Final affected Go fmt/vet/all 17 tested packages/build/E2E-tag-vet and Windows amd64/arm64, macOS amd64/arm64 and Linux arm64 cross-builds passed. The first gate wrapper's `umask 077` altered two existing permission fixtures: requested 0555 became 0500 and requested 0755 became 0700. The original exit 1 log is retained. Exact failing cases then passed with normal 022, followed by one successful final full Go gate; no product permission check, fixture or deadline was weakened.

Affected tmux fmt/clippy with `-D warnings`/all-features tests passed **96 tests**, with the existing ignored child fixture invoked by its passing parent. Pinned Node 22.23.4/pnpm 11.13.1 rebuilt Mesh assets with clean strict lint. The complete actual-host UI matrix passed, including **16 focus cases**, opaque/custom preservation and 54 captures. Frozen Console dependency preparation and test selector updates follow the rebased host's `Settings`, `Save changes` and `Reset` names; their earlier setup/selector failures are retained separately from the meaningful hidden-target RED.

The unfiltered final-source Mesh E2E run passed **20/20 top-level tests in 169.322s**, zero failures/skips. Current observation counts remain IDE 8/4/0 each, browser 5/4/13 each, tmux 6/4/1 and computer 12/6/8. The selected public report contains 20 passing outcomes and 105 curated records; SHA256 `c4abffa18ffdff3ae908c3fe34a999840517590f20d453086dc600f193548a78`. Its two raw credential-bearing frame captures are excluded. Untouched harness/fp/proxy/approval gates and the independent proxy 10/10 result above remain Task26 evidence and were not rerun.

The rebuilt Linux Mesh binary is 22,118,560 bytes, SHA256 `97ed3f429bff22dfa3d7d64614b5102e742b12de3534ad7d75be052a473228ec`. Embedded JS is 15,695 bytes, SHA256 `587ef0deb1a6c8d59207d5be1f343fa8870661b8446f287c0c9e114d2b50eb1e`; CSS remains unchanged. All actual worker/engine binaries retain their Task26 identities; tmux was rebuilt and rehashed. Full new logs, original failures, manifests, browser captures and the detailed fix report remain private under `/home/anderson/mesh-worker-delivery/final-fix/`. The original Task26 archive and earlier-source evidence boundaries remain intact.

## Evidence boundaries and remaining review items

Tasks 19–25 retain their original commands and failures, regression RED/GREEN records, actual IDE frames, final Console render captures, restart/gateway diagnoses, current observation payloads and process logs in the private delivery archive. This document publishes selected secret-free evidence; raw frames and process logs are not public attachments. Task20's first full gate output was truncated before a complete file was saved; its complete focused current/base reproductions and later corrective evidence are retained.

Exact remote configuration identity/schema lookup works in default and project namespaces. Node-only deployment still cannot deliver the refused Console form assets. An already-loaded local IDE form can match `ui_form=ide`; identity lookup alone does not prove asset delivery.

Node-change events have bounded queues, publication-time eligible binding generations, connection-bound cancellation and no worker history/replay. The SDK can buffer an already-admitted invocation across a narrow disconnect/send race; strict transport cancellation or no-late-delivery is not claimed.

Reviewed restart and gateway corrections have scoped diagnoses and regression evidence. The original natural WireGuard packet-loss location and production cold-first-request latency remain unproved. Historical failures are retained rather than erased by later successful gates.

The Console rapid post-save edit/reset visibility observation has no established cause or fix. Prior independent reset/save checks and the actual-host render matrix passed at their recorded source states. The final-review fix wave covers disclosure/deep-link focus and tmux regression cleanup under a nondefault `TMUX_TMPDIR`; the current actual-host matrix includes opaque-value preservation. The already-correct SettingsField accessibility metadata was retained. Retained Mesa/XKB and networking diagnostics remain visible in private logs.

Native Ubuntu 26.04/macOS/Windows 11 installation, privileged services/UAC/TUN/routes/hosts, named tunnel and cross-network paths, drive mounts/locking, packet-capture acceptance, live Registry artifact publication, installer resolution and remote CI remain **unverified**. Cross-builds establish compilation only. The unrelated engine on port 49134 was left untouched.

The earlier ticket/PR delivery plan is superseded by the latest local-only user instruction. No ticket ID has been invented, and no issue, PR, push or other external write occurred. The existing local branch/worktree is preserved.


## 2026-11-05 — observable-protocol migration acceptance

Task2 consumes reviewed runtime29936bcb2 and migrates all20original E2E cases to current Call/Observe APIs, retaining their intent and adding a default/project ADE-content and node-event case. The final unfiltered runner passes **21tests,0failures,0skips in206.981s**, using engine0.26.0 with pinned SDKv0.26.0-rc.5. It proves signed supervised current IDE/browser/computer/tmux workers and authoritative prefixed ownership; actual owned configuration/event delivery and hot reload; canonical bare-source collision confinement; namespace/node/source/args/filter/shared-producer/ADE refusals; node joined/online/removed delivery, watched-node filtering and all-nodes no historical replay. Explicit observer IDs are globally owned by the current engine; generated IDs preserve same-worker-name confinement across nodes.

All8required frontends and10auxiliary Rust binaries were rebuilt with current source and no SKIP_UI_BUILD. The Mesh asset uses host.iii.call. Actual ADE SPA/browser-SDK checks passed desktop/mobile, light/dark, saved/reset configuration and opaque-shape matrices, retaining54screenshots. The public-report regression passes1test and projects only allowlisted current-protocol metadata/outcomes; raw credential-bearing logs/frames remain private. Public E2E report counts are21passed,0failed,0skipped,117observations. CI pins engine0.26.0 and uploads only the curated public report.

Go fmt/vet/build, E2E-tag vet, and the final standalone whole normal suite pass:17packages,617tests/subtests,0failures in133.33s, including the current real proxy claim and production telemetry opt-out regression. Two existing opt-in/native skips remain: official Wintun offline archive provenance and root-only credential-drop acceptance. Controller-owned Windowsamd64/arm64 cross-builds pass; these establish compilation only. One initial normal run timed out in unchanged TestLiveGatewayOwnershipLifecycle waiting for desktop's encrypted TSMP path; unchanged focused reproduction passed9.33s and final whole suite passed it9.32s. Its cause was not established, and no timeout/assertion was weakened.

Exact commands, outputs, binary SHA256 manifest and raw/private evidence are retained under `/home/anderson/mesh-worker-delivery/observable-migration/task2/`; operational detail is retained in `/home/anderson/mesh-worker-delivery/observable-migration/sdd-complete/task-2-report.md`. README and the active skill describe exact-owned configuration filters, the narrowly scoped legacy Mesh hook upgrade, explicit alternative engine ports/temp state, generated observer IDs and the SDK admitted-callback reconnect limit. Queued/unadmitted node events have no history; a callback already admitted before a disconnect can still enqueue after reconnect to the same retained observer. Native services/networking/mounts/session acceptance, complete ADE backend acceptance and live registry/publication remain unverified. Protected enginePID201913 and listener127.0.0.1:49134 retain the baseline starttime; tests use separate loopback engines and owned cleanup. No push, PR, ticket or publication was performed.

Final embedded-asset cross-build verification on delivery source `ba66ee35f` also passed for Windows amd64 and arm64: `CGO_ENABLED=0 GOWORK=off GOFLAGS=-mod=readonly GOTOOLCHAIN=local GOOS=windows GOARCH=<arch> go build ./...` from `mesh/`. The parent evidence directory retains `crossbuild-final-results.json` and `windows-{amd64,arm64}-final-build.log`; this remains compilation evidence only.
