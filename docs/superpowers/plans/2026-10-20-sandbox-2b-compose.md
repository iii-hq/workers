# Sandbox Compose Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Compose binds the engine's sandbox router, installs the configured sandbox adapter when a project needs one, and boots `bundle` and `runtime.base_image` workers through the adapter's `worker::*` functions instead of exec'ing `iii-worker`.

**Architecture:** A new `crates/iii-compose/src/sandbox.rs` owns need detection, adapter identity, start ordering and the bus client for VM containers (`RemoteVm`). The daemon writes the adapter as a generated container through the existing add machinery (`upsert_container` and the lock), and the lifecycle supervises a VM as `Child::Vm`, which has the same `poll`/`wait`/`stop` as a process. The materialized `engine-config.yaml` always carries the S1 router binding.

**Tech Stack:** Rust 2024; `iii-sdk` (`trigger`, `helpers::create_channel`, `channels::ChannelReader`); `iii-sandbox-contract` (path dependency); serde_yaml; wiremock through `registry::TEST_REGISTRY` in tests; GitHub Actions with `/dev/kvm`.

**Spec:** docs/superpowers/specs/2026-10-20-sandbox-worker-design.md, sections 2, 5.4, 5.5, 5.7 (Compose part), 6.3 (Compose part), 6.4, 7.1-7.9, 10 (step 2), 11. Shared names: docs/superpowers/plans/2026-10-20-sandbox-shared-interfaces.md

**Repo and base:** iii repo. Citations are at `a1d08d8b0`; re-locate by symbol where P2a moved lines. Branch `sandbox/p2b-compose` off P2a's branch `sandbox/p2a-router`, rebased onto main when P2a merges. P2a merges only once this PR is approved and `sandbox-builtin` 1.0.0 resolves for the three targets, and this PR merges right after it, so P2a, P2b and P3 merge back to back (index row 3). Prerequisites: P0 (`crates/iii-sandbox-contract` in the workspace); P2a's branch (the engine accepts a top-level `sandbox:` key, and `"iii-sandbox"` is gone from `CONFIGURABLE_ENGINE_WORKERS`, `crates/iii-compose/src/config.rs:58-64`, together with the Compose fixtures that declared it, `managed_engine.rs:908-910, 949-954` and `tests/config_validation.rs:121-149`); `sandbox-builtin` 1.0.0 published to the registry by P1 (Task 8 pulls it). P3 follows this PR.

## Global Constraints

- Consumers keep calling `sandbox::*` in namespace `default`, unchanged (2, goal 4); no `sandbox::*` request, response or S-code wire change (2, anti-goal).
- Compose never execs an adapter binary to boot VMs (2, anti-goal); it calls `<adapter>::worker::*` directly in its project namespace (5.2, Q4).
- A Compose-managed engine always gets a binding: the section's values, or `{ adapter: sandbox-builtin, namespace: <owner namespace> }` when `engine.sandbox` is absent (6.3, S1). An External-mode engine gets none (6.3).
- Changing `engine.sandbox` needs a Compose restart (`EngineRestartRequired`) (6.3).
- `SANDBOX_CONFIG_MOVED` text is spec 6.4 verbatim; it prints the replacement YAML; there is no `compose::migrate` (7.9).
- The adapter Compose writes is a generated container under exactly `# added by compose::add` (`edit.rs:115`); the reason (`sandbox adapter for: my-bundle`) is printed, never written (7.2). The `SANDBOX_CONFIG_MOVED` replacement carries no marker: it is operator-declared, as the template (D13), so Compose never removes it or its `config_override` (7.9).
- Compose resolves `^<CONTRACT_MAJOR>`, compiled in from the contract crate, and writes the exact version it resolved (7.2, S2).
- The adapter is recognized by its manifest name, never by its container key; `AdapterInfo.adapter` must equal the prefix, else `SANDBOX_ADAPTER_MISMATCH`; the operator's declaration wins (7.2).
- Outcomes: supported host and no adapter, write and install (`up`, `add`, `update`, `iii compose build`); no build for the host, including the registry's `/resolve` `unsupported_platform` refusal with no fallback, `UNSUPPORTED_PLATFORM`; `--frozen` and no adapter, `SANDBOX_ADAPTER_NOT_DECLARED` with the one-line fix (7.2, S5, D11). Only `update` and `remove` delete the generated adapter, when their edit makes `needs(file)` empty (D11): they pass `allow_remove`; `up`, `build` and `add` never do.
- Start check before any `worker::prepare`: `adapter::info`, major (S302) then name; a failed check fails each VM container's start like a failed spawn (5.7). `function_not_found` is retried for up to 10 s (7.4).
- `worker::prepare` keeps the 20-minute budget; `worker::status` is polled every second; a bus error is `Outcome::Unknown`, which never restarts or cascades; S504 counts as an exit with no code (7.4). S502 from `start` and S503 from `status` or `stop` name the live `{vm_id, instance}` in their `fix` (P1): Compose stops that instance and retries the call once; an S503 on the retry is an exit with no code. `stop` never takes a reply that is not a parsed worker reply as stopped: it retries for up to `STOP_RETRY` (60 s), then warns and returns (Task 6).
- `state_key = hash(<compose-dir>/.iii/compose/<ns>, key)`; `source.package` is the manifest name; the guest env is today's `spawn_plan` env; a restart is `stop` then `start` with the same `state_key`; `rm` calls `worker::destroy` (7.4).
- `ChildRecord.pid` and `.birth` become `Option` with `#[serde(default)]`, so existing state files load (7.4).
- Token: per project, kept in the 0600 state, passed as `III_SANDBOX_WORKER_TOKEN`; host roots: the compose dir and the package cache (7.8). The adapter container also gets `III_DISABLE_TRACE_PAYLOADS=1`, so its SDK records no handler input (`auth`, the guest `env`, `channel.access_key`) as a span event.
- The worker-VM guard assumes bus clients are trusted: a client that claims `<adapter>::adapter::info` or `<adapter>::worker::*` in the project namespace before the adapter does receives the token and the guest env. No protocol change in v1; this is the known limit spec 12 records for the router.
- Not in v1 (2): consumer soft needs, `SANDBOX_ADAPTER_IS_A_VM` (an adapter that is a VM container is a `DEPENDENCY_CYCLE`), capability fields, `worker::{exec, sync, wait, list, renew}`, leases, archive sources, hot-applied adapter config. Cut by D3/D11: `WorkerStartRequest.resources`, `WorkerSource::Local.sync`, a `Need::Added`, a lock-graph entry or GC for the adapter, a persisted log cursor (`VmRecord.log_cursor`).

## Review Focus

1. An engine restart or a dropped bus while VMs run: a reasonable person expects the VMs to keep running and Compose not to restart or cascade them; an adapter restart that lost the VM (S504) should apply the restart policy. Pinned by Task 6 `bus_errors_are_unknown_and_s504_is_an_exit_without_code`.
2. A 0.24 file with `engine.workers.iii-sandbox` (also `iii-sandbox#second` and an empty `{}` block): the printed replacement YAML, pasted as-is, should parse and keep every old key under `config_override`. Pinned by Task 2 `replacement_yaml_parses_and_keeps_the_old_block`.
3. An operator who already declares the adapter (another key, a pinned 1.x, or a `path://` checkout whose manifest is `sandbox-builtin`): Compose must not add a second adapter, refresh theirs, or remove it. Pinned by Task 4 `an_operator_adapter_is_never_added_twice_refreshed_or_removed`.
4. CI running `up --frozen` on a project with a bundle or base-image worker whose file never went through `up` or `iii compose build` (both write the adapter, D11): it should refuse before anything starts, name the fix, and leave the file and lock byte-identical. Pinned by Task 4 `frozen_up_refuses_an_undeclared_adapter_and_writes_nothing`.
5. The worker token is a secret: it must reach only the adapter container, never another container or a VM guest (whose env inherits the host env, S3), even when the operator's shell exports `III_SANDBOX_WORKER_TOKEN`. Pinned by Task 5 `sandbox_guards_reach_only_the_adapter`.

---

### Task 1: `engine.sandbox` and the S1 router binding

