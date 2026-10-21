# Sandbox Engine Router (P2a) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The engine owns `sandbox::*` in `default` through a built-in `iii-sandbox` router that forwards each call to the adapter bound in `EngineConfig.sandbox`, and it no longer spawns `iii-worker`.

**Architecture:** A new in-process worker `engine/src/workers/sandbox/` registers the 16 ephemeral ids plus `sandbox::adapter::info` from the contract crate's `OPERATIONS` and forwards opaque JSON with `call_value_with_metadata_ns` to `<adapter>::<suffix>` in the binding namespace, after a cached `<adapter>::adapter::info` handshake. The binding comes from a new `sandbox:` engine-config key, injected into the router's worker entry by one `normalize_worker_entries` helper used at build and reload. The `KNOWN_EXTERNAL` spawn path, its port/config plumbing and the learn-iii prefetch are deleted; WebSocket workers can no longer register `sandbox::*` in `default`.

**Tech Stack:** Rust 2024 (`iii` engine crate), tokio, serde/serde_json/serde_yaml, schemars 0.8, `inventory` (`register_worker!`), `iii-sandbox-contract` (P0, path dependency), `tokio-tungstenite` (fake adapter), GitHub Actions YAML and the `.github/scripts` Python validators.

**Spec:** docs/superpowers/specs/2026-10-20-sandbox-worker-design.md, sections 2, 5.2, 5.6, 5.7 (router side), 6.1, 6.2, 6.3 (engine part), 6.4 (`CONFIGURABLE_ENGINE_WORKERS` entry with its Compose fixtures, D9, and the engine `migration_guidance` arm), 6.5 (engine code, tests, publish rows), 7.5, 8.2 (README and skill move, D10), 10 (step 2), 11 (engine tests), 12 (keep `_publish-engine-workers.yml` working). Shared names: docs/superpowers/plans/2026-10-20-sandbox-shared-interfaces.md

**Repo and base:** iii repo. Citations are at `a1d08d8b0`. Branch `sandbox/p2a-router`, cut from main after P0 merges (P0 makes `crates/iii-sandbox-contract` a workspace member). It merges once `sandbox-builtin` 1.0.0 resolves for the three targets (the P3 Task 7 Step 3 curl) and P2b is approved, so P2a, P2b and P3 merge back to back. No stable tag is cut from main between its merge and 0.25.0 (registry §1), so that window covers only this run. Engine 0.25.0 is not cut until P3 merges.

## Global Constraints

- Contract wire version: `CONTRACT_MAJOR = 1`, `CONTRACT_MINOR = 0`; "Only a major bump breaks."
- Error wire format stays `ErrorBody{code: "invocation_failed", message: <SandboxErrorWire JSON>}`; "the router passes adapter errors through untouched."
- "The router never deserializes payloads. The forward is a fresh invocation without the caller's timeout or cancellation."
- Registration: "every ephemeral id plus `sandbox::adapter::info` is registered through `EngineTrait::register_function` (namespace `default`), with schemas from the crate", also with no binding, "so ownership stays pinned; calls get S303."
- Handshake: "lazy and cached. On the first forward it calls `<adapter>::adapter::info`. A different major gives S302. The cache is cleared on S301 or `invocation_stopped`. There is no attach protocol."
- S303 text: "no sandbox adapter configured: this engine is not managed by a Compose file; set sandbox.adapter in its engine config (Compose-managed engines default to sandbox-builtin)".
- S301 text: "adapter sandbox-builtin not connected in namespace my-proj; compose::add iii-sandbox in that project installs it" (adapter and namespace substituted).
- `ENGINE_RESERVED_PREFIXES = ["sandbox::"]`; in `default` only: `RegistrationRejected{FUNCTION_NAMESPACE_CONFLICT, owner_worker_id: ""}`, logged as "sandbox::* is served by the engine's sandbox router; adapters register <adapter>::*". "Other namespaces stay allowed."
- `EngineConfig` gains `#[serde(default)] sandbox: Option<SandboxRouterConfig>`, "with `deny_unknown_fields`, adapter names `[a-z0-9-]+` and the namespace checked by `iii_compose::namespace::check`"; "Bad values fail with `[INVALID_SANDBOX_CONFIG]`"; "Adapter settings are not engine config."
- The router keeps the worker name `iii-sandbox` (S7) and is registered `mandatory`.
- Anti-goals: no change to any `sandbox::*` request or response shape or the S-code wire format; the engine never depends on a workers-repo artifact at build time; no worker takes over `sandbox::*` by registering first.
- Main stays green after every commit (spec 10); "Router tests use a fake adapter (`fake::*`)".
- "Never release an engine without `iii-worker` before `sandbox-builtin` is in the registry."
- Commits are conventional; no AI co-author trailer and no "Generated with" line; the PR title carries the MOT ticket.

## Review Focus

1. A consumer's lenient payload (exec `cmd` as a string, `env` as a list, a `StreamChannelRef` object, a field the schema does not know) must reach the adapter byte-identical: the router forwards opaque JSON and never round-trips it through contract types. Pin: Task 2 `forwards_across_namespaces_with_payload_and_metadata_untouched`.
2. On an engine with no `sandbox:` key, `sandbox::adapter::info` must answer `{adapter: null, namespace: null, contract: {major: 1, minor: 0}, info: null}` and never S303, because the P5 sandbox-code-runner probe reads it (Compose never does, D11). Pin: Task 2 `unbound_router_answers_s303_but_info_never_errors`.
3. An adapter that drops mid-call and comes back on another contract major (an operator pins a 2.x adapter and restarts it) must get S302 on the next call, not a forward on the stale handshake. Pin: Task 2 `invocation_stopped_on_adapter_disconnect_clears_handshake`.
4. An operator who edits `sandbox.adapter` in a running External-mode engine's config expects the hot reload to rebind the router, not keep the old binding. Pin: Task 3 `changed_sandbox_binding_marks_router_changed`.
5. `sandbox: {adapter: sandbox-builtin, namespace: ""}` (for example a quoted `"${NS:}"`) passes `iii_compose::namespace::check`, which ignores blanks (`crates/iii-compose/src/namespace.rs:57-59`); it must fail at load with `[INVALID_SANDBOX_CONFIG]` instead of a router that forwards to namespace `""`. Pin: Task 1 `validate_rejects_bad_namespaces`.

