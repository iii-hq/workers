# Mesh Worker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the `mesh` worker (hub and node modes) and its prerequisites in
`harness`, `fp` and `rbac-proxy` as one PR to `iii-hq/workers`. With it, an
agent calls `<node>::<function>` on a person's other machines over a private
WireGuard mesh.

**Architecture:** `mesh` is one Go binary with two modes.
- **Hub mode** runs beside the engine. It runs the coordinator, the DERP
  relay and the device-code join, and serves a gateway on its userspace mesh
  node. It registers `mesh::*` and owns the project's `rbac-proxy` config.
- **Node mode** runs as a system service on other machines. It owns a TUN, a
  hosts block and a front-door shim, exposes hub-signed `mesh-node::*`
  functions, and supervises iii workers fetched from the registry.

The network layer is ported from Mainplane (MIT) onto upstream
`tailscale.com`. Remote workers join the bus through `rbac-proxy` under the
prefix `<node>::`.

**Tech Stack:**
- Go 1.27.1, `tailscale.com v1.102.4` (upstream, no `replace`),
  `filippo.io/cpace`, and the iii Go SDK at the same pseudo-version as
  `whatsapp/go.mod`.
- Rust for `harness`, `fp` and `rbac-proxy`.

**Spec:** `tech-specs/2026-10-mesh-worker/README.md` and
`tech-specs/2026-10-mesh-worker/mesh.md`. Read both. The plan argues from them
and does not repeat their reasoning.

**Port source:** `github.com/mainplane-ai/mainplane` at the commit you clone.
Record its SHA in `mesh/NOTICE`.

## Global Constraints

**Delivery**
- One PR, branched from `main`. Phases 0 → 3 land in order. Each phase ends
  with the CI of every worker it touched green:
  - `go` job: `test -z "$(gofmt -l .)"`, `go vet ./...`, `go test ./...`,
    `make build`, with
    `CGO_ENABLED=0 GOWORK=off GOFLAGS=-mod=readonly GOTOOLCHAIN=local`;
  - `rust` job, per worker: `cargo fmt --all -- --check`,
    `cargo clippy --locked --all-targets --all-features -- -D warnings`,
    `cargo test --locked --all-features`.

**Tailscale**
- Upstream `tailscale.com v1.102.4`, with no `replace` directive.
- Set `TS_NO_LOGS_NO_SUPPORT=true`, and never start `logtail` or `logpolicy`.
- The DERP map lists only the hub.

**Licensing**
- Ported files keep Mainplane's MIT header. `internal/coordinator/LICENSE`
  keeps headscale's BSD-3 notice.

**Defaults and naming**
- **Ports:** `listen` `0.0.0.0:49180`, `gateway_port` `49135`, `proxy_url`
  `ws://127.0.0.1:49200`, node UDP `41643`, `stun_port` `0`.
- **Node names:** `^[a-z][a-z0-9-]{0,30}$`. Reserved: `mesh`, `hub`,
  `engine`, `iii`, `configuration`, `harness`, `shell`, `coder`.
- **Hosts block:** names are `hub.iii.internal` and `<name>.iii.internal`,
  never short names. Markers are `# iii mesh begin` / `# iii mesh end`.
- **TUN:** `iii0` (Linux), `utunN` (macOS), and a wintun adapter `iii` with a
  fixed GUID (Windows).
- **Config ids:** `mesh` for the hub, `<node>__<worker>` for node workers.
  The engine accepts only `[a-z0-9_-]{1,64}`.

**Security values**
- **Device code:** 8 Crockford base32 characters, shown as `XXXX-XXXX`.
- **Join rate limit:** 10 refusals per address per 10 minutes.
  `CF-Connecting-IP` / `X-Forwarded-For` count only from `trusted_proxies`.
- **Ticket:** 32 random bytes, single use, valid 10 s, carried in header
  `x-iii-mesh-ticket`. A session key is 32 random bytes, hex encoded.
- **`mesh-node::*` requests:** `{payload, ts, nonce, sig}`, signed ed25519
  with the hub identity key. The `ts` window is 60 s, and nonces are kept for
  60 s.

**Node workers**
- Supervisor backoff: 1 s, doubling, capped at 60 s.
- Worker environment: `III_URL=ws://hub.iii.internal:49135`,
  `III_NAMESPACE=<hub namespace>`, `III_WORKER_NAME=<worker>`,
  `III_CONFIG_NAME=<node>__<worker>`.

**Registry and install**
- Resolve with `POST https://api.workers.iii.dev/resolve`
  `{"worker","version","target"}`.
- Targets: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
  `x86_64-unknown-linux-musl`, `x86_64-apple-darwin`, `aarch64-apple-darwin`,
  `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`.
- Install scripts are served from
  `https://raw.githubusercontent.com/iii-hq/workers/mesh/v<version>/mesh/install/install.{sh,ps1}`.

**Repo rules**
- `harness/src/filesystem_scope.rs` and `fp/src/pipe.rs` predicates stay
  byte-identical mirrors (no shared crate exists).
- **Comments:** a comment states only the why the code cannot show. No
  ticket ids in code, comments or tests.
- **Fixtures:** build secret-shaped test fixtures at runtime, and never
  commit a redact allowlist.
- **Language:** English in code, commits and the PR.

## Review Focus

Five inputs the spec implies but no other task's tests exercise:
1. **Hub restart with nodes online.** State reloads, nodes are online again
   without rejoining, and their workers re-authenticate with fresh session
   keys. The test is `TestHubRestartKeepsNodes`, in Task 8 and Task 10.
2. **Node clock skewed by more than 60 s.** Signed calls are refused, and the
   skew shows in `mesh-node::status` and `mesh::nodes::get`, not as silent
   failures. The test is `TestVerifyReportsSkew`, in Task 15.
3. **`public_url` with a path prefix or a trailing slash**
   (`https://example.com/mesh/`). The shim, `Redeem` and the install lines
   build `/id`, `/join`, `/ts2021` and `/derp` under the prefix. The test is
   `TestShimJoinsPrefixedURL`, in Task 9.
4. **Hostnames outside the name rule** (`DESKTOP_7Q2`, `123-pc`, 40
   characters, non-ASCII, empty). They become valid, unique names. The test
   is `TestSanitizeName`, in Task 8.
5. **Node offline while its config changes.** Links, workers and drives
   apply when it reconnects. The test is `TestApplyOnReconnect`, in Task 15.

---

## Phase 0 — Prerequisites (Rust)

### Task 1: harness strips `fs_scope` from node-prefixed `shell::*` / `coder::*` calls

**Files:**
- Modify: `harness/src/filesystem_scope.rs` (`is_scoped_function` :89,
  `is_stamped_function` :96, `inject` :101, `mod tests` :131)

**Interfaces:**
- Produces: `fn remote_scoped(function_id: &str) -> bool`. It is true when
  `function_id` is not itself scoped but, after any `<segment>::` boundary,
  continues with an id for which `is_scoped_function` is true. Task 2
  mirrors it byte for byte.

- [ ] **Step 1: Write the failing tests** in `mod tests`:

```rust
#[test]
fn strips_model_fs_scope_from_node_prefixed_coder_call() {
    let out = inject("laptop::coder::read-file",
        json!({ "path": "a", "fs_scope": { "root": "/evil" } }),
        Some("/work/session-7"), &[], workspace());
    assert_eq!(out, json!({ "path": "a" }));
}
#[test]
fn stamps_nothing_on_node_prefixed_shell_call() {
    let out = inject("laptop::shell::exec", json!({ "command": "ls" }), Some("/work/session-7"), &[], workspace());
    assert_eq!(out, json!({ "command": "ls" }));
}
#[test]
fn strips_under_any_prefix_depth() {
    let out = inject("a::b::coder::read-file", json!({ "fs_scope": {} }), None, &[], workspace());
    assert_eq!(out, json!({}));
}
#[test]
fn leaves_prefixed_unscoped_ids_alone() {
    let args = json!({ "fs_scope": 1 });
    assert_eq!(inject("laptop::browser::open", args.clone(), Some("/w"), &[], workspace()), args);
    assert_eq!(inject("laptop::shell::filesystem::read", args.clone(), Some("/w"), &[], workspace()), args);
}
```

- [ ] **Step 2: Run them and see them fail.**
  Run `cd harness && cargo test --locked --all-features --lib filesystem_scope::tests`.
  Expected: the first three FAIL (scope kept or stamped).
- [ ] **Step 3: Implement `remote_scoped`.** In `inject`, when
  `remote_scoped(function_id)` holds, remove `FS_SCOPE_FIELD` and return
  before stamping.
- [ ] **Step 4: Run them again and see them pass.**
  Run the same command. Expected: PASS, and existing tests unchanged.
- [ ] **Step 5: Commit.**
  `git commit -m "fix(harness): never carry a filesystem scope to a node-prefixed shell or coder call"`

### Task 2: fp mirrors the rule and refuses node-reserved steps

**Files:**
- Modify: `fp/src/pipe.rs`. Locations:
  - `is_scoped_step` :129;
  - `forbidden_step` :145, with the shell control plane at :162-164;
  - `bus_step_args` :386;
  - `mod tests` :556.
- Modify: `fp/src/batch.rs`. Locations: `forbidden_call` :123, `mod tests`
  :357.

**Interfaces:**
- Consumes: the rule from Task 1.
- Produces: `pub(crate) fn remote_scoped_step(function_id: &str) -> bool`,
  byte-identical to the harness's `remote_scoped`.

- [ ] **Step 1: Write the failing tests** (pipe tests use the existing
  `scope()` helper):

```rust
#[test]
fn bus_step_args_strips_model_scope_on_node_prefixed_steps() {
    let out = bus_step_args("laptop::coder::read-file",
        json!({ "path": "a", "fs_scope": { "root": "/evil" } }), Some(&scope())).unwrap();
    assert_eq!(out, json!({ "path": "a" }));
}
#[test]
fn forbidden_step_refuses_node_reserved_ids() {
    for id in ["laptop::shell::config-status", "laptop::shell::kill-session-jobs",
               "laptop::shell::workspace::reset", "laptop::mesh-node::workers::apply",
               "mesh::auth", "mesh::middleware", "mesh::on-trigger-registration",
               "mesh::on-config-change"] {
        assert!(forbidden_step(id, false).is_some(), "{id}");
    }
    assert!(forbidden_step("laptop::shell::exec", false).is_none());
}
#[test]
fn remote_scoped_step_matches_harness_table() {
    assert!(remote_scoped_step("laptop::coder::read-file"));
    assert!(remote_scoped_step("a::b::shell::exec"));
    assert!(!remote_scoped_step("coder::read-file"));
    assert!(!remote_scoped_step("laptop::shell::filesystem::read"));
}
```

  In `batch.rs` tests, add `batch_refuses_mesh_node_calls`: validating a
  batch whose call is `laptop::mesh-node::update` returns `Err` containing
  `laptop::mesh-node::update`.
- [ ] **Step 2: Run them and see them fail.**
  Run `cd fp && cargo test --locked --all-features --lib`.
  Expected: the new tests FAIL.
- [ ] **Step 3: Implement.**
  - Add `remote_scoped_step`.
  - In `bus_step_args`, a remote-scoped step loses `fs_scope` and gets none.
  - In `forbidden_step`, apply the shell control-plane checks to the
    remainder after any `<segment>::`, and refuse `*::mesh-node::*` and the
    four `mesh::*` internals.
- [ ] **Step 4: Run them again and see them pass.** Same command. Expected:
  PASS.
- [ ] **Step 5: Commit.**
  `git commit -m "fix(fp): strip scope from and refuse reserved node-prefixed steps in pipe and batch"`

### Task 3: rbac-proxy prefixes a session's trigger types both ways

**Files:**
- Modify: `rbac-proxy/src/interceptor.rs`. Locations:
  - `on_register_trigger_type` :354;
  - `on_register_trigger` :381;
  - `handle_upstream` :775, where `registertrigger` passes unchanged at
    :817;
  - `apply_prefix` :179, `strip_prefix` :188;
  - `mod tests` :1003.

**Interfaces:**
- Produces, for a session with prefix `P`:
  - `registertriggertype{id: T}` → forwarded with `id: "P::T"`;
  - `registertrigger{trigger_type: T}`, where `T` is one of the session's
    own types → `"P::T"`;
  - upstream `registertrigger` / `unregistertrigger` whose `trigger_type`
    starts with `P::` → reach the worker stripped.

- [ ] **Step 1: Write the failing tests.** Use the `session(prefix)` helper
  with `allow_trigger_type_registration: true`:
  - `prefixed_session_registers_trigger_type_under_prefix`: frame
    `{"type":"registertriggertype","id":"shell::job-finished",...}`. Expect
    `Forward` whose JSON has `id == "laptop::shell::job-finished"`.
  - `upstream_registertrigger_for_own_type_is_unprefixed`: `handle_upstream`
    on `{"type":"registertrigger","trigger_type":"laptop::shell::job-finished","function_id":"laptop::x",...}`.
    Expect `Forward` with `trigger_type == "shell::job-finished"` and
    `function_id == "x"`.
  - `unprefixed_session_trigger_types_unchanged`.
- [ ] **Step 2: Run them and see them fail.**
  Run `cd rbac-proxy && cargo test --locked --all-features --lib interceptor::tests`.
  Expected: FAIL.
- [ ] **Step 3: Implement.** Use the existing `apply_prefix` / `strip_prefix`
  helpers, and add an upstream arm for `registertrigger` and
  `unregistertrigger`.
- [ ] **Step 4: Run them again and see them pass.** Same command. Expected:
  PASS.
- [ ] **Step 5: Commit.**
  `git commit -m "feat(rbac-proxy): prefix a prefixed session's trigger types in both directions"`

### Task 4: rbac-proxy names a prefixed session's worker under its prefix

**Files:**
- Modify: `rbac-proxy/src/interceptor.rs`: `on_register_worker` :596, with
  the rename at :653-657 and `WORKER_NAME_PREFIX` at :90.

**Interfaces:**
- Produces: worker name `rbac-proxy:<prefix>::<name>` when the session has a
  prefix. It stays `rbac-proxy:<name>` otherwise.

- [ ] **Step 1: Write the failing tests.**
  - `prefixed_session_worker_name_carries_prefix`: prefix `evil`, name `ide`.
    Expect the forwarded name `rbac-proxy:evil::ide`.
  - `unprefixed_worker_name_unchanged`: expect `rbac-proxy:ide`.
- [ ] **Step 2: Run them and see them fail.**
  Run `cd rbac-proxy && cargo test --locked --all-features --lib interceptor::tests`.
  Expected: the first test FAILS.
