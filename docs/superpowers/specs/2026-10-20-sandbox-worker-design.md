# Sandbox worker: moving the VM substrate out of the iii engine

- Status: design, ready for planning. Date: 2026-10-20.
- Code read: iii repo at `342a7704c`, workers repo at `8773952be`. Nothing was built or run; *inference* marks claims not confirmed by running code.
- Tags: **[User]** decided by the product owner · **[Designer]** designer's call, accepted, builder may refine · **[Builder]** left to the builder · **[Designer, plan triage]** changed by plan triage decisions D1-D13 (T1 in 13). Settlements S1-S7 (designer's calls) and decisions Q1-Q4 (user) are indexed in 13.
- Paths under `engine/`, `crates/`, `scripts/` are in the iii repo (the installer is `engine/install.sh`); paths under a worker folder or `template/` are in the workers repo.

## 1. Intent and why

All microVM code lives in the engine repo inside the `iii-worker` binary:
- The engine spawns it as `iii-sandbox` (`engine/src/workers/external.rs:44-48`: `KNOWN_EXTERNAL { name: "iii-sandbox", binary: "iii-worker", args: &["sandbox-daemon"] }`).
- Compose execs it to boot `bundle` and `local` VM containers (`crates/iii-compose/src/lifecycle.rs:1199-1355`).

So every engine release carries the libkrun VMM (`msb_krun`), 69 MB (three blobs, 66 MiB on disk) of committed libkrunfw firmware, a static-musl guest init and a pipeline built around them. The backend can only be libkrun, and its version is welded to the engine's.

The engine should own only the **sandbox interface** (a contract) and the **choice of backend**. Each backend is an **adapter worker**:
- `sandbox-builtin` now: today's libkrun code, moved out;
- Docker and iii-cloud later, through the same interface and with no engine change.

This serves sandbox consumers (computer, sandbox-code-runner, ide, harness, the image pickers), Compose users running bundle or `runtime.base_image` workers, engine maintainers (whose release loses the VM build) and future adapter authors.

## 2. Goals, non-goals, anti-goals

**Goals**
1. All VM code and assets leave the iii repo for one new workers-repo worker, `sandbox-builtin`. [User]
2. The engine keeps a contract crate, a small router and config; no libkrun, firmware or `iii-worker` remains. [User]
3. Compose installs the configured adapter when a project needs one, and boots bundle and local VMs through the contract. [User]
4. Consumers keep calling `sandbox::*` in namespace `default`, unchanged. [User]
5. Old configs fail with actionable errors, and a documented migration step exists. [User]

**Non-goals**
- Docker and iii-cloud adapters. They are later work, but the contract must fit them (5.7). [User]
- Standalone `iii -c config.yaml`, which is deprecated. [User]
- Refactoring the daemon's libkrun-shaped traits (`VmLauncher`, `ShellRunner`, `VmStopper`, `FsRunner`).
- Musl hosts at launch. [User Q3: a product choice, not a firmware limit (8.5)]
- Intel Mac and Windows hosts. [Designer's call: libkrun/libkrunfw ship no x86_64-darwin or Windows build]
- A shell into worker VMs (today's `iii-worker exec` for managed workers). `worker::exec` stays reserved. [Designer's call S6]
- Cut from v1 because nothing uses them yet [Designer, ponytail review]: capability negotiation (5.7), the JSON Schema bundle, an `iii compose migrate` command (7.9), hot-applied adapter config (7.6), moving the guest env off the kernel cmdline (8.3), `create.guest_engine_url`, the `worker::{wait, list, renew}` operations, leases, archive or upload worker sources, consumer soft needs (7.1, User Q2 revised), a separate `SANDBOX_ADAPTER_IS_A_VM` error, and hiding the router's ids on an unbound engine. Each can return as an additive change when an adapter or consumer needs it.

**Anti-goals**
- Changing any `sandbox::*` request or response shape, or the S-code error wire format.
- The engine depending on a workers-repo artifact at build time.
- Any worker taking over `sandbox::*` by registering first.
- Compose exec'ing an adapter binary to boot VMs.

## 3. Current state

| Piece | Where today | Used by |
|---|---|---|
| Sandbox daemon: 16 ids `create, exec, stop, list, catalog::list, run, fs::{ls,stat,mkdir,rm,chmod,mv,grep,sed,write,read}` | `crates/iii-worker/src/sandbox_daemon/` (about 8.5k LOC), registered as `iii-sandbox` in `default` (`mod.rs:125-139`) | computer (`computer/src/driver/sandbox.rs:32`), sandbox-code-runner (`src/engine.rs:66`), ide (`src/triggers.rs:27-45`), harness (`src/clients/engine.rs:315-325`), pickers (`computer/ui/src/lib/computer.ts:37`, `ade/ui/src/configuration-forms/specs/sandbox-catalog.ts`) |
| Engine spawn | `KNOWN_EXTERNAL`, `create_worker` step 3 (`engine/src/workers/config.rs:407-433`), temp `--config` YAML, lifeline, `setsid` (`external.rs:156-255`) | `engine.workers.iii-sandbox` (`CONFIGURABLE_ENGINE_WORKERS`, `crates/iii-compose/src/config.rs:58-64`) |
| Compose VM containers | `iii-worker __bundle-prepare` / `__local-prepare` (20-minute timeout, `lifecycle.rs:1281-1300`), then `__vm-boot` spawned as a plain child: "The handle that comes back is an ordinary child, so readiness, stop, crash cascade and log capture below are unchanged." (`lifecycle.rs:1119-1121`) | Registry `kind: bundle` packages; `path://` with `runtime.base_image` (`manifest.rs:143-153`) |
| VM substrate | `iii-worker`: `vm_boot`, `local_worker`, `bundle_prepare`, `bundle_download`, `erofs`, `rootfs_cache`, `worker_manager/{oci,libkrun,platform}`, `firmware`, `upper` (+ `vendor/upper-golden.iiu.gz`), `daemon_exit`. Crates: `iii-init`, `iii-network`, `iii-filesystem`, `iii-supervisor`, `iii-shell-{proto,client}`. VMM: `msb_krun =0.1.16` from crates.io | Daemon and Compose |
| Firmware and init | `engine/firmware/libkrunfw-{darwin-aarch64.dylib,linux-x86_64.so,linux-aarch64.so}` (69 MB (three blobs, 66 MiB on disk), 5.2.1, ABI 5, plain git blobs). Embedded by `crates/iii-worker/build.rs:24-26`; `iii-init` embedded through `iii-filesystem` `embed-init`; fallback download from tag `iii/v<CARGO_PKG_VERSION>` (`firmware/download.rs:76-82`) | Every boot |
| Prefetch | Only `iii project init --learn-iii` runs `iii-worker __pull-images` (`engine/src/cli/project/mod.rs:334-349`, called at :407 and :424 inside `run_learn_iii`); plain `iii project init` never prefetches | The learn-iii tour |
| Release | `release-iii.yml` jobs `init-build` (:129), `firmware-upload` (:215), `worker-release` (:309); `engine/install.sh:623-674, 805-811`; `iii update` (`engine/src/cli/registry.rs`: `iii-init` at :46-60, `iii-worker` at :100-116) | Users |
| macOS entitlement | Binary re-signs itself at first boot (`worker_manager/platform.rs:18-68`); no CI signing | macOS |

## 4. Target architecture

| Component | Location | Owns |
|---|---|---|
| Contract crate `iii-sandbox-contract` | iii: `crates/iii-sandbox-contract` | Id table, types, S-codes, `AdapterInfo`, JSON Schemas |
| Router, a built-in engine worker named `iii-sandbox` | iii: `engine/src/workers/sandbox/` | Public `sandbox::*` ids in `default`; forwarding; reserved prefix |
| Engine config | iii: `EngineConfig.sandbox` | Which adapter, in which namespace |
| Compose | iii: `crates/iii-compose` | Need detection, auto-install, ordering, VM supervision over the bus |
| `sandbox-builtin` | workers: `sandbox-builtin/` | Both contract families on libkrun: daemon, VM substrate, firmware, `iii-init` |

```
 consumer (computer, ide, ...)                  ENGINE
   sandbox::exec (ns=default) ---> [ iii-sandbox router: owns sandbox::* in default ]
                                        | call_value_with_metadata_ns("my-proj", "sandbox-builtin::exec")
                                        v
                      namespace "my-proj" (Compose project)
                      [ sandbox-builtin container: WS worker, host child of Compose ]
                         sandbox-builtin::{create,exec,...,fs::*}     -> __vm-boot (sandbox VM)
                         sandbox-builtin::worker::{prepare,start,...} -> __vm-boot (bundle VM)
                                        ^
 Compose daemon (worker "compose")      | direct bus calls in ns "my-proj"
   up: install sandbox-builtin, then for VM container "my-bundle":
       worker::prepare -> worker::start -> worker::status / logs ... -> worker::stop
 guest worker in VM --(III_URL via gateway -> host 127.0.0.1)--> engine: registers (my-proj, my-bundle)
```

## 5. The contract

### 5.1 Location and versioning [Designer b]
- `crates/iii-sandbox-contract` in the iii repo.
  - Dependencies: `serde`, `serde_json`, `schemars` only; `iii-sdk` is a dev-dependency for one test.
  - The engine uses it by path. A new `release-iii.yml` job publishes it to crates.io via `_rust-cargo.yml`; alpha tags do not publish it [Designer, plan triage]. Adapters pin it from crates.io.
  - `version.workspace = true`, so the release's existing root-version bump moves it; no `create-tag.yml` or release-script edit. [Designer, plan triage]
- The crate version moves in lockstep with the engine. The wire version is separate: `CONTRACT_MAJOR = 1`, `CONTRACT_MINOR = 0`. Only a major bump breaks. An adapter package's major version equals the contract major it implements (7.2). [Designer's call S2]
- Contents:
  - `OPERATIONS` (the 16 ephemeral ids: suffix, schemas; no family enum), the six `worker::*` suffix constants and `adapter_function_id()` [Designer, plan triage];
  - typed requests and responses, with its own copies of `FsEntry`, `FsMatch`, `FsSedFileResult` (no `iii-shell-proto` dependency) and a wire-compatible `StreamChannelRef`. Today `fs::read`/`fs::write` embed `iii_sdk::channels::StreamChannelRef` (`fs/read.rs:17, 71`, `fs/write.rs:19, 147`), and 5.4 uses it. The dev-dependency test pins its JSON to iii-sdk's type. `FsReadMeta` is internal to the shell protocol and is not copied;
  - the `exec.workdir` description corrected to "`/` when omitted" (today "image default when omitted", `exec.rs:132`; see 5.8);
  - `SandboxErrorCode` and the message builder;
  - `AdapterInfo`;
  - `DOCS_BASE` on iii.dev (today a repo path, `sandbox_daemon/errors.rs:18-19`).