**Files:**
- Modify: `crates/iii-compose/Cargo.toml:26` (add `iii-sandbox-contract = { path = "../iii-sandbox-contract" }` after `iii-sdk`), `Cargo.lock`
- Modify: `crates/iii-compose/src/config.rs:328-333` (`EngineSpec`), `:473-500` (`validate_engine` copies `raw.sandbox`), `:1017-1027` (`RawEngineSpec`)
- Modify: `crates/iii-compose/src/managed_engine.rs:72` (pass `daemon_namespace`), `:243-325`, the test calls at `:932, :971, :1045, :1068, :1314` (pass `"app"`)
- Test: `crates/iii-compose/tests/config_validation.rs`, `crates/iii-compose/src/managed_engine.rs`, `crates/iii-compose/tests/project_cache.rs`

**Interfaces:**
- Consumes: P0 `DEFAULT_ADAPTER`; P2a `SandboxRouterConfig { adapter, namespace }` read from the top-level `sandbox:` key.
- Produces: `config::SandboxSpec` and `EngineSpec.sandbox: Option<SandboxSpec>` (registry §5, verbatim); `fn materialize_engine_config(spec: &EngineSpec, namespace_dir: &Path, owner_namespace: &str) -> Result<PathBuf>` (new parameter, not in registry).

- [ ] **Step 1: Write the failing tests**

```rust
// tests/config_validation.rs (use iii_compose::config::SandboxSpec)
#[test] fn engine_sandbox_section_defaults_to_sandbox_builtin() {
    let engine = parse("engine:\n  sandbox: {}\ncontainers: {}\n").unwrap().engine.unwrap();
    assert_eq!(engine.sandbox, Some(SandboxSpec { adapter: "sandbox-builtin".into(), namespace: None }));
    let engine = parse("engine:\n  sandbox: {adapter: sandbox-docker, namespace: shared}\ncontainers: {}\n").unwrap().engine.unwrap();
    assert_eq!(engine.sandbox.unwrap().namespace.as_deref(), Some("shared"));
    assert_eq!(parse("engine: {workers: {}}\ncontainers: {}\n").unwrap().engine.unwrap().sandbox, None);
}
#[test] fn engine_sandbox_rejects_unknown_keys() {
    assert_eq!(code("engine:\n  sandbox: {adaptor: x}\ncontainers: {}\n"), "INVALID_COMPOSE_FILE");
}
// managed_engine.rs tests
#[test] fn materialized_config_binds_sandbox_builtin_in_the_owner_namespace() {
    // engine_spec() has no sandbox section
    // doc["sandbox"] == yaml("{adapter: sandbox-builtin, namespace: orders}") for materialize_engine_config(&spec, dir, "orders")
}
#[test] fn materialized_config_uses_the_engine_sandbox_section() {
    // sandbox {adapter: sandbox-docker} -> {adapter: sandbox-docker, namespace: orders}
    // sandbox {adapter: sandbox-docker, namespace: shared} -> {adapter: sandbox-docker, namespace: shared}
}
// tests/project_cache.rs, modelled on managed_daemon_requires_restart_when_its_owner_engine_section_changes (:202-231)
#[tokio::test] async fn changing_engine_sandbox_requires_an_engine_restart() {
    // initial "engine: { workers: {} }\n...", rewritten with "engine:\n  workers: {}\n  sandbox: {adapter: sandbox-docker}\n..."
    // assert_eq!(err.code(), "ENGINE_RESTART_REQUIRED");
}
```

- [ ] **Step 2: Run them to verify they fail**
Run: `cargo test -p iii-compose -- engine_sandbox materialized_config changing_engine_sandbox` (no `--locked` here: this run also refreshes Cargo.lock for the new path dependency)
Expected: compile errors `cannot find type SandboxSpec` and `this function takes 2 arguments but 3 arguments were supplied`.