---

### Task 1: `engine.sandbox` binding and validation

**Files:**
- Create: `engine/src/workers/sandbox/mod.rs`, `engine/src/workers/sandbox/config.rs`
- Modify: `engine/Cargo.toml:130` (add `iii-sandbox-contract = { path = "../crates/iii-sandbox-contract" }` after `iii-compose`), `Cargo.lock`
- Modify: `engine/src/lib.rs:25-45` (add `pub mod sandbox;` to the `workers` block)
- Modify: `engine/src/workers/config.rs:57-68` (field), `:74-82` (`Default`), `:149` (validation call), `:2610`, `:2643`, `:2686` (struct literals)
- Modify: `engine/src/logging.rs:1310`, `engine/tests/reload_scope_unit.rs:193, 269` (struct literals get `sandbox: None`)
- Test: `engine/src/workers/sandbox/config.rs` (unit), `engine/src/workers/config.rs` (tests module)

**Interfaces:**
- Consumes: P0 `iii_sandbox_contract::is_valid_adapter_name(name: &str) -> bool`; `iii_compose::namespace::check(value: &str) -> Result<(), &'static str>` (`crates/iii-compose/src/namespace.rs:59`).
- Produces (registry 4): `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(deny_unknown_fields)] pub struct SandboxRouterConfig { pub adapter: String, #[serde(default = "default_namespace")] pub namespace: String }` (default `"default"`); `impl SandboxRouterConfig { pub fn validate(&self) -> anyhow::Result<()> }`; `EngineConfig { #[serde(default)] pub sandbox: Option<SandboxRouterConfig>, .. }`.

- [ ] **Step 1: Write the failing tests.** Add the `Cargo.toml` line and `pub mod sandbox;`; `mod.rs` holds only `pub mod config;`; `config.rs` holds only these tests (`#[cfg(test)] mod tests`).

```rust
// engine/src/workers/sandbox/config.rs
#[test] fn namespace_defaults_to_default() {
    let c: SandboxRouterConfig = serde_yaml::from_str("adapter: sandbox-builtin").unwrap();
    assert_eq!(c.namespace, "default");
    c.validate().unwrap();
}
#[test] fn unknown_keys_are_rejected() {
    assert!(serde_yaml::from_str::<SandboxRouterConfig>("adapter: x\nimage_allowlist: []").is_err());
}
#[test] fn validate_rejects_bad_adapter_names() {
    for bad in ["", "Sandbox-Builtin", "sandbox", "sandbox_builtin", "fake::x"] {
        let err = SandboxRouterConfig { adapter: bad.into(), namespace: "default".into() }
            .validate().unwrap_err().to_string();
        assert_eq!(err, format!("[INVALID_SANDBOX_CONFIG] sandbox.adapter '{bad}' must match [a-z0-9-]+ and must not be 'sandbox'"));
    }
}
#[test] fn validate_rejects_bad_namespaces() {
    for (bad, reason) in [
        ("My-Proj", "namespaces are lowercase, and are not lowercased for you"),
        ("my proj", "it may hold only a-z, 0-9, '-' and '_'"),
        ("", "it is blank"),
    ] {
        let err = SandboxRouterConfig { adapter: "fake".into(), namespace: bad.into() }
            .validate().unwrap_err().to_string();
        assert_eq!(err, format!("[INVALID_SANDBOX_CONFIG] sandbox.namespace '{bad}': {reason}"));
    }
}
// engine/src/workers/config.rs, tests module
#[test] fn config_file_reads_and_validates_sandbox_binding() {
    // "sandbox:\n  adapter: fake\n  namespace: fake-ns\nworkers: []\n"
    //   -> cfg.sandbox == Some(SandboxRouterConfig { adapter: "fake".into(), namespace: "fake-ns".into() })
    // "workers: []\n" -> cfg.sandbox == None
    // "sandbox:\n  adapter: sandbox\n" -> err.to_string().contains("[INVALID_SANDBOX_CONFIG] sandbox.adapter 'sandbox'")
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p iii --all-features --lib sandbox`
Expected: compile errors `cannot find type SandboxRouterConfig in this scope` and `no field sandbox on type EngineConfig`. (No `--locked` here: this run also refreshes `Cargo.lock` for the new path dependency.)

- [ ] **Step 3: Implement `SandboxRouterConfig` and `validate(&self) -> anyhow::Result<()>` in `engine/src/workers/sandbox/config.rs`**

Adapter first (`is_valid_adapter_name`), then namespace: `""` gives reason `it is blank`, otherwise `iii_compose::namespace::check`'s reason, in the exact texts the tests pin. `EngineConfig::config_file` calls `sandbox.validate()?` right after `cfg.validate_declared_workers()?` (`config.rs:149`), so startup and reload (`reload.rs:122`) both reject bad values before any worker changes. Every `EngineConfig { .. }` literal listed above gets `sandbox: None`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --locked -p iii --all-features --lib sandbox` and `cargo test --locked -p iii --all-features --test reload_scope_unit`
Expected: the five new tests pass; `reload_scope_unit` passes unchanged.