### 5.2 Function ids
- Adapter ids are `<adapter>::<suffix>`, where the suffix is the public id without `sandbox::` (`sandbox::fs::ls` becomes `sandbox-builtin::fs::ls`). Every adapter registers `<adapter>::adapter::info`.
- **Ephemeral family**: the 16 existing ids. The router exposes and forwards them. [User: unchanged for consumers]
- **Worker-VM family** [User Q4: over the bus]: `<adapter>::worker::{prepare, start, status, logs, stop, destroy}`. `worker::{exec, sync, wait, list, renew}` are reserved and not built in v1 (S6, 2). The six suffixes are constants used with `adapter_function_id`, not `OPERATIONS` rows. [Designer, plan triage]
- Compose calls the worker-VM family **directly** in its project namespace; the router does not forward it in v1. [Designer] Why:
  - each project's VMs stay on that project's adapter;
  - the router needs no caller identity, which a built-in `FunctionHandler` does not receive (`engine/src/function.rs:75-83` passes only `invocation_id, function_id, input, metadata`).
- `sandbox::adapter::info` is router-owned and internal. It returns `{adapter, namespace, contract, info|null}` for consumers' startup probes (9) and diagnostics. Compose never calls it (7.7). [Designer, plan triage]

### 5.3 Ephemeral family
- Shapes are today's daemon shapes (`sandbox_daemon/{create,exec,stop,list,catalog,run}.rs`, `fs/*.rs`), moved verbatim into the crate. [User]
- The router forwards opaque JSON, so the daemon's lenient forms keep working: three exec input shapes, env as a list or a map, base64 stdin.
- `exec.exit_code` stays `Option<i32>`; it is `null` on signal death (`exec.rs:151`).
  - ide declares it as `i32` (`ide/src/exec/sandbox.rs:23`). This pre-existing bug is fixed in ide.

### 5.4 Worker-VM family (v1) [Designer; builder may rename]
```rust
struct WorkerStartRequest {
    auth: String,                 // per-project token (7.8)
    state_key: String,            // hash(compose state dir + container key); same key = same persistent upper
    owner: Owner,                 // { kind: "compose", id: <project ns>, container: <key> }
    worker_name: String,          // container key -> III_WORKER_NAME
    source: WorkerSource,
    env: BTreeMap<String, String>,// complete guest env; body only, never argv or kernel cmdline
    engine_url: String,           // as seen from the host
    network: bool,
}
enum WorkerSource {
    Bundle { package: String /* manifest name, not the container key */,
             host_path: String /* the bundle in Compose's package cache */ },
    Local  { host_path: String, run_override: Option<String> },
}
struct WorkerStartResponse { vm_id: String, instance: u64 }
struct WorkerStatus { vm_id: String, instance: u64,
    state: State /* Preparing | Running | Exited | Stopped | Unknown */,
    exit_code: Option<i32>, signal: Option<i32>, started_at_ms: u64, ended_at_ms: Option<u64> }
```
No `resources` or `sync` field: the adapter reads resources from the manifest and always syncs local sources, as today. [Designer, plan triage]

The other operations:

| Operation | Behaviour |
|---|---|
| `prepare{auth, source}` | Idempotent image and base-layer pull. Long-running: Compose gives it 20 minutes, today's budget. Progress reporting [Builder] |
| `status{auth, vm_id, instance}` | Returns `WorkerStatus`; Compose polls it (7.4) |
| `logs{auth, vm_id, cursor?, follow, channel}` | Writes `{stream, line, ts_ms, cursor}` records to the channel |
| `stop{auth, vm_id, instance, grace_ms}` | Idempotent; `grace_ms: 0` kills; returns the final status |
| `destroy{auth, state_key}` | Refused while the VM is running |

### 5.5 Handles
- `sandbox_id` stays a UUID string, which consumers may validate. The adapter maps it to its own handle: PID, container id or machine id.
- `vm_id` is opaque and stable per `state_key`. `instance` increments on every boot.
- Every call that acts on a VM carries `instance`, so a stale stop cannot hit a newer boot. This replaces `ChildRecord{pid, birth}`.

### 5.6 Errors [User: keep S-codes]
- The wire format stays `ErrorBody{code: "invocation_failed", message: <SandboxErrorWire JSON>}` (`sandbox_daemon/errors.rs:438-470`).
- Consumers parse the message (`sandbox-code-runner/src/error.rs:106-143`), and the router passes adapter errors through untouched.
- Kept: S001-S004, S100-S102, S200, S210-S219, S300, S400.
- New router codes [Designer]: S301 AdapterUnavailable (retryable), S302 ContractMismatch, S303 NoAdapterConfigured (only on an engine no Compose file manages, S1), S304 OperationUnsupported.
- New worker-VM codes [Designer; renumbering allowed before v1]:
  - S500 reserved for capability errors, once capabilities exist (5.7)
  - S501 SourceRejected (bad manifest, `host_path` outside the allowed roots)
  - S502 StateBusy
  - S503 InstanceMismatch
  - S504 VmNotFound
  - S505 Unauthorized
  - S506 PrepareFailed (retryable)

### 5.7 Adapter info and handshake [Designer]

`AdapterInfo{adapter, adapter_version, contract:{major, minor}}`. Capability fields are cut from v1 (2): the only adapter supports every operation. They return as additive fields, with a minor bump, when the first adapter that lacks something ships; until then an adapter that cannot do an operation returns S304.

Handshake:
- **Router**: lazy and cached. On the first forward it calls `<adapter>::adapter::info`.
  - A different major gives S302.
  - The cache is cleared on S301 or `invocation_stopped`.
  - There is no attach protocol: the router finds the adapter by its ids.
- **Compose**: `adapter::info` is a live function, so its check runs at start time. At plan time Compose checks only that an adapter build exists for this host (7.2). Once the adapter is up, and before any `worker::prepare`, Compose calls `<adapter>::adapter::info` and checks the major (S302) and the name (7.2). A failed check fails the start of each VM container exactly as a failed spawn fails a process container today. The adapter and non-VM containers keep running.