- [ ] **Step 3: Implement `SandboxSpec` in `config.rs` and the binding in `materialize_engine_config`**
`RawEngineSpec` gets `#[serde(default)] sandbox: Option<SandboxSpec>`; `validate_engine` copies it; no adapter-name check in Compose (the engine's `INVALID_SANDBOX_CONFIG` is the gate, registry §5). `MaterializedEngineConfig` gains an always-serialized `sandbox: { adapter: &str, namespace: &str }` filled from `spec.sandbox`, else `DEFAULT_ADAPTER` and `owner_namespace`. `EngineSpec` equality already drives `EngineRestartRequired` (`daemon.rs:198-225`); nothing else changes.

- [ ] **Step 4: Run the tests to verify they pass**
Run: `cargo test --locked -p iii-compose`
Expected: `test result: ok` for every binary.

- [ ] **Step 5: Commit**
`git add Cargo.lock crates/iii-compose/Cargo.toml crates/iii-compose/src/config.rs crates/iii-compose/src/managed_engine.rs crates/iii-compose/tests/config_validation.rs crates/iii-compose/tests/project_cache.rs`
`git commit -m "feat(compose): bind the engine sandbox router from engine.sandbox"`

---

### Task 2: Migration errors for the old sandbox block

**Files:**
- Modify: `crates/iii-compose/src/config.rs:473-500` (an `engine_worker_type(&name) == "iii-sandbox"` arm before the injected and configurable checks), plus `fn sandbox_replacement(old: &serde_yaml::Value) -> String`
- Modify: `crates/iii-compose/src/error.rs:46-51` (text), new variant beside it, `code()` (`:558`)
- Modify: `engine/worker-compose.yaml:19-26` (the commented `iii-sandbox` block becomes a commented `sandbox: {adapter: sandbox-builtin}` at `engine:` level, D9)
- Test: `crates/iii-compose/tests/config_validation.rs`

**Interfaces:**
- Consumes: P0 `CONTRACT_MAJOR`.
- Produces: `ComposeError::SandboxConfigMoved { replacement: String }`, code `SANDBOX_CONFIG_MOVED` (registry §5); `config::sandbox_replacement` (not in registry).

- [ ] **Step 1: Write the failing tests**

```rust
const MOVED: &str = "engine.workers.iii-sandbox was removed in iii 0.25: sandboxes now run in the sandbox-builtin worker, which Compose installs for you. Replace the block with `engine.sandbox.adapter: sandbox-builtin` (optional, the default) and move image_allowlist/custom_images/default_* to `containers.sandbox-builtin.config_override`. Guide: https://iii.dev/docs/upgrading/sandbox-worker";
#[test] fn the_0_24_sandbox_block_fails_with_its_replacement() {
    let error = parse("engine:\n  workers:\n    iii-sandbox:\n      image_allowlist: [python]\n      default_cpus: 2\ncontainers: {}\n").unwrap_err();
    assert_eq!(error.code(), "SANDBOX_CONFIG_MOVED");
    assert_eq!(error.to_string(), format!("{MOVED}\n\ncontainers:\n  sandbox-builtin:\n    worker: package://sandbox-builtin\n    version: '^1'\n    config_override:\n      image_allowlist:\n      - python\n      default_cpus: 2\n"));
}
#[test] fn replacement_yaml_parses_and_keeps_the_old_block() { // Review Focus 2
    for block in ["iii-sandbox:\n      image_allowlist: [python, node]\n      custom_images: {r: {image: 'docker.io/r:1'}}", "iii-sandbox#second: {}"] {
        let Err(ComposeError::SandboxConfigMoved { replacement }) = parse(&format!("engine:\n  workers:\n    {block}\ncontainers: {{}}\n")) else { panic!("{block}") };
        let file = ComposeFile::parse(&replacement, "/tmp/worker-compose.yaml").unwrap();
        // config_override of "sandbox-builtin" equals the old block's mapping; version == Some("^1")
    }
}
#[test] fn unsupported_engine_worker_points_to_engine_sandbox() {
    let message = parse("engine:\n  workers:\n    queue: {}\ncontainers: {}\n").unwrap_err().to_string();
    assert!(!message.contains("iii-sandbox") && message.contains("engine.sandbox"), "{message}");
}
```

- [ ] **Step 2: Run them to verify they fail**
Run: `cargo test --locked -p iii-compose --test config_validation -- sandbox engine_worker replacement_yaml`
Expected: compile error `no variant named SandboxConfigMoved`.

- [ ] **Step 3: Implement the arm, the variant and `sandbox_replacement`**
`#[error("{MOVED text}\n\n{replacement}")]`. `sandbox_replacement` writes `containers:`, `sandbox-builtin:` with no marker (operator-declared, so never removed with its `config_override`, D13), `worker: package://sandbox-builtin`, `version: '^{CONTRACT_MAJOR}'` (offline, so not an exact version, registry §5) and `config_override:` followed by `serde_yaml::to_string(old)` indented six spaces. New `UnsupportedEngineWorker` text: `engine worker '{worker}' is not configurable inside worker-compose.yaml. Move project workers under containers; engine.workers accepts configuration, iii-worker-manager, iii-http-functions and iii-stream. Sandboxes are configured under engine.sandbox`. `EngineWorkerIsBuiltin` (`daemon.rs:2178-2191`) keeps its text: it names no worker, and Task 4 rewrites `compose::add iii-sandbox` before it can fire.

- [ ] **Step 4: Run the tests to verify they pass**
Run: `cargo test --locked -p iii-compose`
Expected: `test result: ok` for every binary.

- [ ] **Step 5: Commit**
`git add crates/iii-compose/src/config.rs crates/iii-compose/src/error.rs crates/iii-compose/tests/config_validation.rs engine/worker-compose.yaml`
`git commit -m "feat(compose): explain the moved sandbox config with its replacement YAML"`

---

### Task 3: Need detection and start ordering

**Files:**
- Create: `crates/iii-compose/src/sandbox.rs`
- Modify: `crates/iii-compose/src/lib.rs:48` (`pub mod sandbox;` after `mod restart;`)
- Modify: `crates/iii-compose/src/manifest.rs:51-55, 88-91, 297-308` (`name`, read as a YAML value; a non-string name stays ignored)
- Modify: `crates/iii-compose/src/project.rs:193-241` (`open_with_shutdown` wires the file before storing it), `:251-266` (`commit_prepared_packages` inserts the adapter container when `resolved` declares it and the held file does not, which happens when `up` just wrote it into an already loaded project, then wires)
- Modify: `crates/iii-compose/src/daemon.rs:1094` (add) and `:1557` (remove): `sandbox::wire_adapter(&mut current)?` before `persist_mutation`, so a cycle fails before anything is written
- Test: `crates/iii-compose/src/sandbox.rs` (tests module)

**Interfaces:**
- Consumes: Task 1 `EngineSpec.sandbox`; P0 `DEFAULT_ADAPTER`; `manifest::read_manifest`; `dag::validate_dependencies`.
- Produces (registry §5): `#[derive(Debug, Clone, PartialEq, Eq)] pub enum Need { VmBundle(String), VmLocal(String), Explicit }`, `pub fn needs(file: &ComposeFile) -> Vec<Need>`, `pub fn wire(file: &mut ComposeFile, adapter_key: &str) -> Result<()>`, `pub fn state_key(state_dir: &Path, key: &str) -> String`. Not in registry: `pub fn adapter_name(file: &ComposeFile) -> &str` (`engine.sandbox.adapter`, else `DEFAULT_ADAPTER`; an attached project has no `engine:`, so it gets `DEFAULT_ADAPTER`, D11), `pub fn adapter_key(file: &ComposeFile, adapter: &str) -> Result<Option<String>>` (first container whose resolved package name, else reference package name, or `path://` manifest `name` equals `adapter`), `pub(crate) fn wire_adapter(file: &mut ComposeFile) -> Result<()>` (`adapter_key` then `wire`; no adapter, no change), `Manifest.name: Option<String>`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn needs_lists_bundles_base_images_and_the_section() {
    // tmp/local/iii.worker.yaml = "runtime: {base_image: python}\nscripts: {start: run}\n"
    // file: app = package with resolved_package.kind "bundle"; bin = package with kind "binary"; local = path://./local; engine: {sandbox: {}}
    assert_eq!(needs(&file), vec![Need::VmBundle("app".into()), Need::VmLocal("local".into()), Need::Explicit]);
}
#[test] fn the_adapter_is_found_by_manifest_name_not_by_key() {
    // "sb: {worker: package://sandbox-builtin, version: 1.0.0}" -> Some("sb")
    // "sandbox-builtin: {worker: path://./other}" with manifest "name: other" -> None
    // "dev: {worker: path://./dev}" with manifest "name: sandbox-builtin" -> Some("dev")
}
#[test] fn wire_starts_every_vm_after_the_adapter_in_memory() {
    wire(&mut file, "sb").unwrap();
    assert!(file.containers["app"].start_after.contains(&"sb".into()) && file.containers["local"].start_after.contains(&"sb".into()));
    assert!(!file.containers["bin"].start_after.contains(&"sb".into()));
    wire(&mut file, "sb").unwrap(); // idempotent: "sb" appears once
}
#[test] fn an_adapter_that_is_a_vm_container_is_a_dependency_cycle() {
    // path adapter whose manifest is "name: sandbox-builtin\nruntime: {base_image: python}\nscripts: {start: x}\n"
    assert_eq!(wire_adapter(&mut file).unwrap_err().code(), "DEPENDENCY_CYCLE");
}
#[test] fn state_key_is_stable_per_state_dir_and_container() {
    let key = state_key(Path::new("/srv/app/.iii/compose/shop"), "web");
    assert_eq!(key.len(), 64); assert!(key.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(key, state_key(Path::new("/srv/app/.iii/compose/shop"), "web"));
    assert_ne!(key, state_key(Path::new("/srv/app/.iii/compose/shop"), "api"));
    assert_ne!(key, state_key(Path::new("/srv/other/.iii/compose/shop"), "web"));
}
```

- [ ] **Step 2: Run them to verify they fail**
Run: `cargo test --locked -p iii-compose --lib sandbox::`
Expected: compile errors `cannot find function needs in this scope` (and the other Task 3 names).

- [ ] **Step 3: Implement `needs`, `adapter_name`, `adapter_key`, `wire`, `wire_adapter`, `state_key`**
`VmBundle` comes from `resolved_package.kind == "bundle"` (attached from the lock, `lockfile.rs:168-188`, or by `prepare`); `VmLocal` from `read_manifest(dir)` with a `base_image` (manifest errors are left to `validate_offline`); `Explicit` when `engine.sandbox` is set. `state_key` is hex SHA-256 over the state dir bytes, `0x00`, then the key (registry §5). The four call sites are spec 7.3's callers that return `Result` (D11).

- [ ] **Step 4: Run the tests to verify they pass**
Run: `cargo test --locked -p iii-compose`
Expected: `test result: ok` for every binary.

- [ ] **Step 5: Commit**
`git add crates/iii-compose/src/sandbox.rs crates/iii-compose/src/lib.rs crates/iii-compose/src/manifest.rs crates/iii-compose/src/project.rs crates/iii-compose/src/daemon.rs`
`git commit -m "feat(compose): detect sandbox needs and start VM containers after the adapter"`

---

### Task 4: Install the adapter as a generated container

**Files:**
- Modify: `crates/iii-compose/src/sandbox.rs` (`adapter_request`, `AdapterChange`, `adapter_change`, `apply_adapter_change`, `is_iii_sandbox`)
- Modify: `crates/iii-compose/src/registry.rs:911-920` (a refusal whose `error.code` is `unsupported_platform`, with no fallback left, returns `UnsupportedPlatform { container, name, version: version_range, target: candidates[index], available: <its list joined by ", "> }`), `:1874-1886` (assert `error.code() == "UNSUPPORTED_PLATFORM"`)
- Modify: `crates/iii-compose/src/error.rs` (`SandboxAdapterNotDeclared` and its `code()` arm)
- Modify: `crates/iii-compose/src/daemon.rs`: `:717-737` (up); `:953-957` (add: rewrite `iii-sandbox` requests in `add_configured_scoped`, before planning, D11); `:1053-1094` (add); `:384-428` (`workers_to_update` skips the generated adapter); `:1365-1405` (update); `:1526-1557` (remove); `:2079` (`persist_mutation` becomes `pub(crate)`)
- Modify: `crates/iii-compose/src/build.rs:64-80` (`build_with_mode`, not frozen: the sandbox step after `prepare`, then `persist_mutation` instead of `prepared.write_if_changed()`, so `build` then `up --frozen` works, D11)
- Test: `crates/iii-compose/src/sandbox.rs`, `crates/iii-compose/src/daemon.rs`, `crates/iii-compose/tests/project_cache.rs`

**Interfaces:**
- Consumes: Task 3 `needs`, `adapter_name`, `adapter_key`; P0 `CONTRACT_MAJOR`; existing `registry::resolve_node` (`registry.rs:527`), `edit::{upsert_container, remove_container, is_generated_container, parse_worker}`, `lockfile::prepare_metadata_with_versions`, `PreparedLock::install`, `persist_mutation` (`daemon.rs:2079`).
- Produces: `ComposeError::SandboxAdapterNotDeclared { adapter, needed_by }`, code `SANDBOX_ADAPTER_NOT_DECLARED`, registry §5 text; not in registry: `pub(crate) async fn adapter_request(adapter: &str) -> Result<NewContainer>` (`package://<adapter>`, resolved with range `^{CONTRACT_MAJOR}`, pinned to the exact `Node.version`); `pub(crate) enum AdapterChange { Upsert(NewContainer), Remove(String) }`; `pub(crate) async fn adapter_change(text: &str, file: &ComposeFile, refresh: bool, allow_remove: bool) -> Result<Option<AdapterChange>>`; `pub(crate) fn is_iii_sandbox(worker: &NewContainer) -> bool`; `pub(crate) async fn apply_adapter_change(path: &Path, edited: &mut String, current: &mut ComposeFile, prepared: &mut PreparedLock, force: &mut BTreeSet<String>, selected: &mut BTreeMap<String, String>, refresh: bool, allow_remove: bool) -> Result<Option<AdapterChange>>` (passes both flags to `adapter_change`).