- [ ] **Step 3: Implement** the rename from `self.prefix()`.
- [ ] **Step 4: Run them again and see them pass.** Expected: PASS.
- [ ] **Step 5: Commit.**
  `git commit -m "feat(rbac-proxy): name a prefixed session's worker under its prefix"`

### Task 5: rbac-proxy confines a trigger's `condition_function_id`

**Files:**
- Modify: `rbac-proxy/src/interceptor.rs`: `on_register_trigger` :381, where
  `function_id` is rewritten at :437 and denials go through
  `deny_trigger_registration` :445.

**Interfaces:**
- Produces: `config.condition_function_id` is resolved like `function_id`.
  - The session's own function → rewritten to the prefixed id.
  - A function the grants allow → kept.
  - Anything else → `REGISTRATION_DENIED`, with a message naming the id.

- [ ] **Step 1: Write the failing tests.**
  - `condition_function_outside_grants_is_denied`: config
    `{"condition_function_id":"tailscale::logout"}` gets a
    `ReplyToClient` with `REGISTRATION_DENIED`.
  - `own_condition_function_is_prefixed`: the forwarded `config` has
    `condition_function_id == "laptop::cond"`.
  - `allowed_condition_function_passes`.
- [ ] **Step 2: Run them and see them fail.** Same command as Task 4.
  Expected: FAIL.
- [ ] **Step 3: Implement** in `on_register_trigger`, after the
  `function_id` check.
- [ ] **Step 4: Run them again and see them pass.**
  Run `cd rbac-proxy && cargo test --locked --all-features`, then
  `cd rbac-proxy/tests/e2e && ./run-tests.sh` (needs `III_BIN` and Node 20).
  Expected: PASS.
- [ ] **Step 5: Commit.**
  `git commit -m "fix(rbac-proxy): confine a trigger's condition function to the session's grants"`

---

## Phase 1 — Core (Go)

### Task 6: Scaffold the `mesh` Go worker

**Files:**
- Create in `mesh/`: `go.mod`, `go.sum`, `main.go`, `Makefile`,
  `package.json`, `iii.worker.yaml`, `NOTICE`, `README.md`,
  `iii-permissions.yaml`, `internal/hub/hub.go`, `internal/hub/hub_test.go`.
- Modify: `.deploy/workers.yaml`. Add a `mesh:` entry after `whatsapp:`,
  copying its shape:
  - `kind: go-binary`, toolchain `go 1.27.1`, `binary: mesh`;
  - the 7 targets in Global Constraints;
  - no `windows_exception`.

**Interfaces:**
- **`main.go`:** `var version string`. With no subcommand it runs
  `hub.Run(ctx, hub.Options{URL: *url, Version: version})`. Subcommands
  `join`, `status`, `leave` and `node` are added in later tasks.
- **`internal/hub`:** `func Run(ctx context.Context, o Options) error`
  connects with `iii.New(o.URL, iii.WithName("mesh"), iii.WithNamespace(<env namespace>))`
  and registers `mesh::status`.
- **`func Status(version string) StatusResult`:** a pure function.
  `StatusResult` has the spec's fields; this task fills only `Version`.
- **`go.mod`:** `module github.com/iii-hq/workers/mesh`, `go 1.27.1`, the
  SDK at `whatsapp`'s pseudo-version.
- **`iii.worker.yaml`:** `language: go`, `deploy: binary`,
  `manifest: package.json`, `bin: mesh`, and `dependencies: [configuration, rbac-proxy]`.
- **`package.json`:** `{"name":"mesh","version":"0.1.0","private":true}`.

- [ ] **Step 1: Write the failing test.** `TestStatusReportsVersion`:
  `Status("0.1.0").Version == "0.1.0"`.
- [ ] **Step 2: Run it and see it fail.** Run `cd mesh && go test ./...`.
  Expected: FAIL (it does not compile).
- [ ] **Step 3: Implement** the scaffold. The `Makefile` `build` target
  copies `whatsapp`'s, with `-o bin/mesh`. Its `test` target is
  `go test ./...`.
- [ ] **Step 4: Run the checks and see them pass.**
  Run, in `mesh/`: `test -z "$(gofmt -l .)" && go vet ./... && go test ./... && make build`.
  Then run
  `python3 .github/scripts/validate_worker.py --worker mesh --base-ref origin/main --source-changed`.
  Expected: all pass.
- [ ] **Step 5: Commit.**
  `git commit -m "feat(mesh): scaffold the mesh Go worker"`

### Task 7: Join: device code, CPace exchange, rate limit

**Files:**
- Create in `mesh/internal/join/`: `code.go`, `limit.go`, `handler.go`,
  `redeem.go`, `join_test.go`. Port them from Mainplane `pkg/auth/code.go`
  and the `Limit` / `Addr` parts of `pkg/auth/auth.go`. Drop `Store` and API
  keys.

**Interfaces** (produces):
- `func NewCode() string`.
- `func ParseCode(s string) (string, error)`: returns 8 normalized
  characters, or `ErrCode`.
- `func Secret(k ed25519.PrivateKey, code string) string`.
- `func NewLimit(max int, window time.Duration, now func() time.Time) *Limit`,
  with `(*Limit) Allow(addr string) error` and `(*Limit) Refused(addr string)`.
- `func ClientAddr(r *http.Request, trusted []netip.Prefix) string`.
- `func Handler(k ed25519.PrivateKey, code func() string, l *Limit, trusted []netip.Prefix) http.Handler`
  serves `POST /join`.
- `func Redeem(ctx context.Context, publicURL, code string) (secret string, identity ed25519.PublicKey, err error)`.
  It fetches `/id`, runs CPace bound to that identity, and opens the sealed
  secret.

- [ ] **Step 1: Write the failing tests:**
  - `TestParseCode`: `"abcd-efgh"` → `"ABCDEFGH"`; `"O1IL-0000"` →
    `"01110000"`; `"ABC"` → `ErrCode`.
  - `TestLimitRefusesAfterTen`: 10 × `Refused("a")`, then `Allow("a")` is an
    error and `Allow("b")` is nil. After `now` advances 10 min,
    `Allow("a")` is nil.
  - `TestClientAddrTrustsOnlyListedProxies`:
    - peer `203.0.113.9:1` with `CF-Connecting-IP: 198.51.100.4` and no
      trusted proxies → `"203.0.113.9"`;
    - peer `127.0.0.1:1` with trusted `[127.0.0.1/32]` → `"198.51.100.4"`.
  - `TestRedeemRoundTrip` (`httptest`): the right code gives
    `secret == Secret(k, code)`. A wrong code gives `ErrCode`, and the
    limiter has counted one refusal.
  - `TestRedeemAgainstImpostorFails`: a server holding another key fails,
    with a non-nil error.
- [ ] **Step 2: Run them and see them fail.**
  Run `cd mesh && go test ./internal/join/`. Expected: FAIL.
- [ ] **Step 3: Implement** by porting. The limit is `NewLimit(10, 10*time.Minute, time.Now)`.
- [ ] **Step 4: Run them again and see them pass.** Same command. Expected:
  PASS.
- [ ] **Step 5: Commit.**
  `git commit -m "feat(mesh): device-code join with CPace and a per-address limit"`

### Task 8: Coordinator, relay admission and hub state

**Files:**
- Create in `mesh/internal/coordinator/`: `coordinator.go`, `names.go`,
  `state.go`, `LICENSE`, `coordinator_test.go`. Port them from Mainplane
  `pkg/coordinator/coordinator.go`, and persist to `state.json` instead of
  `nodes.json`.