- [ ] **Step 5: Commit**

```bash
git add engine/Cargo.toml Cargo.lock engine/src/lib.rs engine/src/workers/sandbox/mod.rs engine/src/workers/sandbox/config.rs engine/src/workers/config.rs engine/src/logging.rs engine/tests/reload_scope_unit.rs
git commit -m "feat(engine): add the engine.sandbox adapter binding with INVALID_SANDBOX_CONFIG validation"
```

### Task 2: `SandboxRouter` forwarding and handshake

**Files:**
- Create: `engine/src/workers/sandbox/router.rs`
- Modify: `engine/src/workers/sandbox/mod.rs` (`mod router; pub use router::SandboxRouter;`)
- Test: `engine/tests/sandbox_router_e2e.rs` (create)

**Interfaces:**
- Consumes (P0, registry 2): `OPERATIONS` (the 16 ephemeral rows only; no `Family`, D3), `Operation { suffix, description, request_schema, response_schema }`, `Operation::public_id(&self) -> String`, `ROUTER_INFO_ID`, `ADAPTER_INFO_SUFFIX`, `CONTRACT_MAJOR`, `ContractVersion::CURRENT`, `AdapterInfo`, `AdapterInfoRequest`, `RouterInfo`, `adapter_function_id(adapter: &str, id: &str) -> String`, `SandboxErrorCode::{S211, S301, S302, S303}`, `error_payload(code, message: &str, fix: Option<Value>, fix_note: Option<&str>) -> Value`, `code_of(message: &str) -> Option<SandboxErrorCode>`.
- Consumes (engine): `Engine::call_value_with_metadata_ns(&self, namespace: &str, function_id: &str, input: Value, metadata: Option<Value>) -> Result<Option<Value>, ErrorBody>` (`engine/src/engine/mod.rs:2543`, `pub(crate)`); `EngineTrait::register_function(&self, RegisterFunctionRequest, Box<dyn FunctionHandler + Send + Sync>)` (`mod.rs:325-331`); `FunctionHandler::handle_function` (`engine/src/function.rs:75-83`); Task 1 `SandboxRouterConfig`.
- Produces: `#[derive(Clone)] pub struct SandboxRouter { engine: Arc<Engine>, binding: Option<SandboxRouterConfig>, handshake_ok: Arc<AtomicBool> }`; `impl Worker for SandboxRouter` (`name() == "iii-sandbox"`; `create` parses `config` with `serde_json::from_value::<SandboxRouterConfig>`, `None` = unbound; `initialize` is a no-op; `register_functions` registers every `OPERATIONS` row by `public_id()`, `description`, both schemas, plus `ROUTER_INFO_ID` with `AdapterInfoRequest`/`RouterInfo` schemas and `metadata: {"internal": true}`, each with `Box::new(self.clone())`); `impl FunctionHandler for SandboxRouter`.
- Test double (registry 7; helper names not in registry): `async fn spawn_engine(binding: Option<Value>) -> (u16, Arc<Engine>)` = `namespace_conflict_e2e.rs:36-68` plus `SandboxRouter::create` / `initialize` / `register_functions`; `async fn fake_adapter(port: u16, major: u32) -> FakeAdapter`, a raw `tokio_tungstenite` client that calls `engine::workers::register` with `{name: "fake", namespace: "fake-ns"}`, registers `fake::adapter::info` (`{adapter: "fake", adapter_version: "0.0.0", contract: {major, minor: 0}}`), `fake::exec` (result `{"input": data, "metadata": metadata}`), `fake::fs::ls` (error `{code: "invocation_failed", message: error_payload(S211, "file not found: /nope", None, None).to_string()}`) and `fake::create` (never answers; sends its id on `in_flight`), and returns once `engine.functions.get("fake-ns", "fake::exec")` is `Some`; `struct FakeAdapter { in_flight: mpsc::UnboundedReceiver<String>, task: JoinHandle<()> }`, where `task.abort()` drops the socket.

- [ ] **Step 1: Write the failing tests** (all in `engine/tests/sandbox_router_e2e.rs`; calls go through `EngineTrait::call_with_metadata_ns(&engine, "default", id, input, metadata)`; `payload(&err)` = `serde_json::from_str::<Value>(&err.message)`)