`adapter_change` decides, with `generated` = `edit::is_generated_container(text, key)`:

| `needs(file)` | adapter container | result |
|---|---|---|
| non-empty | none | `Upsert(adapter_request)` |
| non-empty | generated, `refresh`, resolved version differs | `Upsert` under its key |
| empty | generated, `allow_remove` | `Remove(key)`. Only update and remove pass `allow_remove = !sandbox::needs(&file_before_edit).is_empty()`, computed after `lockfile::attach`, so the list "becomes empty" (D11); up, build and add pass `false`, so an adapter added by `compose::add iii-sandbox` for `sandbox::*` callers stays |
| anything else | | `None` |

- [ ] **Step 1: Write the failing tests**

```rust
// sandbox.rs, wiremock as registry.rs:1832-1888
#[tokio::test] async fn the_adapter_resolves_within_the_contract_major_and_pins_the_exact_release() {
    // POST /resolve with body_partial_json({"worker": "sandbox-builtin", "version": "^1"}) -> graph [{name: "sandbox-builtin", version: "1.2.3", type: "binary", binaries: {<host_targets()[0]>: {sha256, url}}}]
    let request = registry::TEST_REGISTRY.scope(server.uri(), adapter_request("sandbox-builtin")).await.unwrap();
    assert_eq!(request.source, Source::Package { reference: "api.workers.iii.dev/sandbox-builtin".into(), version: Some("1.2.3".into()) });
    let edit::Outcome::Added(text) = edit::upsert_container("containers: {}\n", &request).unwrap() else { panic!() };
    assert!(edit::is_generated_container(&text, "sandbox-builtin").unwrap());
}
#[tokio::test] async fn a_generated_adapter_goes_with_its_last_vm() {
    // generated sandbox-builtin, no VM container: adapter_change(.., false, true) -> Some(AdapterChange::Remove("sandbox-builtin"))
    // same file with allow_remove false (up, build, add) -> None, so up leaves the file byte-identical
}
#[tokio::test] async fn an_operator_adapter_is_never_added_twice_refreshed_or_removed() { // Review Focus 3
    // hand-written "sb: {worker: package://sandbox-builtin, version: 1.0.0}" with a bundle need -> adapter_change(.., true, true) == None
    // same file with no VM need -> None; hand-written path://./dev named sandbox-builtin with a bundle need -> None
}
#[test] fn compose_add_iii_sandbox_names_the_adapter() { assert!(is_iii_sandbox(&edit::parse_worker("iii-sandbox").unwrap()) && !is_iii_sandbox(&edit::parse_worker("./iii-sandbox").unwrap())); }
// daemon.rs tests
#[test] fn update_all_leaves_the_generated_adapter_to_the_sandbox_step() {
    // generated sandbox-builtin plus a path VM container; workers_to_update(&compose, &[], &generated, &BTreeMap::new()) names no "sandbox-builtin"
}
#[tokio::test] async fn a_host_without_an_adapter_build_fails_unsupported_platform() {
    // "containers:\n  vm:\n    worker: path://./vm\n", vm/iii.worker.yaml "runtime: {base_image: python}\nscripts: {start: run}\n", EnginePolicy::External
    // /resolve answers 422 {"error": {"code": "unsupported_platform", "available": ["riscv64gc-unknown-linux-gnu"]}}
    // TEST_REGISTRY.scope(uri, daemon.up(..)) -> err.code() == "UNSUPPORTED_PLATFORM", message names "sandbox-builtin"; file bytes unchanged
}
#[tokio::test] async fn compose_add_iii_sandbox_keeps_the_adapter_without_a_vm() {
    // "containers: {}\n", EnginePolicy::External; /resolve for sandbox-builtin "^1" answers 1.2.3 (as the first sandbox.rs test)
    // TEST_REGISTRY.scope(uri, daemon.add(Some(&file), &["iii-sandbox".into()], "add".into())); whatever the start's outcome,
    // the file holds sandbox-builtin at version 1.2.3 and edit::is_generated_container(&text, "sandbox-builtin") is true
}
// tests/project_cache.rs
#[tokio::test] async fn frozen_up_refuses_an_undeclared_adapter_and_writes_nothing() { // Review Focus 4
    // path://./vm with manifest "runtime: {base_image: python}\nscripts: {start: run}\n"; lock "version: 1\ncontainers: {}\n"
    let err = daemon.up_frozen(Some(&file), None, "frozen".into()).await.unwrap_err();
    assert_eq!(err.code(), "SANDBOX_ADAPTER_NOT_DECLARED");
    assert_eq!(err.to_string(), "vm needs the sandbox adapter 'sandbox-builtin', and --frozen does not edit the file. Run up once without --frozen, or: iii trigger compose::add worker=iii-sandbox");
    // file and lock bytes are what the test wrote
}
```

- [ ] **Step 2: Run them to verify they fail**
Run: `cargo test --locked -p iii-compose -- adapter frozen_up update_all a_host_without`
Expected: compile errors `cannot find function adapter_request` and `no variant named SandboxAdapterNotDeclared`.

- [ ] **Step 3: Implement the sandbox step and its five call sites**
`apply_adapter_change` edits `edited` with `upsert_container`/`remove_container`, re-parses `current`, adds the adapter key and its exact version to `force`/`selected`, and re-runs `prepare_metadata_with_versions` with `selected` plus every version the first pass chose, so no other entry resolves differently. Each caller runs it right after its first prepare and before its `replace_graph` loop and Task 3's `wire_adapter`, then installs and persists as today.
- up (`prepare_start_project_scoped`): read the text; after `prepare`/`prepare_frozen`, if frozen and `needs` is non-empty and `adapter_key` is `None`, return `SandboxAdapterNotDeclared { adapter, needed_by: <first VM key, else "engine.sandbox"> }` before resolving the adapter; otherwise `refresh = false`, `allow_remove = false`, `prepared.install`, `persist_mutation` when the text changed, and print `{adapter}: sandbox adapter for: {vm keys}` with `report::daemon_line`.
- build (`build_with_mode`, not frozen): as up, without the frozen branch or the line.
- add: each `is_iii_sandbox` request becomes `adapter_request(adapter_name(&file))` for `file = ComposeFile::load(path)?`, or the declared adapter's own declaration when `adapter_key` finds one, so the reply's `details.container` is the adapter key (P5). After `prepare_with_versions`: `refresh = false`, `allow_remove = false`, then `prepared.install`.
- update: `workers_to_update` skips the adapter key when it is in `generated`; after `prepare_metadata_with_versions`: `refresh = true`, `allow_remove` = `needs` of the file before the edit is non-empty, before the existing install.
- remove: after `prepare_metadata`: `refresh = false`, `allow_remove` as update; a `Remove(key)` is appended to `removal_order` so `reconcile_removals` stops it after its VMs. Needs, not lock graphs, drive the adapter's removal (D11), which also covers `path://` VM roots.

- [ ] **Step 4: Run the tests to verify they pass**
Run: `cargo test --locked -p iii-compose`
Expected: `test result: ok` for every binary.

- [ ] **Step 5: Commit**
`git add crates/iii-compose/src/sandbox.rs crates/iii-compose/src/registry.rs crates/iii-compose/src/error.rs crates/iii-compose/src/daemon.rs crates/iii-compose/src/build.rs crates/iii-compose/tests/project_cache.rs`
`git commit -m "feat(compose): install the sandbox adapter as a generated container"`