- Create in `mesh/internal/relay/`: `relay.go` and `admit.go`.

**Interfaces** (produces):
- `func New(dir string, join func(secret, addr string) (name string, err error)) (*Coordinator, error)`.
  `state.json` (mode 0600, written by temp file and rename) holds:
  - the identity ed25519 key and the control key;
  - the ULA /48 and the code seed;
  - the nodes and the refused keys.
- `(*Coordinator)` methods:
  - `Handle(mux *http.ServeMux)`: `/id`, `/key`, `/ts2021`;
  - `Listen(ctx context.Context, control string, ports ...int) ([]net.Listener, error)`:
    listeners on the hub's userspace node;
  - `Node(a netip.Addr) (string, bool)`;
  - `Nodes() []NodeInfo`;
  - `Remove(name string) error`: refuses the keys and rotates the code seed;
  - `Link(pairs [][2]string)`;
  - `Known(k key.NodePublic) bool`;
  - `Identity() ed25519.PrivateKey`;
  - `Code() string`.
- `func SanitizeName(host string) string` and
  `func AllocateName(host string, taken func(string) bool) string`.
- `func relay.New(verifyURL string) *derpserver.Server` calls
  `SetVerifyClientURL(verifyURL)` and fails closed.
  `func relay.AdmitHandler(known func(key.NodePublic) bool) http.Handler`
  serves the DERP verify protocol (`tailcfg.DERPAdmitClientRequest` →
  `DERPAdmitClientResponse`) on a loopback-only listener.

- [ ] **Step 1: Write the failing tests:**
  - `TestLinksDecideVisibility`: nodes a, b, c with `Link([][2]string{{"a","b"}})`.
    a's map contains b and the hub, but not c. c's map contains only the hub.
  - `TestSanitizeName` (Review Focus 4): `DESKTOP_7Q2` → `desktop-7q2`;
    `123-pc` → `n-123-pc`; `café-box` → `caf-box`; `""` → `node`; 40× `a` →
    31× `a`.
  - `TestReservedAndTakenNamesGetSuffix`: `mesh` → `mesh-2`; `laptop` when
    already taken → `laptop-2`.
  - `TestRemoveRefusesKeysAndCyclesCode`: after `Remove("a")`,
    re-registration with the same node key is refused, and `Code()` differs
    from before.
  - `TestHubRestartKeepsNodes` (Review Focus 1, coordinator half): `New`,
    register a, close, then `New` on the same dir. `Nodes()` contains a, and
    `Identity()` is unchanged.
  - `TestRelayAdmitsOnlyKnownKeys`.
- [ ] **Step 2: Run them and see them fail.**
  Run `cd mesh && go test ./internal/coordinator/ ./internal/relay/`.
  Expected: FAIL.
- [ ] **Step 3: Implement** by porting. A sanitized name that does not
  start with a letter gets the prefix `n-`.
- [ ] **Step 4: Run them again and see them pass.** Expected: PASS.
- [ ] **Step 5: Commit.**
  `git commit -m "feat(mesh): coordinator, relay admission and hub state ported from Mainplane"`

### Task 9: Node network: meshnet port, hosts block, front-door shim

**Files:**
- Create in `mesh/internal/meshnet/`: `mesh.go`, `frag.go`, `mss.go`,
  `hosts.go`, `status.go`, `router_linux.go`, `router_darwin.go`, and their
  tests. Port them from Mainplane `pkg/mesh`, renamed per Global
  Constraints.
- Create: `mesh/internal/meshnet/shim.go` and `shim_test.go`.

**Interfaces** (produces):
- `func Up(dir, publicURL, secret, name string, pinned key.MachinePublic) (*Mesh, error)`
  brings up the TUN, with control and DERP pointed at a `Shim`.
- `(*Mesh)` methods: `Removed() <-chan struct{}`, `Self() netip.Addr`,
  `Peer(name string) (netip.Addr, bool)`, `Close() error`,
  `Logout() error`.
- `func NewShim(publicURL string, pinned key.MachinePublic) (*Shim, error)`,
  with `(*Shim) URL() string` (`http://127.0.0.1:<port>`) and
  `(*Shim) Close() error`.
- `func WriteHostsBlock(path string, names map[string]netip.Addr) error`.

- [ ] **Step 1: Write the failing tests:**
  - `TestShimServesPinnedKey`: `GET <shim>/key?v=<capver>` decodes as
    `tailcfg.OverTLSPublicKeyResponse` with `PublicKey == pinned`.
  - `TestShimBridgesUpgradeToWebSocket`: a fake front door (`httptest`)
    echoes WebSocket bytes at `/ts2021`. A client HTTP upgrade to the shim
    at `/ts2021` gets the same bytes back. Repeat the test for `/derp`.
  - `TestShimJoinsPrefixedURL` (Review Focus 3): with
    `publicURL = <srv>/mesh/`, the fake front door sees the path
    `/mesh/ts2021`.
  - `TestHostsBlockWritesOnlyInternalNames`:
    - lines outside the block are kept byte for byte;
    - the block holds `fd7a::2 laptop.iii.internal` and no bare `laptop`;
    - a second write replaces the block;
    - an input with CRLF line endings keeps them.
  - Port Mainplane's `pkg/mesh` tests, if any exist.
- [ ] **Step 2: Run them and see them fail.**
  Run `cd mesh && go test ./internal/meshnet/`. Expected: FAIL.
- [ ] **Step 3: Implement.** The shim uses `net/http` and a WebSocket dial
  to `publicURL` joined with the route.
- [ ] **Step 4: Run them again and see them pass.** Expected: PASS (TUN
  tests self-skip without root).
- [ ] **Step 5: Commit.**
  `git commit -m "feat(mesh): node network with a loopback front-door shim on upstream Tailscale"`

### Task 10: In-process mesh integration

**Files:**
- Create: `mesh/internal/integration/mesh_test.go`.

**Interfaces:**
- Consumes: Tasks 7–9. The test nodes are upstream `tsnet.Server`s whose
  `ControlURL` is a `Shim`.

- [ ] **Step 1: Write the tests:**
  - `TestJoinLinkRemove`:
    1. The hub and nodes a and b join with `Redeem`.
    2. a cannot dial b's address.
    3. After `Link([][2]string{{"a","b"}})`, a can dial b within 5 s.
    4. After `Remove("a")`, b's peer list lacks a within 5 s.
    5. a, re-registering with fresh keys and its saved secret, is refused.
  - `TestHubRestartKeepsNodes` (Review Focus 1): restart the hub, and both
    nodes are online within 10 s with no new `Redeem`.
- [ ] **Step 2: Run them.**
  Run `cd mesh && go test ./internal/integration/ -count=1`.
  Expected: PASS once Tasks 7–9 are done. Fix there, not here.
- [ ] **Step 3: Commit.**
  `git commit -m "test(mesh): in-process join, link, removal and restart"`

### Task 11: Hub config and the `mesh::*` functions

**Files:**
- Create in `mesh/internal/hub/`: `config.go`, `functions.go`, `install.go`,
  and their tests.
- Modify: `mesh/main.go`.

**Interfaces** (produces):
- `type Hub struct`, built by `hub.Run`. It holds the coordinator, the live
  `Config`, the SDK client(s), and the gateway and sessions of Tasks 12–13.
  Later tasks add methods to it.