```rust
#[tokio::test] async fn router_registers_every_public_id_even_when_unbound() {
    // spawn_engine(None)
    assert_eq!(OPERATIONS.len(), 16);
    for op in OPERATIONS { let f = engine.functions.get("default", &op.public_id()).unwrap();
        assert_eq!(f.request_format, Some((op.request_schema)()));
        assert_eq!(f.response_format, Some((op.response_schema)())); }
    assert_eq!(engine.functions.get("default", ROUTER_INFO_ID).unwrap().metadata, Some(json!({"internal": true})));
}
#[tokio::test] async fn unbound_router_answers_s303_but_info_never_errors() {
    // sandbox::exec -> code_of == Some(S303), payload["retryable"] == false,
    // payload["message"] == "no sandbox adapter configured: this engine is not managed by a Compose file; set sandbox.adapter in its engine config (Compose-managed engines default to sandbox-builtin)"
    // sandbox::adapter::info {} -> Ok(Some(json!({"adapter": null, "namespace": null, "contract": {"major": 1, "minor": 0}, "info": null})))
}
#[tokio::test] async fn bound_router_without_adapter_answers_s301() {
    // spawn_engine(Some(json!({"adapter": "fake", "namespace": "fake-ns"}))), no fake
    // sandbox::exec -> S301, payload["retryable"] == true,
    // payload["message"] == "adapter fake not connected in namespace fake-ns; compose::add iii-sandbox in that project installs it"
    // sandbox::adapter::info -> {"adapter": "fake", "namespace": "fake-ns", "contract": {"major": 1, "minor": 0}, "info": null}
}
#[tokio::test] async fn forwards_across_namespaces_with_payload_and_metadata_untouched() {
    let input = json!({"sandbox_id": "0b6c2f0e-1f7e-4a53-9a43-5d1f8f2b9c11", "cmd": "echo hi", "env": ["A=1"],
        "channel": {"channel_id": "c1", "access_key": "k1", "direction": "write"}, "not_in_schema": true});
    let metadata = json!({"trace": "m1"});
    // sandbox::exec(input, Some(metadata)) -> Ok(Some(json!({"input": input, "metadata": metadata})))
    // sandbox::adapter::info -> ["info"] == {"adapter": "fake", "adapter_version": "0.0.0", "contract": {"major": 1, "minor": 0}}
}
#[tokio::test] async fn adapter_errors_pass_through_untouched() {
    // sandbox::fs::ls -> Err(e): e.code == "invocation_failed",
    // e.message == error_payload(SandboxErrorCode::S211, "file not found: /nope", None, None).to_string()
}
#[tokio::test] async fn s302_when_adapter_major_differs() {
    // fake_adapter(port, 2); sandbox::exec -> S302, payload["retryable"] == false,
    // payload["message"] == "adapter fake implements sandbox contract 2.0, but this engine needs major 1; install a fake release whose major version is 1"
}
#[tokio::test] async fn invocation_stopped_on_adapter_disconnect_clears_handshake() {
    // fake major 1; sandbox::exec succeeds (handshake cached)
    // spawn sandbox::create; in_flight.recv() == "fake::create"; task.abort()
    // the create call returns Err(e) with e.code == "invocation_stopped" within 5 s
    // fake_adapter(port, 2); sandbox::exec -> S302 (proves the handshake re-ran)
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --locked -p iii --all-features --test sandbox_router_e2e`
Expected: `error[E0432]: unresolved import iii::workers::sandbox::SandboxRouter`.

- [ ] **Step 3: Implement `SandboxRouter` in `engine/src/workers/sandbox/router.rs`**

`handle_function` returns `Box::pin(self.route(function_id, input, metadata))`. Private signatures and order (spec 6.1):
- `async fn route(&self, function_id: String, input: Value, metadata: Option<Value>) -> FunctionResult<Option<Value>, ErrorBody>`:
  1. `ROUTER_INFO_ID`: reply `RouterInfo { adapter, namespace, contract: ContractVersion::CURRENT, info: adapter_info(..).ok() }`; never S301 or S303.
  2. No binding: S303.
  3. Unless `handshake_ok`: `adapter_info`, then major `!= CONTRACT_MAJOR` gives S302; on success set `handshake_ok`.
  4. Forward with `call_value_with_metadata_ns(&ns, &adapter_function_id(&adapter, &function_id), input, metadata)`. `Ok(v)` becomes `Success(v)`. Error code `function_not_found` gives S301 and clears `handshake_ok`. `invocation_stopped` clears it and passes through. Any other error passes through.