---

### Task 5: Adapter guards: worker token and host roots

**Files:**
- Modify: `crates/iii-compose/src/state.rs:110-122, 154-161` (`sandbox_token`)
- Modify: `crates/iii-compose/src/project.rs:46-66` (field `sandbox_token: String`), `:193-241` (`state.sandbox_token.get_or_insert_with(|| uuid::Uuid::new_v4().simple().to_string())`, saved by `reconcile_recovered`'s save), the six `LifecycleCtx` literals (`:903, :958, :1048, :1083, :1142, :1185`)
- Modify: `crates/iii-compose/src/lifecycle.rs:99-117` (`pub sandbox_token: &'a str`), `:1075-1076` (`let mut user_env`; extended with `adapter_env` when `adapter_key(ctx.file, adapter_name(ctx.file))?` is this container's `key`)
- Modify: `crates/iii-compose/src/spawn.rs:24-57` (`RESERVED_ENV: [&str; 9]` plus a doc bullet), `:195-204` (drop inherited reserved names on every platform before `user_env` is overlaid; today only on windows)
- Modify: `crates/iii-compose/src/sandbox.rs` (`adapter_env`)
- Test: `crates/iii-compose/src/spawn.rs`, `crates/iii-compose/src/sandbox.rs` (old state loading is Task 6's `a_0_24_state_file_still_loads`)

**Interfaces:**
- Consumes: Task 3 `adapter_name`, `adapter_key`.
- Produces: `DaemonState.sandbox_token` (registry §5); env `III_SANDBOX_WORKER_TOKEN` and `III_SANDBOX_HOST_ROOTS` on the adapter container only (registry §5, P1 reads them); not in registry: `pub(crate) fn adapter_env(token: &str, compose_file: &Path, package_cache: &Path) -> BTreeMap<String, String>` (roots = `std::env::join_paths([compose dir, package cache])`; also `III_DISABLE_TRACE_PAYLOADS=1`, so iii-sdk records no handler input as the `iii.invocation.input` span event; overlaid on `user_env`, so an operator value cannot undo it), `LifecycleCtx.sandbox_token`.

- [ ] **Step 1: Write the failing tests**

```rust
// sandbox.rs
#[test] fn adapter_env_carries_the_token_and_both_roots() {
    let env = adapter_env("t0k", Path::new("/srv/app/worker-compose.yaml"), Path::new("/home/u/.iii/compose/packages"));
    assert_eq!(env["III_SANDBOX_WORKER_TOKEN"], "t0k");
    assert_eq!(env["III_DISABLE_TRACE_PAYLOADS"], "1"); // keeps auth, env and channel.access_key out of the trace store
    assert_eq!(std::env::split_paths(&env["III_SANDBOX_HOST_ROOTS"]).collect::<Vec<_>>(),
               vec![PathBuf::from("/srv/app"), PathBuf::from("/home/u/.iii/compose/packages")]);
}
// spawn.rs, Review Focus 5
#[test] fn sandbox_guards_reach_only_the_adapter() {
    assert!(is_reserved_env("III_SANDBOX_WORKER_TOKEN") && is_reserved_env("III_SANDBOX_HOST_ROOTS"));
    let host = env_of(&[("III_SANDBOX_WORKER_TOKEN", "leak"), ("III_SANDBOX_HOST_ROOTS", "/")]);
    // spawn_plan_with_env for StartSpec::Shell and for StartSpec::Vm with empty user_env: neither key is in plan.env
    // with user_env = adapter_env("t0k", ..): plan.env["III_SANDBOX_WORKER_TOKEN"] == "t0k"
}
```

- [ ] **Step 2: Run them to verify they fail**
Run: `cargo test --locked -p iii-compose --lib -- sandbox_guards adapter_env`
Expected: compile error `cannot find function adapter_env`.

- [ ] **Step 3: Implement the token, the reserved names and `adapter_env`**
`DaemonState.sandbox_token` is `#[serde(default, skip_serializing_if = "Option::is_none")]` (registry §5); `StateStore::save` already writes mode 0600 (`state.rs:304-308`).

- [ ] **Step 4: Run the tests to verify they pass**
Run: `cargo test --locked -p iii-compose`
Expected: `test result: ok` for every binary.

- [ ] **Step 5: Commit**
`git add crates/iii-compose/src/state.rs crates/iii-compose/src/project.rs crates/iii-compose/src/lifecycle.rs crates/iii-compose/src/spawn.rs crates/iii-compose/src/sandbox.rs`
`git commit -m "feat(compose): hand the sandbox adapter its worker token and host roots"`

---

### Task 6: Supervise a child that can be a remote VM

**Files:**
- Modify: `crates/iii-compose/src/process/mod.rs:70-75` (`Unknown`), `crates/iii-compose/src/process/unix.rs:256-262` (`Supervised::stop`'s exhaustive `match self.poll()`: `Unknown` cannot happen for a local process; treat it like `Running`)
- Modify: `crates/iii-compose/src/logs.rs` (`LogSink`, `LogStore::sink`)
- Modify: `crates/iii-compose/src/lifecycle.rs:121-129` (`child: Child`), `:149` (`Child`, `Children`), `:963-968` and `:1195` (return a `Child`, record with `from_child`), `:1520-1524` (`is_running`: anything but `Exited`)
- Modify: `crates/iii-compose/src/state.rs:82-108` (`pid`, `birth` optional, `vm`, `VmRecord`, `from_child`; `new` and `from_supervised` keep their signatures for `tests/crash_recovery.rs` and `tests/orphans.rs`), `:202-214` (`reconcile`: no pid is `Gone`)
- Modify: `crates/iii-compose/src/engine.rs:334-376` (`child: &Child`)
- Modify: `crates/iii-compose/src/project.rs:298` (`Running | Unknown`), `:385-438` (reap acts only on `Exited`), `:740-846` (process records destructure `(Some(pid), Some(birth))`), `:1209-1251` (`Running | Unknown` is ready; `pid` from `record.pid`), `:1298-1320` (`Child::is_adopted`), `:1412` (test: `inner.children.insert("worker".into(), Child::Process(child))`)
- Modify: `crates/iii-compose/src/sandbox.rs` (`RemoteVm`, `outcome_of`, `live_instance`, `STOP_RETRY`)
- Test: `crates/iii-compose/src/sandbox.rs`, `crates/iii-compose/src/state.rs`

**Interfaces:**
- Consumes: P0 `WorkerStatusRequest`, `WorkerStatus`, `State`, `WorkerStopRequest`, `WorkerStartResponse`, `SandboxErrorCode`, `code_of`, `adapter_function_id`; P1's S502/S503 payloads, whose `fix` is the live `{vm_id, instance}`; `iii_sdk::IIIClient` (`Clone`).
- Produces (registry §5): `Outcome::Unknown`; `pub enum Child { Process(Supervised), Vm(RemoteVm) }`; `pub type Children = BTreeMap<String, Child>`; `RemoteVm { pub record: VmRecord }` with `poll(&self) -> Outcome`, `async wait(&self) -> ExitStatus`, `async stop(&self, grace: Duration) -> ExitStatus`; `VmRecord`; `ChildRecord.{pid, birth, vm}`. Not in registry: `impl Child { pub fn poll(&self) -> Outcome; pub async fn wait(&self) -> ExitStatus; pub async fn stop(&self, grace: Duration) -> ExitStatus; pub fn is_adopted(&self) -> bool }`; `ChildRecord::from_child(child: &Child, status: ChildStatus) -> Self`; `pub(crate) fn RemoteVm::spawn(client: IIIClient, record: VmRecord, token: String, logs: Option<LogSink>) -> RemoteVm` (starts the poller: `worker::status` every 1 s into a watch channel, ending at a terminal outcome or when the `RemoteVm` drops; `logs: None` is an adopted VM, which is what `is_adopted` reports; Task 7 drives the log stream from `logs`); `pub(crate) fn outcome_of(status: std::result::Result<WorkerStatus, iii_sdk::Error>) -> Outcome`; `pub(crate) fn live_instance(error: &iii_sdk::Error) -> Option<iii_sandbox_contract::WorkerStartResponse>` (the S502/S503 payload's `fix` read as `{vm_id, instance}`; None for any other error); `const STOP_RETRY: Duration = Duration::from_secs(60)`; `#[derive(Clone)] pub(crate) struct LogSink { container: String, sender: mpsc::Sender<LogMessage> }` and `pub(crate) fn LogStore::sink(&self, container: &str) -> LogSink` (a clone of the store's sender, so a detached task can write).

- [ ] **Step 1: Write the failing tests**

```rust
// sandbox.rs, Review Focus 1
#[cfg(unix)] #[test] fn bus_errors_are_unknown_and_s504_is_an_exit_without_code() {
    let remote = |code: &str, message: &str| iii_sdk::Error::Remote { code: code.into(), message: message.into(), stacktrace: None };
    let status = |state, exit_code, signal| WorkerStatus { vm_id: "v".into(), instance: 1, state, exit_code, signal, started_at_ms: 0, ended_at_ms: None };
    assert_eq!(outcome_of(Err(iii_sdk::Error::Timeout)), Outcome::Unknown);
    assert_eq!(outcome_of(Err(remote("function_not_found", "gone"))), Outcome::Unknown);
    let Outcome::Exited(s) = outcome_of(Err(remote("invocation_failed", &error_payload(SandboxErrorCode::S504, "no vm", None, None).to_string()))) else { panic!() };
    assert_eq!(s.code(), None);
    let stale = remote("invocation_failed", &error_payload(SandboxErrorCode::S503, "stale", Some(serde_json::json!({"vm_id": "v", "instance": 3})), None).to_string());
    assert_eq!(live_instance(&stale).map(|l| (l.vm_id, l.instance)), Some(("v".to_string(), 3)));
    let Outcome::Exited(s) = outcome_of(Err(stale)) else { panic!() }; assert_eq!(s.code(), None); // S503 after the retry: superseded
    assert!(live_instance(&iii_sdk::Error::Timeout).is_none());
    assert_eq!(outcome_of(Ok(status(State::Preparing, None, None))), Outcome::Running);
    assert_eq!(outcome_of(Ok(status(State::Running, None, None))), Outcome::Running);
    assert_eq!(outcome_of(Ok(status(State::Unknown, None, None))), Outcome::Unknown);
    let Outcome::Exited(s) = outcome_of(Ok(status(State::Exited, Some(3), None))) else { panic!() }; assert_eq!(s.code(), Some(3));
    let Outcome::Exited(s) = outcome_of(Ok(status(State::Stopped, None, Some(15)))) else { panic!() }; assert_eq!(s.code(), None);
}
// state.rs
#[test] fn a_0_24_state_file_still_loads() {
    // {"compose_path": "/srv/app/worker-compose.yaml", "namespace": "app", "containers": {"api": {"pid": 42, "birth": {"start_time": 7}, "status": "ready", "started_at": 1}}}
    // -> pid == Some(42), birth == Some(BirthIdentity::StartTime(7)), vm == None, sandbox_token == None
}
#[test] fn a_vm_record_round_trips_and_reconciles_as_gone() {
    // ChildRecord { pid: None, birth: None, vm: Some(VmRecord { adapter: "sandbox-builtin", namespace: "app", vm_id: "ab", instance: 2 }), status: Ready, .. }
    // serde_json round trip is equal; reconcile(&record) == Reconciliation::Gone
}
```

- [ ] **Step 2: Run them to verify they fail**
Run: `cargo test --locked -p iii-compose --lib -- bus_errors 0_24_state vm_record`
Expected: compile errors `no variant named Unknown` and `cannot find function outcome_of` (and `live_instance`).

- [ ] **Step 3: Implement the types and every match site**
`outcome_of`: `Preparing`/`Running` are `Running`; `Unknown` and every error except a parsed S503 or S504 are `Unknown`; `Exited`/`Stopped`, S503 (superseded) and S504 are `Exited` via `ExitStatusExt::from_raw` (unix: code `c` is `(c & 0xff) << 8`, signal `s` is `s`, no code is SIGKILL so `code()` is `None`; windows: `code.unwrap_or(1) as u32`). The poller and `stop` handle an S503 before mapping it: they send `worker::stop` to its `live_instance` (the poller with `grace_ms` = `DEFAULT_STOP_GRACE`, `stop` with its own grace) and retry their own call once, so the restart policy never starts a second VM beside a live one; an S503 on the retry maps through `outcome_of` to the no-code exit (registry §5 Supervision). `stop` sends `worker::stop { grace_ms: grace.as_millis() }` with a timeout of grace + 10 s and maps a parsed reply with `outcome_of`; a parsed reply that is not `Exited` returns the no-code status. A reply that is not a parsed worker reply (a bus error, a timeout, `function_not_found` during an engine reconnect) is never taken as stopped: `stop` retries every 1 s, ending early when the watch shows `Exited`, for up to `STOP_RETRY`. Past that, it prints `{adapter}: VM {vm_id} instance {instance} did not answer worker::stop; it may still be running` with `report::daemon_line` (`Tone::Warn`) and returns the no-code status, so `down` cannot hang on an adapter that is gone (its lifeline took its VMs, P1). `wait` resolves at the first `Exited` from the watch.

- [ ] **Step 4: Run the tests to verify they pass**
Run: `cargo test --locked -p iii-compose`
Expected: `test result: ok` for every binary.

- [ ] **Step 5: Commit**
`git add crates/iii-compose/src/process/mod.rs crates/iii-compose/src/process/unix.rs crates/iii-compose/src/logs.rs crates/iii-compose/src/lifecycle.rs crates/iii-compose/src/state.rs crates/iii-compose/src/engine.rs crates/iii-compose/src/project.rs crates/iii-compose/src/sandbox.rs`
`git commit -m "refactor(compose): supervise children through a Child that can be a remote VM"`

---

### Task 7: Boot VM containers through the adapter's worker functions

**Files:**
- Modify: `crates/iii-compose/src/manifest.rs:32-33` (the `StartSpec::Vm` doc says the sandbox adapter, not `iii-worker`, applies the source rules), `:37-49` (`VmSpec::Bundle { package: String, install_dir: PathBuf }`), `crates/iii-compose/src/lifecycle.rs:1059-1062` (`package: installed.name`)
- Modify: `crates/iii-compose/src/lifecycle.rs:114-115` (`vm_dir` becomes `state_dir`), `:1116-1155` (the VM branch calls `sandbox::start_vm` and yields `Child::Vm`; the process branch yields `Child::Process` with `logs.capture` as today), delete `:1199-1355` (`vm_command`, `VmPlan`, `prepare_vm`, `worker_binary`), `:675-716` (`remove_one_scoped`: after the stop, a record with `vm` calls `destroy_vm`; an error is the warning line `{key}: could not destroy VM {vm_id} through {adapter}: {error}; its state stays in ~/.iii/sandbox/workers/{vm_id}`, and the removal still succeeds)
- Modify: `crates/iii-compose/src/project.rs:852-857` (delete `vm_dir`; the six `LifecycleCtx` literals pass `state_dir: self.store.dir()`), `:740-846` (a record with `vm` and a status other than `Stopped` calls `worker::status` and acts on `vm_reconciliation(owner_alive, reply)`: `Adopt` is `RemoteVm::spawn(.., logs: None)`, with no log stream, like an adopted process; `Gone` and `Stopped` as for a process)
- Modify: `crates/iii-compose/src/sandbox.rs` (`check_info`, `start_request`, `start_vm`, `destroy_vm`, `vm_reconciliation`, `spawn_failed_message`, the poller's log stream, `PREPARE_TIMEOUT`, `INFO_RETRY`)
- Modify: `crates/iii-compose/src/logs.rs` (`LogSink::write_record`, `LogSink::capture_records`), `crates/iii-compose/src/error.rs` (`SandboxAdapterMismatch` and its `code()` arm)
- Test: `crates/iii-compose/src/sandbox.rs`, `crates/iii-compose/src/logs.rs`

**Interfaces:**
- Consumes: Task 3 `adapter_name`, `adapter_key`, `state_key`; Task 5 `LifecycleCtx.sandbox_token`; Task 6 `RemoteVm::spawn`, `Child`, `outcome_of`, `live_instance`, `LogSink`, `LogStore::sink`; P0 `AdapterInfo`, `AdapterInfoRequest`, `ContractVersion`, `ADAPTER_INFO_SUFFIX`, `adapter_function_id`, `CONTRACT_MAJOR`, `Worker{Prepare,Start,Logs,Stop,Destroy}Request`, `WorkerStartResponse`, `LogRecord`, `WorkerSource`, `StreamChannelRef`, `code_of` (import the contract as `iii_sandbox_contract as contract`: `WorkerSource` and `LogStream` clash with `config.rs:85`, `logs.rs:44`); `iii_sdk::helpers::create_channel`; `state::Reconciliation`; P1 `sandbox-builtin::worker::*` and its S501/S502/S505 guards.
- Produces: `ComposeError::SandboxAdapterMismatch { container, expected, found }`, code `SANDBOX_ADAPTER_MISMATCH`, registry §5 text. Not in registry: `pub(crate) fn check_info(info: &AdapterInfo, adapter: &str, adapter_key: &str, container: &str) -> Result<()>`; `pub(crate) fn start_request(token: &str, state_dir: &Path, engine_url: &str, key: &str, start: &StartSpec, env: BTreeMap<String, String>) -> Result<WorkerStartRequest>`; `pub(crate) async fn start_vm(ctx: &LifecycleCtx<'_>, key: &str, start: &StartSpec, env: BTreeMap<String, String>) -> Result<RemoteVm>`; `pub(crate) async fn destroy_vm(engine: &EngineClient, token: &str, state_dir: &Path, key: &str, vm: &VmRecord) -> Result<()>`; `pub(crate) fn vm_reconciliation(owner_alive: bool, status: std::result::Result<WorkerStatus, iii_sdk::Error>) -> Reconciliation`; `pub(crate) fn spawn_failed_message(error: &iii_sdk::Error) -> String`; `pub(crate) async fn LogSink::write_record(&self, record: &contract::LogRecord)`; `pub(crate) fn LogSink::capture_records(&self, reader: ChannelReader) -> tokio::task::JoinHandle<Option<u64>>` (resolves, when the channel closes, to the last `LogRecord.cursor` it forwarded); `LifecycleCtx.state_dir`.

`start_vm` runs, in the project namespace: (1) `<adapter>::adapter::info`, `function_not_found` retried every 200 ms for `INFO_RETRY` = 10 s, then `check_info`; (2) `worker::prepare { auth, source }` with `PREPARE_TIMEOUT` = 20 min (today `lifecycle.rs:1281`); (3) `worker::start(start_request(..))` within the container's `startup_timeout`; on S502, `worker::stop` the `live_instance` with the file's `stop_timeout` and retry the start once; (4) `RemoteVm::spawn(.., logs: Some(ctx.logs.sink(key)))`. A failure is `SpawnFailed { container: key, message: spawn_failed_message(&error) }`.

The poller keeps the log stream: on each tick, when `logs` is set, no capture is live and the outcome is not `Exited`, it calls `create_channel`, then `worker::logs { cursor, follow: true, channel: writer_ref }`, and hands the reader to `capture_records`. `cursor` is `None` the first time (the adapter reads from 0) and then the cursor the last finished capture returned, kept in memory only (no `VmRecord.log_cursor`, D11). A failed `worker::logs` is retried on the next tick. So after an engine restart or a dropped bus closes the channel, a VM that still runs gets its stream back from where it stopped.

- [ ] **Step 1: Write the failing tests**

```rust
// sandbox.rs
#[test] fn adapter_info_checks_the_major_then_the_name() {
    let info = |adapter: &str, major| AdapterInfo { adapter: adapter.into(), adapter_version: "1.0.0".into(), contract: ContractVersion { major, minor: 0 } };
    assert!(check_info(&info("sandbox-builtin", 1), "sandbox-builtin", "sandbox-builtin", "app").is_ok());
    let e = check_info(&info("sandbox-builtin", 2), "sandbox-builtin", "sandbox-builtin", "app").unwrap_err();
    assert_eq!(e.code(), "SPAWN_FAILED");
    assert_eq!(e.to_string(), "container 'app' could not start: adapter sandbox-builtin implements sandbox contract 2.0, but this engine needs major 1; install a sandbox-builtin release whose major version is 1");
    let e = check_info(&info("other", 1), "sandbox-builtin", "sb", "app").unwrap_err();
    assert_eq!(e.code(), "SANDBOX_ADAPTER_MISMATCH");
    assert_eq!(e.to_string(), "container 'sb' is the sandbox adapter 'sandbox-builtin', but adapter::info names 'other'. Set engine.sandbox.adapter to 'other' or install 'sandbox-builtin'");
}
#[test] fn start_request_follows_spec_7_4() {
    let (dir, env) = (Path::new("/srv/app/.iii/compose/shop"), BTreeMap::from([("A".to_string(), "1".to_string())]));
    let bundle = StartSpec::Vm(VmSpec::Bundle { package: "onboarding".into(), install_dir: "/c/onboarding/1.0.0".into() });
    let r = start_request("t", dir, "ws://127.0.0.1:49134", "web", &bundle, env.clone()).unwrap();
    assert_eq!((r.auth.as_str(), r.state_key.clone(), r.worker_name.as_str()), ("t", state_key(dir, "web"), "web"));
    // r.source == WorkerSource::Bundle { package: "onboarding", host_path: "/c/onboarding/1.0.0" }
    // r.env == env; r.engine_url == "ws://127.0.0.1:49134"
    // Local { worker_dir: "/srv/app/w", run_override: Some("npm start") } -> WorkerSource::Local { host_path: "/srv/app/w", run_override: Some("npm start") } (no resources or sync, D3)
}
#[test] fn vm_recovery_follows_the_owner_and_the_status() {
    // with remote/status/error_payload as in Task 6's test
    assert_eq!(vm_reconciliation(true, Ok(status(State::Running, None, None))), Reconciliation::Adopt);
    assert_eq!(vm_reconciliation(true, Err(iii_sdk::Error::Timeout)), Reconciliation::Adopt); // Unknown: the poller settles it
    assert_eq!(vm_reconciliation(true, Ok(status(State::Exited, Some(0), None))), Reconciliation::Gone);
    assert_eq!(vm_reconciliation(true, Err(remote("invocation_failed", &error_payload(SandboxErrorCode::S504, "no vm", None, None).to_string()))), Reconciliation::Gone);
    assert_eq!(vm_reconciliation(false, Ok(status(State::Running, None, None))), Reconciliation::Stopped); // the adapter's lifeline took it
}
#[test] fn spawn_failed_message_prefers_the_s_code_message() {
    let e = remote("invocation_failed", &error_payload(SandboxErrorCode::S501, "host_path /etc is outside the host roots", None, None).to_string());
    assert_eq!(spawn_failed_message(&e), "host_path /etc is outside the host roots");
    assert_eq!(spawn_failed_message(&remote("function_not_found", "gone")), remote("function_not_found", "gone").to_string());
}
// logs.rs
#[tokio::test] async fn vm_log_records_reach_compose_logs() {
    store.sink("web").write_record(&LogRecord { stream: contract::LogStream::Stderr, line: "\u{1b}[31mboom\u{1b}[0m".into(), ts_ms: 1, cursor: 9 }).await;
    // store.query(vec!["web".into()], BTreeMap::new(), 10, None, Duration::from_secs(2)) -> entries == [LogEntry { stream: LogStream::Stderr, message: "boom".into() }]
}
```

- [ ] **Step 2: Run them to verify they fail**
Run: `cargo test --locked -p iii-compose --lib -- adapter_info_checks start_request_follows vm_recovery spawn_failed_message vm_log_records`
Expected: compile errors `cannot find function check_info`, `cannot find function vm_reconciliation` and `missing field package in VmSpec::Bundle`.

- [ ] **Step 3: Implement the VM start path, logs, destroy and recovery; delete the `iii-worker` path**
`write_record` runs the line through `TerminalSanitizer` and `send_line` (the 64 KiB split and writer `pump_stream` uses); `capture_records` registers an `on_message` callback that parses each text frame as a `LogRecord` and forwards it, through a task that drives `next_binary` until the channel closes, to the same `send_line` on the sink's sender; the task returns the last cursor it forwarded. `vm_reconciliation`: owner gone is `Stopped`; otherwise `outcome_of(status)` `Running` or `Unknown` is `Adopt` and `Exited` is `Gone`. `spawn_failed_message` is the S-code payload's `message` when `code_of` parses it, else the error's `Display`. `BundleNeedsAVm` (`registry.rs:417-423`) stays as the windows guard, because `install_bundle` is unix-only (`registry.rs:633`) (D11).

- [ ] **Step 4: Run the tests to verify they pass**
Run: `cargo test --locked -p iii-compose` then `rg -n "__bundle-prepare|__local-prepare|__vm-boot|worker_binary|vm_dir|VmPlan" crates/iii-compose/src`
Expected: `test result: ok` for every binary; `rg` prints nothing. (The `crates/iii-worker` mentions at `lib.rs:12` and `manifest.rs:12` go with that crate in P3 Task 5; `registry.rs:220` names `iii-worker` only as history and stays.)

- [ ] **Step 5: Commit**
`git add crates/iii-compose/src/manifest.rs crates/iii-compose/src/lifecycle.rs crates/iii-compose/src/project.rs crates/iii-compose/src/sandbox.rs crates/iii-compose/src/logs.rs crates/iii-compose/src/error.rs`
`git commit -m "feat(compose): boot VM containers through the sandbox adapter's worker functions"`

---

### Task 8: Compose e2e on `sandbox-builtin` 1.0.0

**Files:**
- Create: `engine/tests/compose_sandbox_e2e.rs` (helpers copied from `engine/tests/compose_managed_engine_e2e.rs:8-60`)
- Create: `engine/tests/fixtures/sandbox-vm-worker/iii.worker.yaml` (`name: sandbox-vm-worker`, `runtime: {kind: javascript, base_image: docker.io/iiidev/node:latest}`, `scripts: {install: npm install, start: node index.js}`), `package.json` (dependency `"iii-sdk": "0.24.6"`), `index.js` (registers `sandbox-vm-worker::hello` returning the contents of `reply.txt`), `reply.txt`
- Modify: `.github/workflows/ci.yml:363` (new job `compose-sandbox-e2e-linux` after `compose-test-matrix`: `ubuntu-24.04`, checkout, `dtolnay/rust-toolchain@stable`, rust-cache, the "Enable KVM" step from `:538-553`, `sudo apt-get install -y libcap-ng0`, then the run command below; no `continue-on-error`)
- Test: `engine/tests/compose_sandbox_e2e.rs`

**Interfaces:**
- Consumes: Tasks 1-7 through the `iii` binary; P1 `sandbox-builtin` 1.0.0 in the registry; the registry bundle `onboarding` (workers repo `onboarding/iii.worker.yaml`, `deploy: bundle`); P2a router forwarding `sandbox::*`.
- Produces: the spec 11 primary acceptance run and the one `sandbox::exec` smoke through the router, in iii CI, pinned so a new registry release cannot break iii CI (D5).

- [ ] **Step 1: Write the tests** (each `#[ignore = "boots microVMs; run by compose-sandbox-e2e-linux"]`, `HOME` set to a temp dir, `iii compose --namespace sbx-e2e --up` in the project dir, no `iii-worker` on `PATH`). Pins: `const SANDBOX_BUILTIN: &str = "1.0.0"; const ONBOARDING: &str = "0.1.20";` (D5). Each project file carries the adapter exactly as Compose writes it (`  # added by compose::add\n  sandbox-builtin:\n    worker: package://api.workers.iii.dev/sandbox-builtin\n    version: {SANDBOX_BUILTIN}\n`). It is pre-declared, not left for `up` to write, because `up` would resolve `^1` against the live registry, which D5 rules out for a required check; Task 4's tests cover the write itself.

```rust
#[test] #[ignore = "boots microVMs; run by compose-sandbox-e2e-linux"]
fn a_registry_bundle_runs_through_sandbox_builtin() {
    // worker-compose.yaml: engine.url ws://127.0.0.1:<free port>; containers.onboarding {worker: package://onboarding, version: ONBOARDING, restart: always}; the pinned adapter
    // after up the file bytes are unchanged: Compose takes the generated adapter as its own and does not re-resolve it on up
    // compose::status: sandbox-builtin and onboarding are ready
    // #[cfg(target_os = "linux")] the process whose cmdline holds "__vm-boot" has the sandbox-builtin process as parent, not compose
    // compose::logs container=onboarding returns at least one entry
    // sandbox::create {"image": "python"}, then sandbox::exec {sandbox_id, "cmd": "python3", "args": ["-c", "print(1)"]}: exit_code 0, stdout "1\n"; sandbox::stop
    // SIGKILL the __vm-boot process: onboarding is ready again within 60 s
    // compose::down returns within stop_timeout + 10 s and no __vm-boot process remains
    // up again, then compose::remove worker=onboarding: $HOME/.iii/sandbox/workers holds no entry, and the file no longer has a sandbox-builtin entry (its last VM went, D11)
}
#[test] #[ignore = "boots microVMs; run by compose-sandbox-e2e-linux"]
fn a_local_base_image_worker_runs_and_syncs_its_source() {
    // copy fixtures/sandbox-vm-worker into the project; containers.sandbox-vm-worker {worker: path://./sandbox-vm-worker, restart: always}; the pinned adapter
    // iii trigger --namespace sbx-e2e sandbox-vm-worker::hello == contents of reply.txt
    // rewrite reply.txt, compose::restart worker=sandbox-vm-worker, then hello returns the new text
    // SIGKILL its __vm-boot process: sandbox-vm-worker is ready again within 60 s
    // compose::down returns within stop_timeout + 10 s and no __vm-boot process remains
    // up again, then compose::remove worker=sandbox-vm-worker: $HOME/.iii/sandbox/workers holds no entry, and the file no longer has a sandbox-builtin entry
}
```

- [ ] **Step 2: Run them to verify the harness compiles and skips by default**
Run: `cargo test --locked -p iii --test compose_sandbox_e2e`
Expected: `2 ignored`, `test result: ok`.

- [ ] **Step 3: Add the fixture and the CI job** as listed under Files.

- [ ] **Step 4: Run the e2e on a Linux x86_64 host with `/dev/kvm`, and once by hand on an Apple Silicon Mac**
Run: `cargo test --locked -p iii --test compose_sandbox_e2e -- --ignored --nocapture`
Expected: `test result: ok. 2 passed`. The workflow edit is proven by the job going green on the PR (Task 9).

- [ ] **Step 5: Commit**
`git add engine/tests/compose_sandbox_e2e.rs engine/tests/fixtures/sandbox-vm-worker .github/workflows/ci.yml`
`git commit -m "test(compose): run bundle and base-image workers through sandbox-builtin in CI"`

---

### Task 9: Whole-plan verification and PR note

**Files:** none changed.

- [ ] **Step 1: Format, tests and build**
Run: `cargo fmt --all -- --check && cargo test --locked -p iii-compose && cargo build --locked -p iii && cargo test --locked -p iii --test compose_daemon_e2e --test compose_managed_engine_e2e --test compose_project_state_e2e --test compose_process_title_e2e`
Expected: no diff from fmt; every `test result: ok`.

- [ ] **Step 2: Spec 11 checks this slice owns**
Run: `cargo test --locked -p iii-compose -- sandbox frozen_up adapter engine_sandbox materialized_config replacement`
Expected: all pass. They cover both migration messages and the unmarked replacement YAML (Task 2), the generated container's marker, `^1` resolution, removal with its last VM only from update and remove, and `add iii-sandbox` keeping it (Task 4), `--frozen` refusing (Task 4), `UNSUPPORTED_PLATFORM` (Task 4), and the S1 binding of a Compose-managed engine without `engine.sandbox` (Task 1). The primary acceptance test and the `sandbox::exec` smoke are `compose-sandbox-e2e-linux` (Task 8), green on the PR.

- [ ] **Step 3: Compose no longer execs `iii-worker` (P3's precondition, Compose half)**
Run: `rg -n '"iii-worker"|iii-worker\{' crates/iii-compose/src`
Expected: no output (the `iii-worker` comments P3 rewords do not match).

- [ ] **Step 4: PR-ready note** (do not push or open the PR here)
Title `feat(compose): sandbox adapter support (MOT-<ticket>)`. Body: Compose binds the router (S1), migrates `engine.workers.iii-sandbox` with `SANDBOX_CONFIG_MOVED`, installs `sandbox-builtin` `^1` as a generated container, and drives VM containers over `worker::*`; it is stacked on P2a (`sandbox/p2a-router`), which merges once this PR is approved and `sandbox-builtin` 1.0.0 resolves for the three targets; this PR then merges right after P2a, and P3 right after it (index row 3); `compose-test-matrix` (ubuntu, macos, windows) and `compose-sandbox-e2e-linux` must be green (making the latter required is the index's branch-protection step, D5); P3 then deletes the VM crates. No AI co-author or "Generated with" line.