- `type Config` with JSON fields:
  - `public_url`, `listen`, `trusted_proxies`, `gateway_port`, `proxy_url`,
    `stun_port`;
  - `links [][2]string`;
  - `nodes map[string]NodeConfig`, where `NodeConfig{Workers, Allow []string}`;
  - `drives map[string]DriveConfig`, where `DriveConfig{Server string; Nodes []string}`.
- `func DefaultConfig() Config` and `func (Config) Validate() error`.
- Config is registered through `configuration::*` under id `mesh` (resolved
  via `III_CONFIG_NAME`), following `docs/sops/configuration.md`.
  `mesh::on-config-change` hot-reloads it.
- `func InstallLines(version, publicURL, code string) (unix, windows string)`.
- Handlers: `mesh::status`, `mesh::nodes::list`, `mesh::nodes::get`,
  `mesh::nodes::remove`, `mesh::join::code`, `mesh::join::cycle`. Their
  shapes are in mesh.md § Functions.

- [ ] **Step 1: Write the failing tests:**
  - `TestDefaultConfig`:
    - `Listen == "0.0.0.0:49180"`, `GatewayPort == 49135`;
    - `ProxyURL == "ws://127.0.0.1:49200"`, `StunPort == 0`;
    - `PublicURL == ""`.
  - `TestValidate` refuses:
    - a worker entry not matching `^[a-z0-9-]+(@\S+)?$`;
    - a drive whose `server` is `"*"`;
    - a link to itself.
  - `TestInstallLines`: `InstallLines("0.1.0","https://h","ABCD-EFGH")`
    returns exactly the two one-liners of mesh.md § Join, with
    `mesh/v0.1.0`.
  - `TestJoinCodeNeedsPublicURL`: with `PublicURL == ""`, the error is
    `"public_url is not set"`.
  - `TestRemoveReturnsNewCode`.
- [ ] **Step 2: Run them and see them fail.**
  Run `cd mesh && go test ./internal/hub/`. Expected: FAIL.
- [ ] **Step 3: Implement**, and wire the coordinator, relay, join handler
  and front-door listener into `hub.Run`. Two details:
  - The DERP map's single node gets `STUNPort: -1` unless `stun_port > 0`.
  - `AllocateName`'s `taken` covers the existing node names and the
    top-level prefix of every id from `engine::functions::list`.

  Add `TestTakenIncludesEngineFunctionPrefixes`: with an engine listing
  `shell::exec`, hostname `shell` becomes `shell-2`.
- [ ] **Step 4: Run them again and see them pass.** Expected: PASS.
- [ ] **Step 5: Commit.**
  `git commit -m "feat(mesh): hub config, front door and the mesh status, nodes and join functions"`

### Task 12: Gateway

**Files:**
- Create: `mesh/internal/gateway/gateway.go`, `tickets.go` and their tests.

**Interfaces** (produces):
- `const TicketHeader = "x-iii-mesh-ticket"`.
- `func NewTickets(ttl time.Duration, now func() time.Time) *Tickets`, with
  `(*Tickets) Mint(node string) string` and
  `(*Tickets) Consume(t string) (node string, ok bool)`.
- `func New(lookup func(netip.Addr) (string, bool), proxyURL string, t *Tickets) *Gateway`,
  with `(*Gateway) Serve(ln net.Listener) error` and
  `(*Gateway) CloseNode(name string)`.

- [ ] **Step 1: Write the failing tests:**
  - `TestTicketSingleUseAndExpiry`: one `Consume` returns ok and a second
    returns `!ok`. A ticket minted 11 s before its first `Consume` returns
    `!ok`.
  - `TestGatewayStripsClientTicketHeader`: the client sends its own
    `x-iii-mesh-ticket: forged`. The fake proxy sees exactly one value,
    which `Consume`s to the node name.
  - `TestGatewayRefusesUnknownPeer`.
  - `TestCloseNodeDropsConnections`.
- [ ] **Step 2: Run them and see them fail.**
  Run `cd mesh && go test ./internal/gateway/`. Expected: FAIL.
- [ ] **Step 3: Implement.** `Serve` runs on the listener returned by
  `Coordinator.Listen(ctx, control, 49135)`.
- [ ] **Step 4: Run them again and see them pass.** Expected: PASS.
- [ ] **Step 5: Commit.**
  `git commit -m "feat(mesh): mesh-only gateway with single-use tickets"`

### Task 13: Proxy ownership, `mesh::auth`, middleware and trigger hook

**Files:**
- Create in `mesh/internal/hub/`: `proxy.go`, `auth.go`, `sessions.go`,
  `middleware.go`, `triggerhook.go`, and their tests.

**Interfaces** (produces):
- `func EnsureProxyConfig(current map[string]any) (next map[string]any, write bool, status string)`.
- `type Sessions`, with these methods:
  - `Open(node string) (key string)`;
  - `Valid(key string) (node string, ok bool)`;
  - `CloseNode(node string)`.
- **`mesh::auth`** returns rbac-proxy's `AuthResult` shape
  (`rbac-proxy/src/rbac.rs:638`):
  - `function_registration_prefix`;
  - `namespaces`;
  - `allow_trigger_type_registration`;
  - `allowed_trigger_types`;
  - `context`.
- **`mesh::middleware`:** read the exact request fields at
  `rbac-proxy/src/interceptor.rs:719-772` (payload, action, context,
  namespace). It forwards with `iii.Trigger` and returns the target's
  result.
- **`mesh::on-trigger-registration`:** read the request and mapping shape
  at `rbac-proxy/src/interceptor.rs:396-401` and :987-999.
- **Namespaces:** `hub.Run` registers the middleware and the trigger hook in
  its namespace and, when that is not `default`, on a second client built
  with `iii.WithNamespace("default")`.
- **Grant changes:** a change to `nodes.<n>` or to the defaults calls
  `Gateway.CloseNode(n)` and `Sessions.CloseNode(n)`.

- [ ] **Step 1: Write the failing tests:**
  - `TestEnsureProxyConfig`:
    - `nil` → `write`, with `host 127.0.0.1`, `port 49200`,
      `rbac.auth_function_id mesh::auth`,
      `rbac.on_trigger_registration_function_id mesh::on-trigger-registration`
      and `middleware_function_id mesh::middleware`;
    - the mesh block → `!write`;
    - `auth_function_id: other::auth` → `!write`, with
      `status == "owned by other::auth"`.
  - `TestAuthGrants`: a ticket for `laptop` with hub namespace `proj` gives:
    - `function_registration_prefix == "laptop"`;
    - `namespaces.default` exactly
      `[configuration::get, configuration::ensure, configuration::register, configuration::set]`;
    - `allowed_trigger_types` containing `laptop::*` and `configuration`;
    - `context.mesh_node == "laptop"`, and a `context.session_key` of 64 hex
      characters.

    Reusing the ticket is an error.
  - `TestMiddlewareRefusesWithoutSessionKey`.
  - `TestMiddlewareConfigurationRules`:
    - `get` on `laptop__ide` is forwarded with `raw: true`;
    - `get` on `mesh` is refused;
    - `ensure` with the value `"${OPENAI_API_KEY}"` is refused;
    - `ensure` with `initial_value: {"a":{"b":"x${X}"}}` is refused;
    - `configuration::delete` is refused;
    - `shell::exec` is forwarded unchanged.
  - `TestTriggerHookRules`:
    - `harness::hook::pre-trigger` is refused;
    - a `configuration` binding with filter id `mesh` is refused;
    - a `configuration` binding with filter id `laptop__ide` is allowed;
    - a config with `condition_function_id` is refused;
    - a type outside the grant is refused.
  - `TestGrantChangeClosesSessions`.