### 5.8 Guaranteed vs adapter-defined

| Behaviour | Status |
|---|---|
| Exec is argv-only; timeout clamped, reported as S200; on timeout or disconnect the whole process group is killed (computer relies on it); 1 MiB cap per stream; ENOENT → 127, EACCES → 126 | Guaranteed |
| Runs as uid 0, in the request's `workdir` or else `/` (today the dispatcher inherits PID 1's `--workdir /`, `adapters.rs:113-121`, `iii-init/src/shell_dispatcher.rs:583`, *inference*); image `USER`/`WORKDIR`/`ENTRYPOINT`/`CMD`/`Env` not applied (sandbox-code-runner writes `/opt` and runs a system-wide pip) | Guaranteed in v1 [Designer] |
| `fs::*` semantics and S210-S219; `fs::read`/`write` streamed over channels | Guaranteed |
| Fail-closed allowlist; presets not shadowable; `catalog::list` returns what `create` accepts | Guaranteed (the image set is adapter config) |
| Guest can reach the engine (sandbox with `network: true`; worker VM at its `III_URL`) | Guaranteed (mechanism adapter-defined) |
| `localhost`/`127.0.0.1` → gateway rewrite; outbound-only network, no egress policy, all host loopback reachable | Builtin-defined (today's behaviour) |
| Sandbox lifetime across adapter restarts; idle-activity signal | Adapter-defined; consumers treat S002 as "re-create" |
| Root filesystem layout; persistent upper | Adapter-defined |
| Guest exit code in `WorkerStatus.exit_code` | Guaranteed when the adapter can observe it, else `null` |

## 6. Engine changes

### 6.1 Router [Designer a]
- Module `engine/src/workers/sandbox/{mod.rs, config.rs, router.rs, iii.worker.yaml, README.md, skills/SKILL.md}`. The interface docs and skill move here from `sandbox_daemon/`.
- Registered with `register_worker!("iii-sandbox", SandboxRouter, …, mandatory)`.
  - Keeping the name keeps the registry package, `engine::workers::list`, iii-directory skill lookup (`iii-directory/src/functions/skills.rs`) and `skills: [iii-sandbox]` working. [Designer's call S7]
- Registration: every ephemeral id plus `sandbox::adapter::info` is registered through `EngineTrait::register_function` (namespace `default`), with schemas from the crate.
  - With no binding, which happens only on an engine no Compose file manages (S1), the ids are still registered, so ownership stays pinned; calls get S303.
- `handle_function(id, input, metadata)`:
  1. No binding: return S303 "no sandbox adapter configured: this engine is not managed by a Compose file; set sandbox.adapter in its engine config (Compose-managed engines default to sandbox-builtin)".
  2. If `functions.get_handler(ns, adapter_function_id(adapter, id))` is missing: return S301 "adapter sandbox-builtin not connected in namespace my-proj; compose::add iii-sandbox in that project installs it".
  3. Run the handshake (5.7).
  4. Call `call_value_with_metadata_ns(ns, target, input, metadata)` (`engine/src/engine/mod.rs:2521-2569`) and return the result unchanged. Queue enqueue is the precedent (`mod.rs:1920-1968`). The router never deserializes payloads. The forward is a fresh invocation without the caller's timeout or cancellation (12).

### 6.2 Reserved prefix
- Today a WebSocket worker silently replaces an in-process registration:
  - `claim_in` checks only `function_owners` (`mod.rs:3152-3183`), and "In-process workers do not populate this map" (`mod.rs:395-399`);
  - `register_function_ns` only warns "…already registered… Overwriting." (`function.rs:194-208`);
  - the comment at `mod.rs:2271-2273` ("the ownership conflict with the builtin already blocks the shadow") does not hold for in-process built-ins (*inference*, from reading).
- Add `ENGINE_RESERVED_PREFIXES = ["sandbox::"]` beside the `engine::` check (`mod.rs:2267-2300`).
  - A WebSocket registration of `sandbox::*` **in `default`** gets `RegistrationRejected{FUNCTION_NAMESPACE_CONFLICT, owner_worker_id: ""}` and is logged as "sandbox::* is served by the engine's sandbox router; adapters register <adapter>::*".
  - Other namespaces stay allowed. [Designer]
- Fix the stale comment.

### 6.3 Config [Designer; default binding: Designer's call S1]
The Compose file that owns the engine:
```yaml
engine:
  sandbox:
    adapter: sandbox-builtin   # function prefix = the adapter's manifest name; default
    namespace: my-proj         # optional; default = this project's namespace
```
- On a Compose-managed engine the materialized `engine-config.yaml` (`managed_engine.rs:243-325`) always gets a `sandbox:` binding: the section's values, or `{ adapter: sandbox-builtin, namespace: <owner namespace> }` when the section is absent. [Designer's call S1]
- An engine no Compose file manages (External mode: the file has no `engine:` section, `crates/iii-compose/src/lib.rs:439-459`) gets no binding. Its operator must set `sandbox:` in that engine's config, or its consumers get S303.
- `RawEngineSpec` (`crates/iii-compose/src/config.rs:1017-1027`) and `EngineSpec` (`config.rs:328-333`) gain `sandbox`.
- `EngineConfig` (`engine/src/workers/config.rs:57-68`, `deny_unknown_fields`) gains `#[serde(default)] sandbox: Option<SandboxRouterConfig>`, with `deny_unknown_fields`, adapter names `[a-z0-9-]+` and the namespace checked by `iii_compose::namespace::check`.
- One `normalize_worker_entries` helper replaces the duplicated mandatory-injection loops (`engine/src/workers/config.rs:713-726`, `reload.rs:121-143`). It injects the `iii-sandbox` entry with `config = EngineConfig.sandbox`. Today both loops inject `config: None`, and a factory sees only its entry's config (`fn(Arc<Engine>, Option<Value>)`, `engine/src/workers/registry.rs:15-18`). Using the helper on both paths keeps a reload from unbinding the router.
- Bad values fail with `[INVALID_SANDBOX_CONFIG]`.
- Changing the section needs a Compose restart (`EngineRestartRequired`).
- Adapter settings are not engine config (7.6).

### 6.4 Validation and migration errors [User 6]
- Remove `"iii-sandbox"` from `CONFIGURABLE_ENGINE_WORKERS`.
- Compose `validate_engine` (`config.rs:473-498`) raises `SANDBOX_CONFIG_MOVED` before its generic check: "engine.workers.iii-sandbox was removed in iii 0.25: sandboxes now run in the sandbox-builtin worker, which Compose installs for you. Replace the block with `engine.sandbox.adapter: sandbox-builtin` (optional, the default) and move image_allowlist/custom_images/default_* to `containers.sandbox-builtin.config_override`. Guide: https://iii.dev/docs/upgrading/sandbox-worker". It also prints the replacement YAML built from the old block.
- Update the text of `UnsupportedEngineWorker` (`error.rs:46-50`). `EngineWorkerIsBuiltin` (`daemon.rs:2178-2191`) keeps its text: it names no worker, and the rewrite below runs first. [Designer, plan triage]
- `compose::add` rewrites an `iii-sandbox` request to the configured adapter's package request in `add_configured_scoped`, before planning, so it never reaches the `EngineWorkerIsBuiltin` check. [Designer, plan triage] `compose::add iii-sandbox` therefore keeps working after `iii-sandbox` leaves `CONFIGURABLE_ENGINE_WORKERS`, and the remediation texts in 6.1, 7.7 and 9 stay valid.
- Engine: add an `"iii-sandbox"` arm to `migration_guidance` (`engine/src/workers/config.rs:206-222`).

### 6.5 Engine deletions (iii repo)

**Code**
- `engine/src/workers/{external.rs, secure_temp.rs}`; `engine/src/lib.rs:31,40`.
- `engine/src/workers/config.rs`: `ExternalProcessWorker` (:263-313), create step 3 (:407-427), external port/config setup (:731-764) and its tests (:2552-2755), `iii-sandbox` fixtures (:1287, :1304), the `is_external_process` guard (:516-518, plus `traits.rs:96-98`).
- `Engine.worker_manager_port` and `config_path` (`engine/mod.rs:435-447, 514-544`), once other users are ruled out.
- `engine/src/cli/registry.rs`: both the `iii-init` (:47-61) and `iii-worker` (:100-117) `BinarySpec`s. `engine/src/cli/platform.rs`: the worker libc choice (:54-73) and its tests (:243-277), plus the `iii-init` assertion at :281.
- `engine/src/cli/project/mod.rs` [Designer's call S4]: `LEARN_III_BASE_IMAGE`, `start_image_prefetch` and `declared_base_images` (:314-388); their uses in `run_learn_iii` (:407, :420-424, and the wait loop at :442-452); their tests (:1214-1266). Leaving any of them makes clippy's `-D warnings` fail. `bin_resolve.rs` stays.

**Tests**
- Delete `engine/tests/external_known_sandbox.rs`.
- Delete the `worker_target_for_host` cases in `engine/tests/install_sh_unit.bats:218-267`.
- Rename the `iii-sandbox` stand-in in `engine/tests/config_reload_e2e.rs:96, 529-573`.

**Crates and assets**
- `crates/{iii-worker, iii-init, iii-network, iii-filesystem, iii-supervisor, iii-shell-proto, iii-shell-client}` and their workspace members (`Cargo.toml:4-22`).
- `engine/firmware/`; `scripts/{build-worker-musl.sh, worker-musl.md, sandbox-isolation-e2e.sh}`.
- `.cargo/config.toml` stays. Its `rust-lld` musl linkers are workspace-wide, and the engine's own `x86_64-unknown-linux-musl` builds (`ci.yml:378-405`, the `_rust-binary.yml:110` default matrix) link through them (*inference*). `sandbox-builtin` gets a copy (8.2).

**CI, release, install**
- `release-iii.yml`: `init-build`, `firmware-upload`, `worker-release` and their `needs` edges (:652, :701, :755, :840-873). `publish-builtin-workers` keeps `iii-sandbox` with `worker_dir: engine/src/workers/sandbox`.
- `alpha-release.yml`: delete `init-build` (:333-396) and `worker-release` (:415-434). `engine-release` (:398-413) stays. In `publish-builtin-workers` (:436), drop `worker-release` from `needs` and point the `iii-sandbox` row at `engine/src/workers/sandbox`.
- `_rust-binary.yml:49, 221-237, 295-311`: only the `init_artifacts` input and the musl worker steps; its generic inputs (`features`, `targets`, `system_deps`) stay. [Designer, plan triage]
- `ci.yml` jobs at :65, :411, :480, :523-762 and the install step at :160-186. The `libcap-ng-dev` installs left in other jobs go once `cargo metadata` lists no `capng`. [Designer, plan triage]
- `docker-worker-images.yml`.
- `Makefile`:
  - delete `install-iii-worker` (:50-54) and the VM block (:86-135: `init-build-*`, `sandbox`, `sandbox-debug`);
  - edit `.PHONY` (:17-20);
  - edit `engine-test` and `coverage` (:56-67): drop the `install-iii-worker` prerequisite and the `cargo test -p iii-worker …` line;
  - edit `build:` (:191): replace the `sandbox` prerequisite with `cargo build --release -p iii`, and drop the worker echo lines;
  - the SDK, lint, fmt, typecheck and console targets (:137-190) stay.
- `engine/install.sh:535, 623-674, 700-736, 800-815, 848-865`.

## 7. Compose changes

### 7.1 Need detection
`iii_compose::sandbox::needs(file) -> Vec<Need>`. Every need installs the adapter (7.2); consumers that only call `sandbox::*` declare nothing and get S301 with the fix when no adapter runs [User, revising Q2]:
- `VmBundle(key)`: the resolved package kind is `bundle`, known offline from the lock (`lockfile.rs:168-188`).
- `VmLocal(key)`: a `path://` manifest has `runtime.base_image`.
- `Explicit`: the owning file sets `engine.sandbox`. A managed engine is bound either way (S1). Setting the section only asks Compose to install that adapter, even when its `namespace` names another project (accepted). [Designer, plan triage]
- `compose::add iii-sandbox` is not a need: it adds the adapter container itself (6.4). [Designer, plan triage]

### 7.2 Auto-install [User 5; mechanics Designer]
The adapter is **persisted as a generated container**, not injected at plan time, so lock, `--frozen`, `ps`, logs, restart, `config_override` and console config all work unchanged:
```yaml
  # added by compose::add
  sandbox-builtin:
    worker: package://sandbox-builtin
    version: '1.0.0'
```
- The marker is exactly `MARKER` (`edit.rs:115`), because `is_generated_container` compares the line verbatim (`edit.rs:375`). Any other text makes it an operator-declared container, which Compose never re-resolves or removes. The reason ("sandbox adapter for: my-bundle") is printed by `up`, never written into the file.
- Implementation: reuse `dependencies::plan(Policy::Add)`, `edit::upsert_container` and `lockfile::prepare_with_versions`. No lock-graph entry and no GC for it: `compose::update` and `compose::remove` remove the generated adapter when `needs(file)` becomes empty. [Designer, plan triage]
- **Version.** [Designer's call S2]
  - Compose ships inside the engine and asks the registry for `^<CONTRACT_MAJOR>`, a constant compiled in from the contract crate. It writes the exact version it resolved.
  - `compose::update` re-resolves the generated adapter within the same range, instead of moving it like other generated containers (`Policy::Update`, `dependencies.rs:61-70`).
  - So an older engine never auto-installs an adapter with a newer contract major. No registry schema change in v1.
  - An operator-pinned adapter of another major gets S302 from the handshake.
- **Identity.** [Designer]
  - `engine.sandbox.adapter` is the function prefix. It equals the adapter's manifest name (8.1) and `AdapterInfo.adapter`.
  - Compose recognizes an existing adapter container (registry or `path://` dev checkout) by its manifest name, never by its container key. The key only becomes `III_WORKER_NAME` (`spawn.rs:241`).
  - Once the adapter is up, Compose checks that `AdapterInfo.adapter` equals the prefix, and fails with `SANDBOX_ADAPTER_MISMATCH` otherwise.
  - The operator's declaration wins.
- An adapter that would itself be a VM container fails `dag::validate_dependencies` as a cycle (7.3); no separate error.
- **Triggers.**
  - `compose::add` / `update` run the step in the same atomic edit.
  - `up` runs it in `prepare_start_project`, under the mutation lock (`daemon.rs:717-737`). It writes the container and prints one line. [Designer's call S5]
  - `iii compose build` runs it too, so `build` then `up --frozen` works. [Designer, plan triage]
  - `compose::add iii-sandbox` is rewritten to the adapter's request before planning (6.4). [Designer, plan triage]
  - Outcomes by case:

| Supported host, adapter not declared | No adapter build for this host (`UnsupportedPlatform`, `registry.rs:611`; also the registry's `/resolve` `unsupported_platform` refusal with no fallback [Designer, plan triage]) | `--frozen`, adapter not declared |
|---|---|---|
| Write and install | Fail `UNSUPPORTED_PLATFORM` | Refuse `SANDBOX_ADAPTER_NOT_DECLARED`, printing the one-line fix [S5] |

### 7.3 Start ordering
- `sandbox::wire(&mut ComposeFile, adapter_key)` appends the adapter key to the **in-memory** `start_after` of every VM container, then re-runs `dag::validate_dependencies`. It is called in callers that return `Result`: `Project::open`, `commit_prepared_packages`, and `compose::add`/`remove` before they persist. `reconcile_file` and `reconcile_removals` return none, so their callers wire instead. [Designer, plan triage] Nothing is written to operator-owned containers.
- The adapter starts first and stops last. An adapter crash cascades to its VM containers (`project.rs:381-639`).

### 7.4 VM containers through the contract
- Replace `vm_command`, `prepare_vm` and `worker_binary` (`lifecycle.rs:1199-1355`) with a client built on the crate.
- Add `enum Child { Process(Supervised), Vm(RemoteVm) }`, with the same `poll`, `wait` and `stop`.
- Supervision types need real changes; a drop-in variant is not enough. [Designer]
  - `process::Outcome` (`process/mod.rs:71-75`, today `Exited(ExitStatus) | Running`) gains `Unknown`, and `reap_scoped` and `reconcile_after_reconnect` handle it.
  - A VM exit becomes an `ExitStatus` through `ExitStatusExt::from_raw`. That call is unix-only, which is fine because VM containers never run on Windows. So `wants_restart(status.code().unwrap_or(-1))` (`project.rs:404`) is unchanged. [Builder: or make `Outcome` carry `Option<i32>`]
  - `ChildRecord.pid` and `.birth` (`state.rs:82-92`) are persisted and non-optional today. They become `Option` with `#[serde(default)]`, so existing state files still load.

| Today | Through the contract |
|---|---|
| `__bundle-prepare` / `__local-prepare` | After the adapter is up: `adapter::info`, then the major and name checks (5.7), then `worker::prepare` for every VM container concurrently (20-minute budget) |
| Spawn `__vm-boot` | `worker::start` with: `state_key = hash(<compose-dir>/.iii/compose/<ns>, key)`; `source.package` = manifest name (fixes the coupling at `bundle_prepare.rs:29-31`); env = today's `spawn_plan` env; bundle `host_path` = its directory in Compose's package cache |
| `waitpid` → `wants_restart(code)` | A per-VM task polls `worker::status` every second into a watch channel, and `poll()` stays non-blocking. **A bus error yields `Outcome::Unknown`, and `Unknown` never restarts or cascades.** S504 after an adapter restart counts as an exit with no code |
| SIGTERM, `stop_timeout`, SIGKILL | `worker::stop{instance, grace_ms = stop_timeout}`. Builtin: SIGTERM to `__vm-boot`, wait `grace_ms`, then SIGKILL, as today; no `iii.control` `Shutdown` [Designer, plan triage] |
| Pipes → rotating log file | `worker::logs{follow, channel}` feeds a record-based entry point in `logs.rs`. `compose::logs` and tails are unchanged; Compose persists no log cursor [Designer, plan triage] |
| `ChildRecord{pid, birth}` | `ChildRecord.vm = {adapter, namespace, vm_id, instance}`; `reconcile_recovered` uses `worker::status` |
| Readiness, restart policy, cascade | Unchanged and owned by Compose. A restart is `stop` then `start` with the same `state_key` |
| `rm` keeps VM state (`lifecycle.rs:675-716`) | `rm` calls `worker::destroy` |
| `BundleNeedsAVm` on non-unix (`registry.rs:417-423`) | Kept as the Windows guard [Designer, plan triage]. Plan time: no adapter build for this host (7.2). Start time: the handshake (5.7) |

Adapter readiness means its worker name is registered, which is not proof that its functions are. Compose therefore retries `function_not_found` for up to 10 s. [Builder]

### 7.5 Prefetch [Designer's call S4]
- Deleted. `iii project init --learn-iii` no longer prefetches; plain `iii project init` never did. The first `up` pulls images through `worker::prepare`. The dead code goes with it (6.5).

### 7.6 Adapter config [Designer]
- The adapter is configured like any worker. Layers, lowest first: the shipped registry `config`, then `configuration::get(III_CONFIG_NAME)`, then `config_override` (`lifecycle.rs:1432-1500`). It is console-editable.
- The shipped config is today's `sandbox_daemon/iii.worker.yaml` (`auto_install: true`, `image_allowlist: [python, node]`). `SandboxConfig`'s serde defaults (`config.rs:72-74`) fill only the missing keys.
- Keys:
  - today's `SandboxConfig`: `auto_install`, `image_allowlist`, `default_idle_timeout_secs`, `max_concurrent_*`, `max_exec_timeout_ms`, `default_cpus`, `default_memory_mb`, `per_image_caps`, `custom_images`;
  - the old `III_VM_*` tuning stays as the four env vars the adapter already inherits (`worker_manager/libkrun.rs:44-65`), not config keys. [Designer, plan triage]
- No new `worker_vm` keys: the adapter reads manifest `resources` within today's caps, as today. [Designer, plan triage]
- The adapter reads `III_URL` and SDK-managed identity. It reads its config once at startup, as the daemon does today; a change applies when the adapter restarts (8.3). `--config` and `III_ENGINE_URL` are dropped.

### 7.7 Namespace and multi-project rules
- The adapter runs in its project's namespace, which may be `default`.
- **Router binding.** [Designer's call S1]
  - There is at most one binding per engine. `EngineAlreadyOwned` guarantees at most one owner, not exactly one.
  - It comes from the Compose file that manages the engine: `sandbox-builtin` in the owner's namespace unless `engine.sandbox` says otherwise.
  - An External-mode engine has no binding unless its operator configures one (6.3), so its consumers get S303.
- **Consumer calls.** [User Q1]
  - One adapter per engine, running in the binding namespace, serves every project's `sandbox::*` calls. An attached project installs nothing for them; with the owner's adapter down, calls get S301 naming that namespace.
  - Rejected alternative: route each call to the caller's project adapter. That needs the engine to pass caller identity to built-in handlers, which it does not do today (`function.rs:75-83`).
- **Worker VMs are per project.** An attached project with a VM need installs `sandbox-builtin` (`DEFAULT_ADAPTER`) in its own namespace and runs its own VMs there. It never queries `sandbox::adapter::info`. [Designer, plan triage]

### 7.8 Guarding the worker-VM family [Designer]
`host_path` lets a caller pull host directories into a VM, and `sandbox-builtin::worker::*` is callable by any worker. Two guards:
- **Token.** Compose generates a per-project token, stores it in its 0600 state and passes it to the adapter container as the reserved env `III_SANDBOX_WORKER_TOKEN`. Every `worker::*` request carries it in `auth`; a mismatch gives S505.
- **Host roots.** The builtin accepts `host_path` only under two roots: the project's compose dir and the Compose package cache. A `path://` VM worker outside them gets S501 naming the roots, and the upgrade guide says so. [Designer, plan triage] [Builder: delivery mechanism]

### 7.9 No migrate command [Designer, ponytail review]
- `SANDBOX_CONFIG_MOVED` (6.4) prints the replacement YAML: the generated `sandbox-builtin` container (exact marker, version as in 7.2) with the old block copied into `config_override` (the keys are the same).
- A configuration entry stored under `iii-sandbox` is not moved. The upgrade guide (9) says how to re-enter it under `sandbox-builtin`.
- The guide lists the old `<compose-dir>/.iii/compose/<ns>/vm/<key>/` directories for deletion.

## 8. Builtin adapter worker: `sandbox-builtin`

### 8.1 Naming and layout [Designer]
- **One name everywhere.** Folder, manifest name, Cargo package and binary are all `sandbox-builtin`, as the repo requires (`validate_worker.py:36, 80-82`). The function prefix is `sandbox-builtin::`, which is what `engine.sandbox.adapter` names (7.2).
- **Why not `iii-sandbox`.** That name stays with the router (S7), which lets `engine.workers.iii-sandbox` fail with a clean migration error.
- **One executable.** The root command serves the adapter, and the hidden `__vm-boot` re-executes `current_exe()`, so the VMM, init and firmware always come from one build. A single executable is also required because Compose runs the first executable in the archive (`registry.rs:1287-1310`).
- **Inner crates.** They keep their names under `crates/` with `publish = false`, so the guest ABI is unchanged.

```
sandbox-builtin/
  Cargo.toml        # 1.0.0 (major = CONTRACT_MAJOR, S2); [workspace] members=["crates/*"]; iii-sdk/iii-helpers "=0.24.6";
                    # iii-sandbox-contract (crates.io); msb_krun "=0.1.16"; release profile from iii Cargo.toml:62-66
  Cargo.lock        # seeded from the iii lock (keeps msb_krun_devices 0.1.16; 0.1.39 breaks musl)
  .cargo/config.toml  iii.worker.yaml (deploy: binary)  entitlements.plist  README.md
  firmware/         # libkrunfw blobs as-is + README (5.2.1, ABI 5, kernel 6.12.68, rebuild recipe) + SHA256SUMS
  vendor/  images/{node,python}  scripts/  src/  crates/  tests/
```

### 8.2 What moves

| From (iii repo) | Size | To |
|---|---|---|
| `crates/iii-worker/{src,tests,examples,vendor,images,build.rs}` | 55.6k src / 14.7k test LOC | `sandbox-builtin/`; the legacy CLI is then deleted (8.4) |
| `crates/{iii-init, iii-network, iii-filesystem, iii-supervisor, iii-shell-client, iii-shell-proto}` | about 18.8k src LOC | `sandbox-builtin/crates/` |
| `engine/firmware/*` | 69 MB (three blobs, 66 MiB on disk); about 33 MB added to the git pack | `sandbox-builtin/firmware/`, committed as-is, no LFS [User] |
| `scripts/sandbox-isolation-e2e.sh`, `engine/tests/vm_platform_e2e.rs` | | `scripts/`, `tests/`. `build-worker-musl.sh` and `worker-musl.md` are not copied: no musl in v1 [Designer, plan triage] |
| `.cargo/config.toml` musl linkers | | Copied to `.cargo/`; the iii copy stays (6.5) |
| `sandbox_daemon/README.md` | 549 lines | Moves to the router in step 2 (6.1); the S-code reference goes to the contract docs. `sandbox-builtin` gets its own README and no skill: the router owns the sandbox skill [Designer, plan triage] |

The crate cannot be split by module: from the VM entry points, imports reach 53k of its 55.6k lines through `local_worker.rs`. Move it whole, then delete the legacy code with modules made `pub(crate)` so dead code shows up.

### 8.3 Required source edits
- `build.rs:24-26` reads `firmware/` and checks the ELF/Mach-O header and `SHA256SUMS`, instead of `len > 1`. Update the path in `tests/musl_firmware.rs:10-12`.
- Delete `cli/builtin_defaults.rs:39` (an `include_str!` of an engine yaml), which only legacy code uses.
- `vm_boot_program()` (`worker_manager/libkrun.rs:104-135`) always uses `current_exe`. Today a binary not named `iii-worker` would silently boot an older `iii-worker` found on PATH.
- `CARGO_BIN_EXE_iii-worker` becomes `sandbox-builtin` (4 test files).
- Remove the firmware and init download fallbacks (`firmware/download.rs:76-82, 229-232`). A missing embed fails closed.
- Register `sandbox-builtin::*` and `adapter::info`, using the crate's types (including its `StreamChannelRef`) and errors.
- Config through the bus: today the daemon reads a file once at startup and fails closed (`config::load_config(&args.config)`, `cli/sandbox_daemon.rs:12-13`). That becomes one `configuration::get` at startup, still failing closed. A change applies on the next adapter restart, which cycles every sandbox and VM container through the lifeline (12).
- Implement `worker::*` on `start_worker_impl` (`local_worker.rs:1055-1584`):
  - state under `~/.iii/sandbox/workers/<hash(state_key)>/`;
  - `__vm-boot` spawned with a lifeline;
  - VMM stdio captured to a cursor-addressable file;
  - env passed to `__vm-boot` as today (`--env`, then the kernel cmdline). That env, including the inherited host env (S3), shows in `ps`, and an oversized one overflows `CMDLINE_MAX_SIZE` (`local_worker.rs:1360-1364`); both are pre-existing and stay out of v1 (2, 12);
  - the `localhost` → gateway rewrite kept.

### 8.4 `iii-worker` subcommands

- `sandbox-daemon` becomes the root command; `__vm-boot` stays hidden; `__bundle-prepare`, `__local-prepare` and `__pull-images` become `worker::start` and `worker::prepare`.
- `sandbox run/create/exec/list/stop/upload/download` are deleted in step 1, when `sandbox-isolation-e2e.sh` switches to `iii trigger` (10).
- Everything else is deleted: `add, remove, reinstall, update, clear, start, stop, restart, list, sync, verify, status, logs, init, gen-cli-docs, __watch-source`, and `exec` [Designer's call S6]. That also drops `iii-clap-docs` and `scaffolder-core`.

### 8.5 Build and release (workers repo)

**`.deploy/workers.yaml`** — add a `sandbox-builtin` entry:
- `kind: rust-binary` on Rust 1.97.1, `publish: true`, first release 1.0.0 (S2);
- targets `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`. [User Q3]
  - Leaving out musl is a product choice, not a firmware limit. Today `iii-worker` also ships `x86_64-` and `aarch64-unknown-linux-musl` builds with the same firmware (`release-iii.yml:324`; the x86_64 firmware is aliased to the musl triple at :229-231).
  - The workers pipeline has no aarch64-musl or dynamic-musl build. Musl comes later (12);
- `windows_exception: "microVMs need KVM or Hypervisor.framework"`;
- new fields `features: [embed-init, embed-libkrunfw]`, `prebuild: [scripts/build-init.sh]` and `macos_entitlements: entitlements.plist`.

**Build scripts**
- `.github/scripts/deployment_compiler.py:265`: accept the three new fields.
- `.github/scripts/deployment_train.py:510-531`:
  - run `prebuild` with `BUILD_TARGET` set;
  - pass `--features`;
  - on darwin, run `codesign --sign - --entitlements <plist> --force <bin>` and verify it with `codesign -d --entitlements -`.
- New `scripts/build-init.sh` builds `iii-init` for `<arch>-unknown-linux-musl` with `rust-lld`, into the path `iii-filesystem/build.rs:20-26` expects.
- `_deploy-build.yml`:
  - install `libcap-ng-dev`;
  - aarch64-gnu: `libcap-ng-dev:arm64` with `LIBRARY_PATH`, because the step blanks `RUSTFLAGS`. *Inference*: `LIBRARY_PATH` is enough; prove it with a dry run.

**CI and images**
- `ci.yml`:
  - an exception to the root-only `--all-features` run (:265-278) that runs `--workspace --features embed-init,embed-libkrunfw`;
  - a new `sandbox-builtin-macos` job;
  - a new `sandbox-builtin-vm-linux` job with KVM (model: engine `ci.yml:523-571, 664-727`). It runs the VM tests, the adapter-level worker-VM e2e and the isolation e2e. The section-11 Compose e2e runs in the iii repo (11). [Designer, plan triage]
  - Unlike its model, which is `continue-on-error: true` (`ci.yml:523-526`), `sandbox-builtin-vm-linux` has no `continue-on-error`. Making it a required check that gates every registry release is a branch-protection step the plan index lists. [Designer, plan triage]
- New `sandbox-builtin-images.yml` (copied from the engine's `docker-worker-images.yml`; needs the Docker Hub secrets). Its only trigger is `workflow_dispatch`, so it never pushes while iii's `docker-worker-images.yml` exists; step 3 deletes that one. [Designer, plan triage]
- `.gitattributes`: `sandbox-builtin/firmware/** binary`.

### 8.6 Host prerequisites
- **macOS, Apple Silicon only.**
  - The binary is ad-hoc signed at build time with `com.apple.security.hypervisor` and `com.apple.security.cs.disable-library-validation`.
  - The runtime self-sign check (`platform.rs:18-68`) then returns early. It still signs unsigned dev builds in place; those never sit in the Compose cache. [Designer, plan triage]
  - Signing in place would break Compose's tree-hash check (`registry.rs:750-777`) and trigger a re-download on every `up` (*inference*).
- **Linux, glibc.**
  - The Compose user needs read/write access to `/dev/kvm` (`vm_boot.rs:248-263`).
  - `libcap-ng0` must be installed (*inference*: dynamic linking).
- **Anything else** gets a clear `UnsupportedPlatform` error when a project needs the adapter (7.2).

### 8.7 Tests that move
- Sandbox tests, about 3.8k LOC: `sandbox_*`, `fake_sandbox_relay`, `fixtures/sandbox_error_codes.json`.
- VM tests, about 3.5k LOC: `vm_*`, `firmware_integration`, `musl_firmware`, `bundle_vm_command`, `local_vm_command`, `oci_*`, `worker_manager_integration`, `project_detection_integration`.
- `common/`, and the manifest and resource-cap cases from `bundle_worker_integration`.
- About 6.9k LOC of legacy tests are deleted.

## 9. Migration

**User steps**
1. `iii update` stops installing `iii-worker`/`iii-init` (both `BinarySpec`s go, 6.5) and notes that `~/.local/bin/iii-worker` and `iii-init` can be removed; it deletes nothing. [Designer, plan triage]
2. `up` on a file with `engine.workers.iii-sandbox` fails with `SANDBOX_CONFIG_MOVED` (6.4). The error prints the replacement YAML (7.9).
3. The first `up` installs `sandbox-builtin`. VM state moves to adapter-owned directories, so the first boot of each VM reinstalls dependencies. The upgrade guide lists the old directories.
4. A stray `iii-worker sandbox-daemon` can no longer take over `sandbox::*` (6.2).

No step for `iii worker`: clap has rejected it since 0.23 (`engine/src/main.rs:680-686` at `a1d08d8b0`), so no pointer is added. [Designer, plan triage]

**Consumers**
- computer, sandbox-code-runner, ide and harness keep pinning `default`, where the router lives.
- They declare nothing new (7.1).
- ide accepts `exit_code: null`.
- sandbox-code-runner's startup probe (`src/main.rs:128-155`) asks `engine::functions::info` about `sandbox::create`. The router always registers that id, so the probe would log "iii-sandbox detected" even with no adapter.
  - The probe moves to `sandbox::adapter::info` and checks `info != null`. On engines older than 0.25 it falls back to today's probe.
  - Its remediation text (`compose::add worker=iii-sandbox`) stays valid (7.2).

**Docs to update**
- iii repo, under `docs/next/` only; 0.25.0 rotates it to Latest [Designer, plan triage]:
  - `docs/next/creating-workers/sandboxes.mdx`
  - `docs/next/using-iii/{engine,compose,workers}.mdx`
  - `docs/next/upgrading/workers-to-compose.mdx`
  - `docs/next/cli-reference/index.mdx`
  - their `.skill.md` twins
  - `skills/iii-engine-config/SKILL.md:36,60` and `skills/iii-core-primitives/SKILL.md:50`
  - a new `docs/next/upgrading/sandbox-worker.mdx`
- Workers repo [Designer, plan triage: only text that tells users to configure `engine.workers.iii-sandbox` changes; mere name mentions stay]:
  - `template/worker-compose.yaml:6-17`, which declares `sandbox-builtin`; `up` on a host with no adapter build then fails with `UNSUPPORTED_PLATFORM` (accepted; the template README says to remove the container there). `template/sync.sh` exempts `sandbox-builtin`. [Designer, plan triage]
  - `ide/tests/e2e/*` stays standalone: its mocks register `sandbox-mock::*` instead of `sandbox::*`, and its `config.yaml` replaces the `iii-sandbox` entry with `sandbox: {adapter: sandbox-mock, namespace: ide-e2e}`. `EXEC_SANDBOX_CASES`, `III_SANDBOX_FS_NAMESPACE` and the ide S-code table stay unchanged. [Designer, plan triage]
  - the harness `shell_coder_sandbox` scenario, and `harness/tests/e2e/run-ci.sh:15`, whose companion check requires `iii-init` and `iii-worker` and fails once `engine/install.sh` stops shipping them. Harness launchers that call `iii worker` or `worker::add` are already broken since 0.23 and stay out of scope. [Designer, plan triage]
  - `computer/README.md`, `computer/images/desktop/Dockerfile` and `computer/ui/src/page/Launcher.tsx` (they configure `engine.workers.iii-sandbox`); `computer/src/config.rs:50-52` and `ade/ui/src/configuration-forms/specs/scalar.ts:149` (they send users to iii-sandbox's own configuration, which now belongs to `sandbox-builtin`)
  - `.github/workflows/_worker-e2e.yml:251-253`
  - `README.md:127` and `ide/{README.md, ARCHITECTURE.md, skills/SKILL.md, config.yaml}` hold only name mentions and stay

## 10. Rollout and sequencing

| Step | Repo | Lands | Why main stays green |
|---|---|---|---|
| 0 | iii | `iii-sandbox-contract` and its publish job, shipped in the next engine patch | No behaviour change |
| 1 | workers | `sandbox-builtin` (moved code, `sandbox-builtin::*`, `worker::*`) and the 8.5 pipeline. Adapter tests and `sandbox-isolation-e2e.sh` call `sandbox-builtin::*`; the script goes through `iii trigger`, with the id prefix in a variable, and the hidden `sandbox` CLI is deleted. Released to the registry as 1.0.0 once the required `sandbox-builtin-vm-linux` passes | Nothing consumes it yet; a 0.24.x engine has no router, so nothing can call `sandbox::*` on it |
| 2 | iii | Router, reserved prefix, `engine.sandbox` with its default binding, migration errors, Compose client, auto-install; remove `KNOWN_EXTERNAL` and the prefetch | Router tests use a fake adapter (`fake::*`); the Compose e2e (11) pulls `sandbox-builtin` 1.0.0 |
| 3 | iii | Delete the VM crates, firmware, jobs, `engine/install.sh` sections and Makefile targets (`.cargo/config.toml` stays); update docs | Nothing references them after step 2 |
| 4 | iii | Release engine 0.25.0 (steps 2 and 3) | The adapter is already in the registry |
| 5 | workers | The sandbox-code-runner probe, the ide `exit_code` fix, template, the ide e2e mocks renamed to `sandbox-mock::*` with `sandbox: {adapter: sandbox-mock, namespace: ide-e2e}`, harness text and `run-ci.sh`, `III_RELEASE_TAG` pins, `_worker-e2e.yml`, the isolation e2e on `sandbox::*` on the script's own engine with an explicit `sandbox:` binding, not under Compose [Designer, plan triage] | Needs 0.25.0 |

Steps 2 and 3 are separate PRs in one engine release. Never release an engine without `iii-worker` before `sandbox-builtin` is in the registry.

## 11. Success criteria and verification

**Primary acceptance test** [User 7]

On Apple Silicon macOS and on Linux x86_64 with KVM, with no `iii-worker` on PATH, a fresh project declaring a registry `bundle` worker runs `iii compose up`. Compose writes and installs `sandbox-builtin`, starts it first, and calls `worker::prepare` then `worker::start`. The bundle boots in a libkrun VM whose `__vm-boot` is a child of `sandbox-builtin`, not of Compose. The guest registers as `(ns, key)` and reaches ready, and its output appears in `compose::logs`. Killing the guest worker triggers the restart policy, `down` stops the VM within `stop_timeout`, and `rm` destroys its state. The same must pass for a `path://` worker with `runtime.base_image`, including source sync. This runs in the iii repo, in step 2's `compose-sandbox-e2e-linux` job, which pins `sandbox-builtin` and the registry bundle `onboarding` at fixed versions. Making it a required check is a branch-protection step the plan index lists. [Designer, plan triage]

**Regression checks**

Consumer code is unchanged, apart from the ide `exit_code` fix and the sandbox-code-runner probe (9).
- computer: a sandbox desktop session (screenshot, click, re-attach after restart).
- sandbox-code-runner: Python and Node runs, including the engine dial-back via `localhost`.
- ide: `shell::exec` with a sandbox target, and `shell::fs::*` forwarding.
- The ADE and computer pickers list `sandbox::catalog::list`.
- harness: `shell_coder_sandbox`.
- One `sandbox::exec` smoke through the router inside the section-11 Compose e2e.
- Error JSON matches `fixtures/sandbox_error_codes.json`.

**Engine and Compose tests**
- `sandbox::*` registration is refused in `default` and allowed elsewhere.
- Forwarding across namespaces passes errors and metadata through.
- S301 and S302. S303 only on an engine whose config has no `sandbox` key. A Compose-managed engine without `engine.sandbox` is bound to `sandbox-builtin` (S1), and a config reload keeps the binding.
- `invocation_stopped` on adapter disconnect.
- Both migration messages, and the replacement YAML printed for a 0.24 file.
- The generated adapter container: exact marker, removed by `update`/`remove` once `needs(file)` is empty, resolved within `^<CONTRACT_MAJOR>` (S2).
- `--frozen` refuses an undeclared adapter that a need requires.
- A registry fixture with no build for the host: a need fails with `UNSUPPORTED_PLATFORM`.
- The contract's `StreamChannelRef` serializes to the same JSON as iii-sdk's.

**Release check**
- The 0.25.0 GitHub release has no `iii-worker-*`, `iii-init-*` or `libkrunfw-*` assets.
- `cargo metadata` for the iii workspace lists no `msb_krun` package.
- `engine/install.sh` and `iii update` fetch neither `iii-worker` nor `iii-init`.

## 12. Risks and questions left to the builder

### Risks

| Risk | Mitigation |
|---|---|
| VM liveness now rides on the bus: an engine outage looks like a VM exit | Bus errors become `Unknown`, which never restarts; the builtin's VMs outlive the reconnect window, since their lifeline is the adapter (`project.rs:277-342`) |
| `host_path` reads host directories; adapter ids bypass the router | Token and host roots (7.8) |
| VM guests keep inheriting the whole host environment (`spawn.rs:161-166`) [Designer's call S3]. Today msb_krun passes that env on the kernel cmdline, and an oversized env overflows `CMDLINE_MAX_SIZE` and panics the VM builder (`local_worker.rs:1360-1364`). A cloud adapter would ship the env off the machine | Pre-existing and unchanged in v1 (8.3). The env now travels in the request body to the adapter; the policy and the cmdline transport are revisited before a cloud adapter |
| Graceful stop slows `down` and exposes workers that ignore SIGTERM | `grace_ms = stop_timeout`; `0` still kills |
| Orphaned persistent uppers; the first boot reinstalls dependencies | Documented (9). [Builder: optional one-time import] |
| An adapter crash or restart kills every VM through the lifeline, including the restart a config change needs (8.3) | Orderly cascade through the implicit edges; restart backoff; config changes are rare |
| `prepare` exceeds normal invocation timeouts | Separate operation with an explicit 20-minute caller timeout |
| The x86_64 Linux engine is a musl build, so Compose tries musl packages first (`registry.rs:228-241`); a dynamic-musl adapter on Ubuntu fails | No musl artifact at launch [User Q3]. A later musl adapter must be static, or Compose must stop preferring musl on glibc hosts |
| The macOS self re-sign churns the Compose cache | Sign at build time (8.6) |
| Build drift: the iii repo builds this code on floating `stable` (`dtolnay/rust-toolchain@stable` everywhere, no rust-toolchain file; only `build-worker-musl.sh:14` pins 1.98.1), while the workers repo pins 1.97.1; a fresh lock could pull `msb_krun_devices` 0.1.39 | Compile under 1.97.1 first; seed the lock from iii |
| Each firmware update adds about 33 MB to the 364 MiB pack | Accepted [User]; never use LFS, and the header/SHA check catches pointer files |
| A new contract operation needs an engine release (fixed router id table) | Additive fields do not; plan operations with releases |
| The ide e2e registers mock `sandbox::fs::*` and runs standalone `iii -c` with `iii-sandbox` (`ide/tests/e2e/config.yaml:18`) | The mocks become `sandbox-mock::*` and the config binds `sandbox: {adapter: sandbox-mock, namespace: ide-e2e}`; the suite stays standalone (step 5) |
| The router forwards through `call_value_with_metadata_ns`, which starts a fresh invocation with no invocation id, session or timeout (`engine/src/engine/mod.rs:2536-2548`). A consumer's timeout or cancellation does not stop adapter-side work: create, long exec, fs streams (*inference*). An adapter disconnect mid-exec surfaces as `invocation_stopped`, and a retry can repeat side effects | Documented; not an S-code. [Builder: router ceiling and cancellation] |
| `_publish-engine-workers.yml` boots standalone `iii --config` to capture interfaces | Keep it working, or move it to Compose before standalone is removed |

### Left to the builder
- S5xx numbering, the 5.4 field names, and progress reporting for `prepare`.
- The token and host-roots mechanism (7.8), or replacing it with engine-issued tokens or RBAC.
- Whether the router checks the worker name of the `<adapter>::create` owner. A worker in the adapter's namespace could pre-claim the adapter's ids.
- Whether the router enforces a ceiling on forwarded calls and propagates cancellation to the adapter (risk above).
- Keeping git history (`git filter-repo`) or making a plain copy.
- Whether `--control-sock` is needed for sandbox exec. The comments disagree: `adapters.rs:262-266` vs `iii-init/src/supervisor.rs:54-60`.
- When to extract backend-neutral daemon logic (allowlist, caps, admission, `run`, request normalization) into a shared crate. Deferred to the Docker adapter.

## 13. Decisions log

| # | Decision | Who | Rationale |
|---|---|---|---|
| 1-3 | All VM code, crates and firmware (as-is) move to one workers-repo worker; the engine keeps only the interface and adapter choice; each adapter is its own worker (builtin now, Docker and iii-cloud later) | User | Engine free of libkrun; independent backends and releases |
| 4 | Compose-only; standalone `iii -c` out of scope | User | Deprecated |
| 5 | Compose installs the configured adapter and boots VMs through the contract | User | Bundle workers follow the backend |
| 6-7 | A migration step with clear errors; success = a bundle worker in a VM via the new worker, consumers unchanged | User | Old configs exist; consumers must not break |
| 8 | The router owns `sandbox::*` in `default` and forwards to `<adapter>::*` in any namespace | Designer (a) | No consumer change, no hijacking, project-namespace adapters |
| 9 | A contract crate in the iii repo on crates.io, with two families | Designer (b) | The engine cannot depend on the workers repo |
| 10 | `engine.sandbox.adapter`; adapter settings are the adapter's own config, read at startup | Designer (c) | Console-editable; applies on adapter restart |
| 11 | `iii-worker` and `iii-init` leave the engine release, `engine/install.sh` and `iii update` | Designer (d) | Follows from 1 |
| 12 | The doc lives at `docs/superpowers/specs/` in the workers repo | Designer (e) | |
| 13 | Compose calls the worker-VM family directly on its project's adapter | Designer | Per-project VMs; the router needs no caller identity |
| 14 | Adapter named `sandbox-builtin`; `engine.sandbox.adapter` is its function prefix and manifest name, never a container key | Designer | The router's prefix cannot drift from an operator's container key |
| 15 | `sandbox::*` is reserved in `default` only | Designer | Closes the shadowing hole; keeps the e2e mocks legal |
| 16 | The adapter is persisted as a generated container under the exact `compose::add` marker | Designer | Lock, `--frozen` and console config work unchanged |
| 17 | Lazy router handshake via `<adapter>::adapter::info` (major and name only); Compose's check runs at start, after the adapter is up; new codes S301-S304 and S501-S506 | Designer | `adapter::info` is live; no attach protocol; wire format kept |
| 18 | Build-time signing; no Intel Mac or Windows builds | Designer | Avoids cache churn; libkrun/libkrunfw ship no x86_64-darwin or Windows build (Intel Macs do have Hypervisor.framework) |
| 19 | Exec as root, `workdir` defaulting to `/`, image config ignored, guaranteed in v1 | Designer | Consumers depend on it; Docker can comply |
| S1 | A Compose-managed engine without `engine.sandbox` is bound to `sandbox-builtin` in the owner's namespace; S303 only on an engine no Compose file manages, whose operator must set `sandbox:` | Designer's call | Removes the 6.3/7.1 contradiction; ids are never hidden under Compose |
| S2 | Adapter package major = `CONTRACT_MAJOR`; Compose resolves `^<CONTRACT_MAJOR>`, compiled in, on add, up and update; no registry schema change in v1 | Designer's call | An older engine never auto-installs an incompatible adapter |
| S3 | VM guests keep inheriting the host environment | Designer's call | Today's behaviour; the env leaves the kernel cmdline (8.3) |
| S4 | The image prefetch (only in `iii project init --learn-iii`) is dropped with its dead code | Designer's call | The first `up` pulls; less code |
| S5 | `up` without `--frozen` writes the generated adapter container; `--frozen` refuses with a one-line fix | Designer's call | One-step start; a frozen file never changes |
| S6 | No shell into worker VMs in v1; `worker::exec` stays reserved | Designer's call | Not needed for acceptance |
| S7 | The router keeps the public worker name `iii-sandbox` | Designer's call | Registry package, skills and docs keep resolving |
| Q1 | One adapter per engine, in the binding namespace, serves every project's `sandbox::*` calls; an attached project installs nothing for those calls, which get S301 when the owner's adapter is down | User | Per-caller routing would need caller identity in built-in handlers (`function.rs:75-83`) |
| Q2 | Revised: consumers declare no sandbox need. Compose installs the adapter only for VM containers, `engine.sandbox` or `compose::add iii-sandbox`; other callers get S301 with the fix | User (ponytail review) | Less Compose code; the S301 message already says how to install it |
| Q3 | No musl at launch (gnu and `aarch64-apple-darwin` only); musl later | User | A product choice; the firmware already covers musl |
| Q4 | Compose drives bundle and local VMs through the `worker::*` family over the bus, not by exec'ing the adapter binary | User | Fits remote adapters such as iii-cloud |
| P1 | The v1 cuts listed in 2 | Designer, ponytail review | Nothing uses them yet; each can return additively |
| T1 | Plan triage decisions D1-D13: contract version, license and trims; no alpha publish; where the acceptance e2e runs; builtin stop; image workflow trigger; no musl scripts; P2a/P2b split; router limits; Compose need, removal and host-root rules; P3 and P5 scope | Designer | Listed in the plan index under `docs/superpowers/plans/`; they override the drafts and are marked [Designer, plan triage] in this spec |