- `async fn adapter_info(&self, b: &SandboxRouterConfig) -> Result<AdapterInfo, ErrorBody>`: calls `adapter_function_id(&b.adapter, ADAPTER_INFO_SUFFIX)` with `{}`. Any call error gives S301 and clears `handshake_ok`. A reply that does not parse gives S302 with the text `adapter {adapter} returned an adapter::info reply this engine cannot read ({error}); this engine needs sandbox contract major {CONTRACT_MAJOR}` (not in registry).
- `fn sandbox_error(code: SandboxErrorCode, message: &str) -> ErrorBody` = `ErrorBody { code: "invocation_failed".into(), message: error_payload(code, message, None, None).to_string(), stacktrace: None }`. Messages are the registry 4 texts for S301, S302 and S303.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --locked -p iii --all-features --test sandbox_router_e2e`
Expected: `test result: ok. 7 passed`.

- [ ] **Step 5: Commit**

```bash
git add engine/src/workers/sandbox/router.rs engine/src/workers/sandbox/mod.rs engine/tests/sandbox_router_e2e.rs
git commit -m "feat(engine): add the sandbox router that forwards sandbox::* to the bound adapter"
```

### Task 3: The router becomes the mandatory `iii-sandbox` worker

**Files:**
- Modify: `engine/src/workers/sandbox/mod.rs` (`register_worker!`)
- Modify: `engine/src/workers/config.rs:206-222` (arm), `:480-494` (new helper beside `assign_instance_ids`), `:710-729` (build uses helper), `:1287`, `:1304` (fixture), `:1329-1378` (extend test)
- Modify: `engine/src/workers/reload.rs:18-21` (imports: drop `assign_instance_ids`, add `normalize_worker_entries`), `:125-144`
- Modify: `crates/iii-compose/src/config.rs:58-64` (drop `"iii-sandbox"`), `crates/iii-compose/tests/config_validation.rs:121-122, 142, 149`, `crates/iii-compose/src/managed_engine.rs:909-910, 949, 953` (drop the `iii-sandbox` fixture, list item and `auto_install` assertion)
- Modify: `engine/tests/config_reload_e2e.rs:96, 513-573` (stand-in `iii-sandbox` becomes `iii-http-functions`: the const, the store file `iii-http-functions.yaml`, the YAML entry, `register_worker`, the rewrite needle and asserts, the section comment)
- Modify: `engine/config.yaml:24-34` (delete the commented `iii-sandbox` block; add a commented top-level `sandbox:` / `adapter: sandbox-builtin` / `namespace: default` after line 1)
- Modify: `engine/src/workers/telemetry/mod.rs:200-219` (`is_iii_builtin_function_id` gains `|| id.starts_with("sandbox::")`), `:2429-2449` (extend `test_is_iii_builtin_function_id`). The router is engine-owned like `state::`; without this, `engine/tests/builtin_functions_e2e.rs:36-79` fails on a bare engine and telemetry counts the 17 router ids as user functions (`telemetry/mod.rs:548-556`).
- Test: `engine/tests/reload_manager_unit.rs`, `engine/tests/reload_scope_unit.rs`, `engine/src/workers/config.rs`, `engine/src/workers/telemetry/mod.rs`

**Interfaces:**
- Consumes: Task 2 `SandboxRouter`; Task 1 `EngineConfig.sandbox`.
- Produces: `pub fn normalize_worker_entries(config: EngineConfig) -> Vec<WorkerEntry>` (registry 4); `crate::register_worker!("iii-sandbox", SandboxRouter, description = "Routes sandbox::* in the default namespace to the configured sandbox adapter worker (sandbox-builtin by default).", mandatory)` (description text not in registry); `CONFIGURABLE_ENGINE_WORKERS == ["configuration", "iii-worker-manager", "iii-http-functions", "iii-stream"]`; the `migration_guidance` arm.

- [ ] **Step 1: Write the failing tests**

```rust
// engine/tests/reload_manager_unit.rs; parse(yaml) = ReloadManager::parse_and_normalize on write_config(yaml)
#[tokio::test] async fn reload_keeps_sandbox_binding() {
    let yaml = "sandbox:\n  adapter: fake\n  namespace: fake-ns\nworkers: []\n";
    let (first, second) = (parse(yaml).await, parse(yaml).await);
    let router = first.iter().find(|e| e.name == "iii-sandbox").expect("router injected");
    assert_eq!(router.config, Some(json!({"adapter": "fake", "namespace": "fake-ns"})));
    assert!(diff_entries(&first, &second).unchanged.contains(&"iii-sandbox".to_string()));
    let unbound = parse("workers: []\n").await;
    assert_eq!(unbound.iter().find(|e| e.name == "iii-sandbox").unwrap().config, None);
}
#[tokio::test] async fn changed_sandbox_binding_marks_router_changed() {
    let diff = diff_entries(&parse("sandbox:\n  adapter: fake\n").await, &parse("sandbox:\n  adapter: other\n").await);
    assert_eq!(diff.changed.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["iii-sandbox"]);
}
// engine/tests/reload_scope_unit.rs
#[tokio::test] async fn engine_config_binds_the_mandatory_router_on_build() {
    let (mut config, _dir) = minimal_config_for_builder_tests();
    config.sandbox = Some(SandboxRouterConfig { adapter: "fake".into(), namespace: "fake-ns".into() });
    let builder = EngineBuilder::new().with_config(config).build().await.unwrap();
    assert_eq!(builder.engine().call("sandbox::adapter::info", json!({})).await.unwrap(),
        Some(json!({"adapter": "fake", "namespace": "fake-ns", "contract": {"major": 1, "minor": 0}, "info": null})));
    builder.destroy().await.unwrap();
}
// engine/src/workers/config.rs: test_config_file_rejects_all_user_managed_workers_with_migration_guidance
// gains "  - name: iii-sandbox\n" under modules and
assert!(message.contains("'iii-sandbox' -> remove it; set sandbox.adapter (Compose: engine.sandbox) and configure the sandbox-builtin worker"), "{message}");
// engine/src/workers/telemetry/mod.rs: test_is_iii_builtin_function_id gains
assert!(is_iii_builtin_function_id("sandbox::exec"));
assert!(is_iii_builtin_function_id("sandbox::adapter::info"));
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --locked -p iii --all-features --test reload_manager_unit --test reload_scope_unit` and `cargo test --locked -p iii --all-features --lib workers::telemetry::tests::test_is_iii_builtin_function_id`
Expected: `reload_keeps_sandbox_binding` panics `router injected`; `engine_config_binds_the_mandatory_router_on_build` gets `function_not_found`; the telemetry test fails `assertion failed: is_iii_builtin_function_id("sandbox::exec")`.

- [ ] **Step 3: Implement `normalize_worker_entries(config: EngineConfig) -> Vec<WorkerEntry>` in `engine/src/workers/config.rs`**

It concatenates `workers` and `modules`, injects each missing mandatory registration as today's two loops do, and gives the injected `iii-sandbox` entry `config: config.sandbox` as a `serde_json::Value` (`to_value` cannot fail for this struct). It then runs `assign_instance_ids`. `EngineBuilder::build` calls it after setting the grace (it replaces `:710-729`). `ReloadManager::parse_and_normalize` returns `Ok(normalize_worker_entries(cfg))`. Add `register_worker!` and the arm `"iii-sandbox" => "remove it; set sandbox.adapter (Compose: engine.sandbox) and configure the sandbox-builtin worker"`, and remove the list entry. The arm can only fire once the entry is gone (`config.rs:22`, `:160`). Add the `sandbox::` prefix to `is_iii_builtin_function_id`. Then make the fixture, stand-in and example edits listed above.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --locked -p iii --all-features` and `cargo test --locked -p iii-compose`
Expected: both `test result: ok`, with the three new tests, the extended migration and telemetry tests, and `builtin_functions_e2e`'s `all_functions_on_bare_engine_are_iii_builtins` passing. The full engine suite runs here because every engine build now includes the router.