- [ ] **Step 2: Run them and see them fail.**
  Run `cd mesh && go test ./internal/hub/`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run them again and see them pass.** Expected: PASS.
- [ ] **Step 5: Commit.**
  `git commit -m "feat(mesh): own rbac-proxy and confine node sessions with auth, middleware and trigger hook"`

### Task 14: Registry client

**Files:**
- Create: `mesh/internal/registry/registry.go` and `registry_test.go`.

**Interfaces** (produces):
- `const DefaultBase = "https://api.workers.iii.dev"`.
- `type Artifact struct{ Version, URL, SHA256 string }`.
- `func Resolve(ctx context.Context, c *http.Client, base, worker, versionRange, target string) (Artifact, error)`.
  Errors: `ErrNotBinary`, `ErrNoTarget`, `ErrInsecureURL`, `ErrBadDigest`.
- `func TargetFor(goos, goarch string, musl bool) string`.

- [ ] **Step 1: Write the failing tests** (`httptest` registry):
  - `TestResolvePicksNamedEntry`: the graph holds `ide` and `configuration`,
    and the call returns `ide`'s `binaries[target]`.
  - `TestResolveRejects`: an `http://` url → `ErrInsecureURL`; a 63-hex
    sha256 → `ErrBadDigest`; `type: "image"` → `ErrNotBinary`; a missing
    target → `ErrNoTarget`.
  - `TestTargetFor`:
    - `linux/amd64` glibc → `x86_64-unknown-linux-gnu`, musl →
      `x86_64-unknown-linux-musl`;
    - `darwin/arm64` → `aarch64-apple-darwin`;
    - `windows/arm64` → `aarch64-pc-windows-msvc`.
- [ ] **Step 2: Run them and see them fail.**
  Run `cd mesh && go test ./internal/registry/`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run them again and see them pass.** Expected: PASS.
- [ ] **Step 5: Commit.**
  `git commit -m "feat(mesh): thin client of the registry's resolve endpoint"`

### Task 15: Node service: signed `mesh-node::*` calls, update, reconnect

**Files:**
- Create in `mesh/internal/node/`: `signed.go`, `functions.go`, `update.go`,
  `node.go`, and their tests.
- Create: `mesh/internal/hub/nodecalls.go` and its test.

**Interfaces** (produces):
- `type Signed struct{ Payload json.RawMessage; TS int64; Nonce, Sig string }`,
  with JSON fields `payload`, `ts`, `nonce`, `sig`.
- `func Sign(k ed25519.PrivateKey, fn string, payload any, now time.Time) (Signed, error)`.
- `func NewVerifier(pub ed25519.PublicKey, window time.Duration) *Verifier`,
  with `(*Verifier) Verify(fn string, s Signed, now time.Time) (json.RawMessage, error)`
  and `(*Verifier) LastSkew() time.Duration`.
  Errors: `ErrUnsigned`, `ErrBadSig`, `ErrSkew`, `ErrReplay`.
- `node.Run(ctx) error` connects to `ws://hub.iii.internal:49135` and
  registers:
  - `mesh-node::status`;
  - `mesh-node::update {version, artifact_url, sha256}`;
  - `mesh-node::workers::apply`;
  - `mesh-node::drives::apply` (a no-op until Task 21).
- `func (h *Hub) OnNodeOnline(ctx context.Context, node string) error`: if
  the versions differ, it calls only `update`. Otherwise it calls
  `workers::apply`, then `drives::apply`.

- [ ] **Step 1: Write the failing tests:**
  - `TestVerifyRefuses` (table):
    - an unsigned request → `ErrUnsigned`;
    - a flipped byte in `sig` → `ErrBadSig`;
    - `ts` at `now - 61s` → `ErrSkew`;
    - the same nonce twice → `ErrReplay`;
    - a valid request → its payload.
  - `TestVerifyReportsSkew` (Review Focus 2): after an `ErrSkew` at +90 s,
    `LastSkew() == 90*time.Second`, and `mesh-node::status` carries
    `clock_skew_secs: 90`.
  - `TestUpdateFirstWhenVersionsDiffer`: with fake node calls, version
    `0.1.0` against hub `0.2.0` calls `[update]`. Equal versions call
    `[workers::apply, drives::apply]`.
  - `TestApplyOnReconnect` (Review Focus 5): change `nodes.laptop.workers`
    while laptop is offline. `OnNodeOnline("laptop")` sends the new list.
  - `TestUpdateRefusesBadSHA`.
- [ ] **Step 2: Run them and see them fail.**
  Run `cd mesh && go test ./internal/node/ ./internal/hub/`.
  Expected: FAIL.
- [ ] **Step 3: Implement.** `update` replaces the binary by temp file and
  rename, then exits so the service manager restarts it.
- [ ] **Step 4: Run them again and see them pass.** Expected: PASS.
- [ ] **Step 5: Commit.**
  `git commit -m "feat(mesh): hub-signed node functions, self-update and apply on reconnect"`

### Task 16: Supervisor

**Files:**
- Create in `mesh/internal/supervisor/`: `supervisor.go`, `runas_unix.go`
  (`SysProcAttr.Credential` for the operator), and `supervisor_test.go`.

**Interfaces** (produces):
- `type Desired struct{ Name, Version, URL, SHA256 string }`.
- `func New(cacheDir, logDir string, op Operator, node, hubNS string) *Supervisor`.
- `(*Supervisor) Apply(ctx context.Context, ds []Desired) []Result`.
- `func WorkerEnv(node, hubNS, worker string) []string`.
- `func Backoff(restarts int) time.Duration`.

- [ ] **Step 1: Write the failing tests:**
  - `TestWorkerEnv`: `WorkerEnv("laptop","proj","ide")` equals
    `[III_URL=ws://hub.iii.internal:49135, III_NAMESPACE=proj, III_WORKER_NAME=ide, III_CONFIG_NAME=laptop__ide]`.
  - `TestBackoff`: restarts 0..7 give 1, 2, 4, 8, 16, 32, 60, 60 seconds.
  - `TestShaMismatchRefusesStart`: the `Result` error names the expected
    digest, and nothing is started.
  - `TestStopsUndesired`.
- [ ] **Step 2: Run them and see them fail.**
  Run `cd mesh && go test ./internal/supervisor/`. Expected: FAIL.
- [ ] **Step 3: Implement.** The cache key is the sha256. Logs rotate at
  10 MiB, keeping 3 files.
- [ ] **Step 4: Run them again and see them pass.** Expected: PASS.
- [ ] **Step 5: Commit.**
  `git commit -m "feat(mesh): supervise node workers as the operator with backoff"`

### Task 17: Service install, elevation, CLI and `install.sh` (Linux, macOS)

**Files:**
- Create: `mesh/internal/service/` with `service.go`, `systemd.go`,
  `launchd.go` and their tests.
- Create: `mesh/internal/elevate/`, ported from Mainplane.
- Create: `mesh/internal/cli/` with `join.go`, `status.go`, `leave.go`.
- Create: `mesh/install/install.sh`.
- Modify: `mesh/main.go`.

**Interfaces** (produces):
- `service.Install(binPath, operator string) error` and
  `service.Uninstall() error`.
- `cli.Join(ctx context.Context, publicURL, code string, port int) error`.
  It runs `Redeem`, writes the node state, installs the service and starts
  it.
- `cli.Status(w io.Writer) error` and `cli.Leave(ctx context.Context) error`.
- `install.sh <version> <public_url> <code>`:
  1. resolves `mesh@=<version>` for the host target through `DefaultBase`;
  2. checks the sha256;
  3. runs `mesh join` under sudo.

  `--dry-run` prints the resolved URL and exits.

- [ ] **Step 1: Write the failing tests:**
  - `TestSystemdUnit`: the unit contains
    `ExecStart=/usr/local/bin/mesh node` and `Restart=always`.
  - `TestLaunchdPlist`: the plist has `KeepAlive` true.
  - `TestJoinArgs`.
- [ ] **Step 2: Run them and see them fail.**
  Run `cd mesh && go test ./internal/service/ ./internal/cli/`.
  Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run them again and see them pass.**
  Run the same tests, then `shellcheck mesh/install/install.sh` and
  `sh mesh/install/install.sh --dry-run 0.1.0 https://h ABCD-EFGH`.
  Expected: PASS, and the dry run prints an `https://` artifact URL.
- [ ] **Step 5: Commit.**
  `git commit -m "feat(mesh): join, status and leave with systemd and launchd services"`

### Task 18: Permissions, skill and README

**Files:**
- Modify: `iii-permissions.yaml` (root).
- Create: `mesh/iii-permissions.yaml`, `mesh/skills/SKILL.md`.
- Modify: `mesh/README.md` and `approval-gate/tests/repository_permissions.rs`.

**Interfaces:**
- Root rules:
  - allow `mesh::status`, `mesh::nodes::list`, `mesh::nodes::get`;
  - deny `'!*::mesh-node::*'`, `'!mesh::auth'`, `'!mesh::middleware'`,
    `'!mesh::on-trigger-registration'`, `'!mesh::on-config-change'`,
    `'!mesh::ui-content'`, `'!mesh::configuration-id'`;
  - deny `'!*::shell::pty::*'`, `'!*::shell::scm::*'`,
    `'!*::shell::ui-state::*'`, `'!*::shell::config-status'`,
    `'!*::shell::kill-session-jobs'`, `'!*::shell::on-config-change'`.

  First match wins, so no new rule may shadow an existing one.
- Skill content: the six points of mesh.md § Agent exposure.

- [ ] **Step 1: Write the failing test** in
  `repository_permissions.rs`:
  - `laptop::mesh-node::update` → Deny;
  - `mesh::nodes::list` → Allow;
  - `laptop::shell::exec` → needs approval;
  - `laptop::coder::read-file` → needs approval;
  - `shell::config-status` keeps its current decision.
- [ ] **Step 2: Run it and see it fail.**
  Run `cd approval-gate && cargo test --locked --test repository_permissions`.
  Expected: FAIL.
- [ ] **Step 3: Implement.** Add the rules. Write the skill and the README
  sections that `validate_worker.py` requires.
- [ ] **Step 4: Run it again and see it pass.**
  Run the same command, plus the `validate_worker.py` line from Task 6.
  Expected: PASS.
- [ ] **Step 5: Commit.**
  `git commit -m "feat(mesh): permissions, skill and README"`

### Task 19: Phase 1 end-to-end and adversarial suite

**Files:**
- Create: `mesh/tests/e2e/e2e_test.go` (build tag `e2e`) and
  `mesh/tests/e2e/run-tests.sh`, which needs `III_BIN` and built `ide` and
  `rbac-proxy`.
- Create: `.github/workflows/mesh-e2e.yml`, triggered on `mesh/**` and
  `rbac-proxy/**`, modelled on `rbac-proxy-e2e.yml`.

**Interfaces:**
- Consumes: Tasks 1–18. The nodes are `tsnet` nodes in-process. They run
  the real `ide` binary and a fake Go worker.

- [ ] **Step 1: Write the tests.** Each is named after the mesh.md § Testing
  case it proves:
  - **Bootstrap:** `TestE2EBootstrapIDEDefaultNamespace`,
    `TestE2EBootstrapIDEProjectNamespace`.
  - **Coexistence:** `TestE2ETwoNodesSameWorkerName`.
  - **Adversarial:**
    - `TestE2ETemplatesRefused`, `TestE2EHooksRefused`,
      `TestE2EConditionCallbackRefused`;
    - `TestE2ESupervisorUnreachable`: direct, `fp::pipe`, `fp::batch` and a
      replayed signed request;
    - `TestE2EMiddlewareNeedsSessionKey`;
    - `TestE2ERevocationWithin5s`;
    - `TestE2ENameSquatting`;
    - `TestE2ELoopbackWithoutTicket`;
    - `TestE2ENodeRegistersMeshIDGetsPrefix`;
    - `TestE2EFsScopeOnRemoteCoder`: a direct call, `fp::pipe` and
      `fp::batch`.
  - **SDK behaviour:** `TestE2ERefusedRegistrationLogged` (Open item 5):
    record whether the Rust SDK reconnects.
- [ ] **Step 2: Run them.**
  Run `cd mesh/tests/e2e && ./run-tests.sh`. Expected: PASS. Fix failures in
  the owning task's code.
- [ ] **Step 3: Run the machine acceptance and record it.** Run mesh.md
  acceptance 1–7, 9 and 10 on a fresh Ubuntu 26.04 VM, a Mac on another
  network, and the hub behind a named Cloudflare tunnel. Record the
  evidence (commands and outputs) for the PR description.
- [ ] **Step 4: Commit.**
  `git commit -m "test(mesh): end-to-end and adversarial suite"`

---

## Phase 2 — Reach

### Task 20: Windows nodes

**Files:**
- Create: `mesh/internal/meshnet/router_windows.go` (ported).
- Create in `mesh/internal/service/`: `windows.go`, and `wintun.go`, where
  the `wintun.dll` sha256 is a const.
- Create: `mesh/internal/supervisor/runas_windows.go`, which starts workers
  in the operator's session.
- Create: `mesh/install/install.ps1`.

**Interfaces:**
- Produces: the same `service.Install` / `Uninstall` and `Supervisor`
  contracts on Windows, plus `func verifyWintun(b []byte) error`.

- [ ] **Step 1: Write the failing tests:**
  - `TestVerifyWintun`: one changed byte → error.
  - `TestHostsBlockWindowsCRLF`: runs on every OS.
  - A CI cross-build step: `GOOS=windows GOARCH=amd64 go build ./...` and
    `GOOS=windows GOARCH=arm64 go build ./...`.
- [ ] **Step 2: Run them and see them fail.**
  Run `cd mesh && go test ./internal/service/ ./internal/meshnet/`.
  Expected: FAIL.
- [ ] **Step 3: Implement.** The skill names the Windows nodes, which have
  no `shell::*` / `coder::*` until `ide` ships for Windows.
- [ ] **Step 4: Run them again and see them pass,** including both cross
  builds.
- [ ] **Step 5: Run the acceptance.** Run mesh.md acceptance 1–6 on
  Windows 11, including a relayed path. If TCP stalls on the relayed path,
  record that under Open item 6 (IPv6 first fragments) and stop that path;
  do not fork in this PR.
- [ ] **Step 6: Commit.**
  `git commit -m "feat(mesh): Windows nodes"`