- [ ] **Step 5: Commit**

```bash
git add engine/src/workers/sandbox/mod.rs engine/src/workers/config.rs engine/src/workers/reload.rs engine/src/workers/telemetry/mod.rs crates/iii-compose/src/config.rs crates/iii-compose/tests/config_validation.rs crates/iii-compose/src/managed_engine.rs engine/tests/config_reload_e2e.rs engine/tests/reload_manager_unit.rs engine/tests/reload_scope_unit.rs engine/config.yaml
git commit -m "feat(engine): register the sandbox router as the mandatory iii-sandbox worker"
```

### Task 4: Reserve `sandbox::` in `default`

**Files:**
- Modify: `engine/src/engine/mod.rs:2289-2325` (guard), `:2294-2296` (the stale comment), `:4217-4221` (test doc)
- Modify: `.github/workflows/ci.yml:711-727`: delete the `Sandbox isolation end to end (UBI 9 image, real microVMs)` step (`:711-719`) and its log-upload step (`:721-727`) from `sandbox-isolation-linux`; keep the job's build and `cargo test -p iii-worker` steps. After this commit the engine refuses the old daemon's `sandbox::*` registrations in `default`, and the unbound router answers the script's `sandbox list` readiness probe (`scripts/sandbox-isolation-e2e.sh:90`) with S303, so the step can only fail. P1's `sandbox-builtin-vm-linux` runs the moved isolation e2e (D5); P3 Task 4 deletes the rest of the job.
- Test: `engine/src/engine/mod.rs` tests module (beside `register_function_reserves_non_provider_engine_prefix_outside_default`, `:4222`)