### Task 21: Drives

**Files:**
- Modify: `mesh/internal/coordinator/` (port `pkg/coordinator/drives.go`).
- Create: `mesh/internal/drives/`, ported from `pkg/worker/drives*.go`.
- Modify: `mesh/internal/node/functions.go` (`drives::apply`) and
  `mesh/internal/hub/config.go`.

**Interfaces:**
- Produces:
  - `(*Coordinator) SetDrives(map[string]Drive) []error`;
  - `(*Coordinator) Desired(node string) drives.Desired`;
  - `drives.Apply(d drives.Desired) []Result`.

- [ ] **Step 1: Write the failing tests:**
  - `TestDesiredExcludesEphemeralFromStar`.
  - `TestMountTargets`: Linux `/drives/proj`, macOS `/Volumes/proj`,
    Windows `W:`.
  - `TestServerMustBeLinux`.
  - `TestSmbdUnitConfinedToMesh`: the unit contains `IPAddressDeny=any` and
    `IPAddressAllow=<ula>/48`.
  - `TestExportsOnlyMembers`.
- [ ] **Step 2: Run them and see them fail.**
  Run `cd mesh && go test ./internal/coordinator/ ./internal/drives/`.
  Expected: FAIL.
- [ ] **Step 3: Implement** by porting.
- [ ] **Step 4: Run them again and see them pass.** Then run acceptance 8
  on the machine matrix and record it.
- [ ] **Step 5: Commit.**
  `git commit -m "feat(mesh): shared drives over NFS and SMB"`

### Task 22: Console page

**Files:**
- Create: `mesh/ui/page.tsx`, `mesh/ui/build.mjs`, `mesh/ui/styles.css`.
- Create: built assets under `mesh/assets/`, committed per the go-binary
  SOP.
- Create: `mesh/internal/hub/ui.go`, which `go:embed`s the assets and
  registers `mesh::ui-content` per `docs/sops/injectable-console-ui.md`.

**Interfaces:**
- Produces: a page showing nodes (online, path, version, workers, drives)
  and the join code with its one-liners. Config edits go through the
  generated form.
- Produces: `mesh::configuration-id`, returning `{ "id": "<effective id>" }`
  per `docs/sops/configuration.md`, so the Console form finds the `mesh`
  entry.

- [ ] **Step 1: Write the failing tests.** `TestUIContentServesEmbeddedAssets`
  and `TestConfigurationIDHonoursConfigName` (`III_CONFIG_NAME=mesh-lab` →
  `{"id":"mesh-lab"}`).
- [ ] **Step 2: Run them and see them fail.**
  Run `cd mesh && go test ./internal/hub/ -run 'TestUIContent|TestConfigurationID'`.
  Expected: FAIL.
- [ ] **Step 3: Implement.** Check Open item 2 (whether the Console finds
  node worker forms) and record the result in the PR.
- [ ] **Step 4: Run it again and see it pass.** Run `make ui && go test ./...`.
- [ ] **Step 5: Commit.**
  `git commit -m "feat(mesh): Console page"`

---

## Phase 3 — Automation

### Task 23: Join tokens and ephemeral nodes

**Files:**
- Create: `mesh/internal/join/token.go` and its test.
- Modify: the coordinator's join callback, `mesh/internal/hub/functions.go`
  (`mesh::join::token`, `mesh::join::revoke`) and the state.

**Interfaces:**
- Produces:
  - `type Token struct{ ID, Hash string; Expires time.Time; Reusable, Ephemeral bool; Nodes []string }`;
  - `IssueToken(opts TokenOptions) (id, token string, err error)`;
  - `RedeemToken(token string) (TokenGrant, error)`;
  - `RevokeToken(id string) error`;
  - `RevokeForNode(name string)`.

- [ ] **Step 1: Write the failing tests:**
  - `TestSingleUseTokenConsumed`.
  - `TestReusableRequiresEphemeral`: the error is
    `"reusable requires ephemeral"`.
  - `TestExpiredAndRevokedRefused`.
  - `TestRemoveRevokesCreatingToken`.
  - `TestEphemeralExpiresAfter3Min` (fake clock).
  - `TestDeviceCodeUnaffectedByRevoke`.
- [ ] **Step 2: Run them and see them fail.**
  Run `cd mesh && go test ./internal/join/ ./internal/coordinator/ ./internal/hub/`.
  Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run them again and see them pass.** Same command. Expected:
  PASS.
- [ ] **Step 5: Commit.**
  `git commit -m "feat(mesh): single-use and ephemeral join tokens"`

### Task 24: `mesh::node-changed`

**Files:**
- Create: `mesh/internal/hub/events.go` and its test.

**Interfaces:**
- Produces: `RegisterTriggerType("mesh::node-changed", ...)`.
  - The binding config is `{node?}`; any other key is refused.
  - Deliveries are `{name, event, at}`, with `event` one of `joined`,
    `online`, `offline`, `removed`, `outdated`.

- [ ] **Step 1: Write the failing tests:**
  - `TestBindingRejectsUnknownKeys`.
  - `TestDeliversOnlyForBoundNode`.
  - `TestNoReplayForLateBinding`.
- [ ] **Step 2: Run them and see them fail.**
  Run `cd mesh && go test ./internal/hub/ -run 'TestBinding|TestDelivers|TestNoReplay'`.
  Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run them again and see them pass.** Same command. Expected:
  PASS.
- [ ] **Step 5: Commit.**
  `git commit -m "feat(mesh): node-changed trigger type"`

### Task 25: `browser`, `computer` and `tmux` on nodes; default lists

**Files:**
- Create: `mesh/internal/hub/defaults.go` and its test.
- Modify: `mesh/tests/e2e/e2e_test.go`.

**Interfaces:**
- Produces: `func DefaultAllow() []string` and
  `func DefaultTriggerTypes() []string`, derived per Open item 1 and
  limited to what passes the confinement test.

- [ ] **Step 1: Record what each worker calls.** Run `ide`, `browser`,
  `computer` and `tmux` behind the gateway, with the e2e harness logging
  every call and binding. Save the log in the PR description.
- [ ] **Step 2: Write the failing test.** `TestDefaultsExcludeUnconfinable`:
  no entry matches `harness::hook::*`, `state::*`, `stream::*`, `queue::*`,
  `http::*` or `cron::*`.
- [ ] **Step 3: Implement** the lists, and add `TestE2EBrowserOnNode`
  (`<node>::browser::*` registered and callable).
- [ ] **Step 4: Run them and see them pass.** Run `go test ./internal/hub/`,
  then `./run-tests.sh` (in `mesh/tests/e2e/`). Then run acceptance 11.
- [ ] **Step 5: Commit.**
  `git commit -m "feat(mesh): default grants and browser, computer and tmux on nodes"`

---

## Task 26: Open the PR

- [ ] **Step 1:** Rebase on `origin/main`. Then run every gate from Global
  Constraints for `mesh`, `harness`, `fp` and `rbac-proxy`, plus
  `approval-gate`'s permission test and both e2e suites.
- [ ] **Step 2:** Create the Linear ticket, which the PR title needs for
  CI. Then open a draft PR whose description maps each phase to its commits
  and attaches the acceptance evidence from Tasks 19–25.
- [ ] **Step 3:** Run `git diff origin/main...HEAD | grep -E '^\+.*M[O]T-[0-9]'`.
  It must print nothing, because ticket ids belong only in the PR title.