**Interfaces:**
- Consumes: P0 `iii_sandbox_contract::PUBLIC_PREFIX`.
- Produces: `const ENGINE_RESERVED_PREFIXES: &[&str] = &[iii_sandbox_contract::PUBLIC_PREFIX];` (registry 4).

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn register_function_reserves_sandbox_prefix_in_default() {
    // in-process: engine.register_function_handler(RegisterFunctionRequest { description: Some("router".into()), ..make_request("sandbox::exec") }, Handler::new(..))
    // WS worker resolved to DEFAULT_NAMESPACE (as at :4275-4281) dispatches RegisterFunction { id: "sandbox::exec", .. }
    assert_eq!(engine.functions.get(DEFAULT_NAMESPACE, "sandbox::exec").unwrap()._description.as_deref(), Some("router"));
    // the first Outbound::Protocol(Message::RegistrationRejected { code, function_id, owner_worker_id, .. }) on rx:
    assert_eq!(code, FUNCTION_NAMESPACE_CONFLICT); assert_eq!(function_id.as_deref(), Some("sandbox::exec")); assert_eq!(owner_worker_id, "");
    // WS worker resolved to "orders" dispatches the same id:
    assert!(engine.functions.get("orders", "sandbox::exec").is_some());
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --locked -p iii --all-features --lib engine::tests::register_function_reserves_sandbox_prefix_in_default`
Expected: assertion failure, `left: None, right: Some("router")` (the WS registration overwrote the in-process one).

- [ ] **Step 3: Widen the existing reserved-id guard at `engine/src/engine/mod.rs:2297`**

The condition becomes `existing || (namespace == DEFAULT_NAMESPACE && ENGINE_RESERVED_PREFIXES.iter().any(|p| reg_id.starts_with(p)))`. The `RegistrationRejected` reply stays as it is. The `tracing::warn!` for the new branch uses the spec text. Rewrite the comment at `:2294-2296` and the test doc at `:4220-4221`: in-process builtins have no `function_owners` entry (`:398-399`), so in `default` only the reserved list blocks a shadow. Delete the two `ci.yml` steps listed above.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --locked -p iii --all-features --lib engine::tests::register_function_reserves`; `rg -n "sandbox-isolation-e2e.sh|sandbox-isolation-linux-logs" .github/workflows/ci.yml`; `python3 -c 'import yaml; yaml.safe_load(open(".github/workflows/ci.yml"))'`
Expected: `2 passed` (the existing `engine::` test and the new one); `rg` prints nothing; the YAML loads with exit 0.

- [ ] **Step 5: Commit**

```bash
git add engine/src/engine/mod.rs .github/workflows/ci.yml
git commit -m "fix(engine): refuse WebSocket registrations of sandbox::* in the default namespace"
```

### Task 5: Delete the engine-spawned `iii-worker` path

**Files:**
- Delete: `engine/src/workers/external.rs`, `engine/src/workers/secure_temp.rs`, `engine/tests/external_known_sandbox.rs`
- Modify: `engine/src/lib.rs:31, 40`; `engine/src/workers/traits.rs:96-98` (`is_external_process`)
- Modify: `engine/src/workers/config.rs:264-318` (`ExternalProcessWorker`), `:366-372` (doc drops step 3), `:407-427` (step 3), `:516-518` (guard), `:731-765` (port and config-path setup), tests `:2549-2601`, `:2634-2719`, `:2754-2757` (port assertion only; `engine_builder_resolves_global_registration_namespace_grace` and both starter tests stay)
- Modify: `engine/src/engine/mod.rs:435-443` (fields), `:485-486` (init), `:514-544` (accessors); comments at `engine/src/bin_resolve.rs:13-14`, `engine/src/cli/platform.rs:201-203` and `engine/Cargo.toml:155-156` drop the `workers::external` mention (`which` stays: `bin_resolve.rs:72` uses it)

**Interfaces:**
- Removes: `iii::workers::{external, secure_temp}`, `Worker::is_external_process`, `Engine::{worker_manager_port, set_worker_manager_port, config_path, set_config_path}`. `EngineBuilder::config_path()` stays (`engine/tests/reload_scope_unit.rs:223`).

- [ ] **Step 1: Record the references to remove**

Run: `rg -n "KNOWN_EXTERNAL|resolve_external_module|workers::external|secure_temp|ExternalProcessWorker|is_external_process|worker_manager_port|set_config_path|engine\(\)\.config_path|engine\.config_path" engine`

- [ ] **Step 2: Confirm the starting state**

Expected: hits only in the files listed above.

- [ ] **Step 3: Delete the listed code and files**

`runtime_worker_info_from_registration` keeps its `Option` return; only the guard goes.

- [ ] **Step 4: Verify**

Run: the Step 1 `rg`, then `cargo test --locked -p iii --all-features`
Expected: `rg` prints nothing; `test result: ok`.

- [ ] **Step 5: Commit**

```bash
git add -A engine/Cargo.toml engine/src/workers/external.rs engine/src/workers/secure_temp.rs engine/tests/external_known_sandbox.rs engine/src/lib.rs engine/src/workers/traits.rs engine/src/workers/config.rs engine/src/engine/mod.rs engine/src/bin_resolve.rs engine/src/cli/platform.rs
git commit -m "refactor(engine): stop spawning iii-worker for iii-sandbox"
```

### Task 6: Drop the learn-iii image prefetch (S4)

**Files:**
- Modify: `engine/src/cli/project/mod.rs:314-388` (`LEARN_III_BASE_IMAGE` with its doc, `start_image_prefetch`, `declared_base_images`; `LEARN_III_TEMPLATE` at `:313` stays), `:404-407`, `:418-424`, `:442-452` (uses and the wait loop in `run_learn_iii`), `:1213-1266` (both tests)

**Interfaces:** none produced; `iii project init --learn-iii` no longer runs `iii-worker __pull-images` (spec 7.5).

- [ ] **Step 1: Record the references**

Run: `rg -n "LEARN_III_BASE_IMAGE|start_image_prefetch|declared_base_images|__pull-images" engine/src`

- [ ] **Step 2: Confirm the starting state**

Expected: hits only in `engine/src/cli/project/mod.rs` at the listed lines.

- [ ] **Step 3: Delete the listed lines**

- [ ] **Step 4: Verify**

Run: the Step 1 `rg`; `cargo test --locked -p iii --all-features --bin iii cli::project`; `cargo clippy --locked -p iii --all-targets --all-features 2>&1 | rg -- '--> engine/src/cli/project/mod.rs'`
Expected: both `rg` runs print nothing; the remaining `cli::project` tests pass.

- [ ] **Step 5: Commit**

```bash
git add engine/src/cli/project/mod.rs
git commit -m "refactor(engine): drop the learn-iii image prefetch"
```

### Task 7: Move the `iii-sandbox` manifest, README and skill to the router

**Files:**
- Move (`git mv`): `crates/iii-worker/src/sandbox_daemon/{iii.worker.yaml, README.md, skills/SKILL.md}` to `engine/src/workers/sandbox/`
- Modify: `engine/src/workers/sandbox/iii.worker.yaml` (drop the `config:` block, since adapter settings move to `sandbox-builtin` (P1); `description:` = the Task 3 `register_worker!` text)
- Modify: `engine/src/workers/sandbox/README.md`: `:5-7` becomes the router and binding note; `:94-95` and `:431-549` (S-code anchors) become one link to `https://iii.dev/docs/creating-workers/sandboxes` plus one line each for S301, S302 and S303; `:97-144` (host requirements, sample config, config table) become a "Binding" section (the spec 6.3 YAML; External-mode engines set `sandbox:` in their engine config; adapter settings go under `containers.sandbox-builtin.config_override` with the same keys; known limit (D10): the router re-reads `adapter::info` only after an S301 or `invocation_stopped`, so an adapter swapped for another major between calls is not caught until then); `:415-429` (stale `iii sandbox` CLI) is deleted
- Modify: `engine/src/workers/sandbox/skills/SKILL.md:11, 15-17` (the router forwards to an adapter; prerequisites: a bound adapter, S301 names the `compose::add iii-sandbox` fix, S303 means unmanaged engine; hardware needs belong to `sandbox-builtin`)
- Delete: `crates/iii-worker/tests/sandbox_docs_anchor_stability.rs`. It reads the moved README (`:18`); P0's anchor test covers the iii.dev page.
- Modify: `.github/workflows/release-iii.yml:666-667`, `.github/workflows/alpha-release.yml:448` (`worker_dir: engine/src/workers/sandbox`), `.github/workflows/WORKFLOWS.md:381` (drop the `sandbox_daemon` example)
- Modify: `.github/workflows/_publish-engine-workers.yml:151-162`: the inline Python exits 0 without writing `config.yaml` when `/tmp/iii-workers.json` (the boot listing, `:119`) already holds a worker whose `name` or `id` is `$WORKER`. `iii-sandbox` is now mandatory and not configurable, so today's rewrite is refused, and a refused reload stops the engine (`engine/src/workers/config.rs:1001-1009`) before the collect step (spec 12: keep this job working).

**Interfaces:** the registry package `iii-sandbox` is published from `engine/src/workers/sandbox` (spec 6.5); the needs edges belong to P3.

- [ ] **Step 1: Move the files, then run the validator before repointing the rows**

Run: `python3 .github/scripts/validate_release_worker_matrix.py --repo-root .`

- [ ] **Step 2: Confirm it fails**

Expected: exit 1 with `::error::release-iii.yml: missing publishable engine worker 'engine/src/workers/sandbox'` and `::error::release-iii.yml: crates/iii-worker/src/sandbox_daemon: missing iii.worker.yaml for matrix worker 'iii-sandbox'` (the same for `alpha-release.yml`).

- [ ] **Step 3: Make the edits listed above**

- [ ] **Step 4: Verify**

Run: `python3 .github/scripts/validate_release_worker_matrix.py --repo-root .`; `(cd .github/scripts && python -m pytest -q)` (as `test-scripts.yml:44-47`); `actionlint .github/workflows/_publish-engine-workers.yml`. Then run the reload heredoc once against a real boot listing, from an empty temp dir with `R` set to the repo root: `cargo build -p iii --manifest-path "$R/Cargo.toml"`; `printf 'workers: []\n' > config.yaml`; start `"$R/target/debug/iii" --config config.yaml --no-update-check &`; once it answers, `"$R/target/debug/iii" trigger 'engine::workers::list' --json '{}' > /tmp/iii-workers.json`; `sed -n "/<<'PY'/,/^ *PY\$/p" "$R/.github/workflows/_publish-engine-workers.yml" | sed '1d;$d;s/^ \{10\}//' > reload.py`; `WORKER=iii-sandbox WORKER_DIR="$R/engine/src/workers/sandbox" python3 reload.py`; stop the engine.
Expected: `::notice::release worker matrix validation passed`; pytest passes; actionlint prints nothing; `/tmp/iii-workers.json` holds an `iii-sandbox` row; `reload.py` exits 0 and `config.yaml` still reads `workers: []`.

- [ ] **Step 5: Commit**

```bash
git add -A crates/iii-worker/src/sandbox_daemon crates/iii-worker/tests/sandbox_docs_anchor_stability.rs engine/src/workers/sandbox .github/workflows/release-iii.yml .github/workflows/alpha-release.yml .github/workflows/WORKFLOWS.md .github/workflows/_publish-engine-workers.yml
git commit -m "docs(sandbox): move the iii-sandbox manifest, README and skill to the engine router"
```

### Task 8: Whole-plan verification and PR note

**Files:** none changed.

- [ ] **Step 1: Format and lint**

Run: `cargo fmt --all -- --check`; `cargo clippy --locked -p iii --all-targets --all-features 2>&1 | rg -- '--> engine/src/(workers/(sandbox/|config\.rs|reload\.rs|telemetry/mod\.rs)|engine/mod\.rs|cli/project/mod\.rs)'`
Expected: fmt is clean; `rg` prints nothing (no new warnings in touched files).

- [ ] **Step 2: Full tests for the touched packages** (as `ci.yml:186-187, 363` run them)

Run: `cargo test --locked -p iii --all-features`; `cargo test --locked -p iii-compose`; `cargo test --locked -p iii-worker`
Expected: each ends `test result: ok`. The spec 11 engine checks this slice owns are among them: reserved prefix (`register_function_reserves_sandbox_prefix_in_default`); forwarding with errors and metadata (`forwards_across_namespaces_with_payload_and_metadata_untouched`, `adapter_errors_pass_through_untouched`); S301/S302/S303 (`bound_router_without_adapter_answers_s301`, `s302_when_adapter_major_differs`, `unbound_router_answers_s303_but_info_never_errors`); reload keeps the binding (`reload_keeps_sandbox_binding`, `engine_config_binds_the_mandatory_router_on_build`); `invocation_stopped` on disconnect (`invocation_stopped_on_adapter_disconnect_clears_handshake`).

- [ ] **Step 3: Static checks**

Run: `rg -n '"iii-sandbox"' crates/iii-compose/src/config.rs`; `rg -n "KNOWN_EXTERNAL|start_image_prefetch|__pull-images" engine`; `python3 .github/scripts/validate_release_worker_matrix.py --repo-root .`
Expected: both `rg` runs print nothing; the validator exits 0.

- [ ] **Step 4: PR-ready note** (do not push or open the PR)

- Create the MOT ticket first. Title: `feat(engine): sandbox router owns sandbox::* and forwards to the bound adapter (MOT-<id>)`.
- Body:
  - the 7 commits;
  - the new `sandbox:` engine-config key;
  - `engine.workers.iii-sandbox` now fails with the migration arm;
  - the engine no longer spawns `iii-worker`;
  - `sandbox-isolation-linux` loses its end-to-end step and log upload (Task 4): the engine now refuses the old daemon's `sandbox::*` registrations in `default`; P1's `sandbox-builtin-vm-linux` runs the moved isolation e2e, and P3 deletes the rest of the job;
  - accepted docs break: 0.24.x `docs_url` links (`https://github.com/iii-hq/iii/blob/main/crates/iii-worker/src/sandbox_daemon/README.md#Sxxx`) stop resolving once this merges, because Task 7 moves that README and drops its anchors; the S-codes live at `https://iii.dev/docs/creating-workers/sandboxes` from the 0.25.0 docs rotation (P3's upgrade guide says the same);
  - the Review Focus list.
- Merge order: after P0, and only once `sandbox-builtin` 1.0.0 resolves for the three targets (the P3 Task 7 Step 3 curl) and P2b is approved, so P2a, P2b and P3 merge back to back; no stable tag from main from this merge until 0.25.0, which is cut only after P3.
- Known interim state until P2b merges (same release):
  - Compose writes no `sandbox:` binding yet, so `sandbox::*` on a Compose-managed engine answers S303;
  - the Compose `UnsupportedEngineWorker` text still lists `iii-sandbox` (`crates/iii-compose/src/error.rs:46-50`);
  - `compose::add iii-sandbox` answers "It is injected automatically and must not be declared" (`crates/iii-compose/src/daemon.rs:2181-2185`).
