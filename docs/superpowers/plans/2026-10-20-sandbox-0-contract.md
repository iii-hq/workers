# Sandbox Contract Crate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the `iii-sandbox-contract` crate (function ids, wire types, S-codes, adapter info) to the iii workspace and publish it to crates.io with the next engine patch, with no behaviour change.

**Architecture:** A library crate at `crates/iii-sandbox-contract` holds copies of today's daemon wire types plus the new worker-VM, info and error types, all re-exported at the crate root. Parity tests pin its JSON to today's daemon wire and to iii-sdk's `StreamChannelRef`. A `release-iii.yml` job publishes it through `_rust-cargo.yml`; the crate inherits the workspace version, which the release already bumps.

**Tech Stack:** Rust 2024, serde 1, serde_json 1, schemars 0.8 (draft-07 via `schema_for!`, as the SDK registers today); GitHub Actions; Mintlify MDX docs.

**Spec:** docs/superpowers/specs/2026-10-20-sandbox-worker-design.md, sections 2, 5.1-5.7, 10 (step 0), 11. Shared names: docs/superpowers/plans/2026-10-20-sandbox-shared-interfaces.md

**Repo and base:** iii repo at `a1d08d8b0`, branch `sandbox/p0-contract`. No prerequisites. Downstream: P1 pins `=0.24.7` from crates.io once the next `iii/v*` patch tag publishes it, and P2a uses the crate by path.

## Global Constraints

- Dependencies: `serde`, `serde_json`, `schemars` only; `iii-sdk` is a dev-dependency for one test (5.1).
- The crate version moves in lockstep with the engine. The wire version is separate: `CONTRACT_MAJOR = 1`, `CONTRACT_MINOR = 0`. Only a major bump breaks (5.1).
- Anti-goal: no change to any `sandbox::*` request or response shape, or to the S-code error wire format (2). Shapes are today's daemon shapes, moved verbatim (5.3).
- `exec.exit_code` stays `Option<i32>`; it is `null` on signal death (5.3).
- Own copies of `FsEntry`, `FsMatch`, `FsSedFileResult` (no `iii-shell-proto` dependency) and a wire-compatible `StreamChannelRef`; `FsReadMeta` is not copied (5.1).
- The `exec.workdir` description becomes "`/` when omitted" (5.1).
- Wire stays `ErrorBody{code: "invocation_failed", message: <SandboxErrorWire JSON>}`. Kept: S001-S004, S100-S102, S200, S210-S219, S300, S400. New: S301 AdapterUnavailable (retryable), S302 ContractMismatch, S303 NoAdapterConfigured, S304 OperationUnsupported, S501 SourceRejected, S502 StateBusy, S503 InstanceMismatch, S504 VmNotFound, S505 Unauthorized, S506 PrepareFailed (retryable); S500 reserved with no variant (5.6).
- `AdapterInfo{adapter, adapter_version, contract:{major, minor}}`, no capability fields (5.7). `sandbox::adapter::info` returns `{adapter, namespace, contract, info|null}` (5.2).
- Not in v1 (2): capability negotiation, the JSON Schema bundle, `create.guest_engine_url`, `worker::{wait, list, renew}`, leases, archive or upload worker sources. `worker::{exec, sync, wait, list, renew}` stay reserved (5.2). Cut by D3: no `Family` enum, `Resources`, `WorkerStartRequest.resources` or `WorkerSource::Local.sync`; `OPERATIONS` holds only the 16 ephemeral ids and the six worker-VM suffixes are `WORKER_*` constants (5.1, 5.2).
- Step 0 has no behaviour change (10): no file under `crates/iii-worker` or `engine/` changes.

## Review Focus

1. Consumers' lenient exec inputs (shell line, `cmd`+`args`, `argv`; env as a list or a map; base64 `stdin`) parse exactly as the daemon parses them today (5.3). Pinned by Task 2 `exec_accepts_todays_three_shapes`, `env_accepts_list_and_map`.
2. A signal-killed exec answers `"exit_code": null` with the key present, and `fs::read`/`run` replies omit `body`/`sandbox_id` when absent, as ide and sandbox-code-runner read them today. Pinned by Task 2 `exec_response_keeps_null_exit_code`, `optional_reply_fields_stay_omitted`.
3. Older clients that send extra fields (`sandbox::list {"show_all": true}`, unknown keys on any request) keep working; no moved type gains `deny_unknown_fields`. Pinned by Task 2 `requests_ignore_unknown_fields`.
4. A user who follows the `docs_url` of any code, including the new S301-S506, lands on an anchor that exists. Pinned by Task 1 `every_code_has_a_docs_anchor`.
5. The adapter (built from crates.io) and Compose (path dependency) exchange worker-VM JSON as separately built binaries: kind-tagged `WorkerSource`, snake_case `State` and `LogStream`, `exit_code: null`. Pinned by Task 3 `worker_source_is_kind_tagged`, `worker_status_and_log_record_json`.

---

### Task 1: Crate scaffold and S-codes

**Files:**
- Create: `crates/iii-sandbox-contract/Cargo.toml`, `crates/iii-sandbox-contract/src/lib.rs`, `crates/iii-sandbox-contract/src/errors.rs`
- Create: `crates/iii-sandbox-contract/tests/fixtures/sandbox_error_codes.json` (copy of `crates/iii-worker/tests/fixtures/sandbox_error_codes.json:1-27`, 20 rows)
- Modify: `Cargo.toml:15` (add member `"crates/iii-sandbox-contract"` after `"crates/iii-network"`), `Cargo.lock`
- Modify: `.github/workflows/ci.yml:187`, `Makefile:61`, `Makefile:66` (`-p iii` becomes `-p iii -p iii-sandbox-contract`; CI tests only the packages it names)
- Modify: `docs/next/creating-workers/sandboxes.mdx:137` (append a section); regenerate `docs/next/creating-workers/sandboxes.mdx.skill.md`
- Test: `crates/iii-sandbox-contract/tests/wire_parity.rs`

**Interfaces:**
- Consumes: nothing.
- Produces (registry §2 `errors.rs`, verbatim): `DOCS_BASE`; `SandboxErrorCode` with `as_str(&self) -> &'static str`, `error_type(&self) -> &'static str`, `retryable(&self) -> bool`; `error_payload(code: SandboxErrorCode, message: &str, fix: Option<serde_json::Value>, fix_note: Option<&str>) -> serde_json::Value`; `code_of(message: &str) -> Option<SandboxErrorCode>`; the fixture; one `<a id="Sxxx"></a>` per code in `docs/next/creating-workers/sandboxes.mdx`, which P3 must keep when it rewrites that page.

- [ ] **Step 1: Write the failing test.** Create `Cargo.toml` (below), `src/lib.rs` with `mod errors; pub use errors::*;`, an empty `src/errors.rs`, the workspace member line, and `tests/wire_parity.rs` holding a test-local `ALL` array of the 30 variants guarded by an exhaustive `match`, as `crates/iii-worker/tests/sandbox_error_codes_integration.rs:135-186` does:

```toml
[package]
name = "iii-sandbox-contract"
version.workspace = true   # the root version; bump_manifests.py:118-122 moves it on every release
edition.workspace = true
rust-version = "1.85"
license = "Apache-2.0"
authors.workspace = true
repository.workspace = true
homepage.workspace = true
description = "Wire contract between the iii engine's sandbox router and sandbox adapter workers"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
schemars = "0.8"

[dev-dependencies]
iii-sdk = { workspace = true }   # path-only, so cargo publish strips it
```

```rust
#[test] fn fixture_rows_match_error_payload() // f.docs_base == DOCS_BASE, 30 rows; per row p = error_payload(from_value(row.code), "x", None, None):
    // p["type"] == row.type, p["retryable"] == row.retryable, p["docs_url"] == format!("{DOCS_BASE}{}", row.code)
#[test] fn every_code_has_a_fixture_row() // every ALL entry's as_str() is a fixture `code`
#[test] fn payload_keeps_todays_shape() {
    assert_eq!(error_payload(S211, "parent directory not found: /w", Some(json!({"parents": true})), Some("merge")),
        json!({"type": "filesystem", "code": "S211", "message": "parent directory not found: /w",
               "docs_url": format!("{DOCS_BASE}S211"), "retryable": false, "fix": {"parents": true}, "fix_note": "merge"}));
    let p = error_payload(S301, "m", None, None);
    assert_eq!((&p["type"], &p["retryable"], &p["fix"], &p["fix_note"]), (&json!("transient"), &json!(true), &Value::Null, &Value::Null));
}
#[test] fn code_of_reads_wire_messages() {
    assert_eq!(code_of(&error_payload(S504, "x", None, None).to_string()), Some(S504));
    for m in ["not json", r#"{"code":"S999"}"#, r#"{"message":"x"}"#] { assert_eq!(code_of(m), None); }
}
#[test] fn every_code_has_a_docs_anchor() // page = CARGO_MANIFEST_DIR + "/../../docs/next/creating-workers/sandboxes.mdx";
    // for c in ALL: page.contains(&format!("<a id=\"{}\"></a>", c.as_str()))
```

- [ ] **Step 2: Run it to verify it fails.** `cargo test -p iii-sandbox-contract --test wire_parity` → `error[E0432]: unresolved import` (nothing is defined yet).
- [ ] **Step 3: Implement `errors.rs` and the fixture and docs.**
  - `pub const DOCS_BASE: &str = "https://iii.dev/docs/creating-workers/sandboxes#";`. Variants, derives and signatures per registry §2. `error_type`/`retryable` keep today's arms (`crates/iii-worker/src/sandbox_daemon/errors.rs:71-93`) plus: S301 `transient`/true; S302, S303 `config`/false; S304 `platform`/false; S501-S505 `validation`/false; S506 `transient`/true.
  - `error_payload` returns the seven keys of `errors.rs:263-277`, with `docs_url = DOCS_BASE + code`. `code_of` parses the message as a JSON object and deserializes its string `code`.
  - Fixture: set `docs_base` to `DOCS_BASE` and append 10 rows (S301-S304, S501-S506) with the types above, `"rust_variant": null` and a summary naming the spec 5.6 meaning.
  - Docs: append `## Error codes` to `docs/next/creating-workers/sandboxes.mdx` (the source; a release copies `docs/next/` over the Latest pages, `.github/scripts/pin_docs.py:217-221, 268-277`), one line per fixture row: `- <a id="S001"></a>**S001** (validation): <summary>`. Then run `iii-skill-render --write docs/next/creating-workers/sandboxes.mdx`, as `scripts/generate-cli-docs.sh:64-65` does.
- [ ] **Step 4: Run the tests to verify they pass.** `cargo test -p iii-sandbox-contract --test wire_parity` → `test result: ok. 5 passed`.
- [ ] **Step 5: Commit.** `git add Cargo.toml Cargo.lock Makefile .github/workflows/ci.yml crates/iii-sandbox-contract docs/next/creating-workers/sandboxes.mdx docs/next/creating-workers/sandboxes.mdx.skill.md`; `git commit -m "feat(sandbox): add iii-sandbox-contract crate with S-codes and docs anchors"`

### Task 2: Ephemeral wire types and StreamChannelRef

**Files:**
- Create: `crates/iii-sandbox-contract/src/channel.rs`, `crates/iii-sandbox-contract/src/ephemeral.rs`
- Modify: `crates/iii-sandbox-contract/src/lib.rs`
- Test: `crates/iii-sandbox-contract/tests/stream_channel_ref.rs`, `crates/iii-sandbox-contract/tests/wire_parity.rs`

**Interfaces:**
- Consumes: Task 1 crate.
- Produces (registry §2, verbatim): `StreamChannelRef`, `ChannelDirection`; every type in the registry §2 "Ephemeral types" table; `EnvShape::into_kv_vec(self) -> Result<Vec<String>, String>`; `impl From<FsEntry> for StatResponse`.

- [ ] **Step 1: Write the failing tests.**

```rust
// tests/stream_channel_ref.rs (spec 11)
#[test] fn contract_stream_channel_ref_matches_iii_sdk() // for Read/"read" and Write/"write", channel_id "c", access_key "k":
    // to_value(ours) == to_value(iii_sdk::channels::StreamChannelRef) == {"channel_id":"c","access_key":"k","direction":<s>}; iii-sdk parses ours
// tests/wire_parity.rs
#[test] fn exec_accepts_todays_three_shapes() // {"cmd":"node -v"} -> args []; {"cmd":"node","args":["-v"]};
    // {"argv":["node","-v"],"stdin":"aGk="} -> argv ["node","-v"], cmd "", stdin Some("aGk=") (each with "sandbox_id":"s")
#[test] fn env_accepts_list_and_map() // ["A=1"] -> Ok(["A=1"]); {"B":"2","A":"1"} -> Ok(["A=1","B=2"])
#[test] fn env_map_rejects_bad_names_with_todays_text() {
    assert_eq!(EnvShape::Map(BTreeMap::from([("9bad".into(), "x".into())])).into_kv_vec().unwrap_err(),
        "env contains invalid variable name(s): [\"9bad\"]. Names must match `[A-Za-z_][A-Za-z0-9_]*`.");
}
#[test] fn requests_ignore_unknown_fields() // ListRequest {"show_all":true}, CatalogListRequest {"x":1}, StopRequest {"sandbox_id":"s","extra":1}
#[test] fn request_defaults_match_daemon() // Grep: recursive true, max_matches 10_000, max_line_bytes 4096; Sed: recursive true,
    // regex true, files []; Mkdir mode "0755"; Write mode "0644", content None; Stop wait false
#[test] fn write_content_is_untagged() // "hi" -> Utf8("hi"); {"channel_id":"c","access_key":"k","direction":"read"} -> Stream(_)
#[test] fn exec_response_keeps_null_exit_code() // to_value(ExecResponse { exit_code: None, .. }) has the key "exit_code": null
#[test] fn optional_reply_fields_stay_omitted() // ReadResponse { body: None } has no "body"; RunResponse { sandbox_id: None } no "sandbox_id"
#[test] fn serialized_requests_parse_back() // ExecRequest { argv, env: Map{"A":"1"} }, WriteRequest { content: Stream }, SedRequest { path: "/w" }:
    // to_value(from_value::<T>(to_value(&t))) == to_value(&t), and exec's "env" == {"A":"1"}
```

- [ ] **Step 2: Run them to verify they fail.** `cargo test -p iii-sandbox-contract --tests` → `error[E0432]: unresolved import` for `StreamChannelRef`, `ExecRequest`, ….
- [ ] **Step 3: Implement `channel.rs` and `ephemeral.rs`.** `channel.rs` copies `sdk/packages/rust/iii/src/channels.rs:15-28` (derives per registry §2). `ephemeral.rs` copies each struct with its field docs and serde attributes, `#[schemars(example)]` fn, serde default fns (the two `default_mode` fns become `default_dir_mode` "0755" and `default_file_mode` "0644") and `is_valid_env_name` from the sources in the registry §2 table, without their Elastic License headers (D2), then applies the registry §2 "Changes" paragraph: every request and response derives `Debug, Clone, Serialize, Deserialize, JsonSchema`; `into_kv_vec` returns `Err(<today's format! text>)`; the `workdir` doc ends "`/` when omitted"; the docs at `create.rs:22` and `catalog.rs:93` say "the adapter's `custom_images` config". `lib.rs` adds `mod channel; mod ephemeral;` and their `pub use`.
- [ ] **Step 4: Run the tests to verify they pass.** `cargo test -p iii-sandbox-contract --tests` → `test result: ok` for `stream_channel_ref` (1 passed) and `wire_parity` (14 passed).
- [ ] **Step 5: Commit.** `git add crates/iii-sandbox-contract`; `git commit -m "feat(sandbox): copy the daemon's ephemeral wire types into the contract"`

### Task 3: Adapter info and worker-VM types

**Files:**
- Create: `crates/iii-sandbox-contract/src/info.rs`, `crates/iii-sandbox-contract/src/worker.rs`
- Modify: `crates/iii-sandbox-contract/src/lib.rs`
- Test: `crates/iii-sandbox-contract/tests/wire_parity.rs`

**Interfaces:**
- Consumes: Task 2 `StreamChannelRef`.
- Produces (registry §2, verbatim): `CONTRACT_MAJOR`, `CONTRACT_MINOR` (in `info.rs`), `ContractVersion` with `CURRENT`, `AdapterInfoRequest`, `AdapterInfo`, `RouterInfo`; `WorkerPrepareRequest`, `WorkerPrepareResponse`, `WorkerStartRequest`, `WorkerSource`, `Owner`, `WorkerStartResponse`, `WorkerStatusRequest`, `WorkerStatus`, `State`, `WorkerLogsRequest`, `WorkerLogsResponse`, `LogRecord`, `LogStream`, `WorkerStopRequest`, `WorkerDestroyRequest`, `WorkerDestroyResponse`.

- [ ] **Step 1: Write the failing tests.**

```rust
#[test] fn router_info_unbound_json() {
    assert_eq!(to_value(RouterInfo { adapter: None, namespace: None, contract: ContractVersion::CURRENT, info: None }).unwrap(),
        json!({"adapter": null, "namespace": null, "contract": {"major": 1, "minor": 0}, "info": null}));
}
#[test] fn adapter_info_json() // {"adapter":"sandbox-builtin","adapter_version":"1.0.0","contract":{"major":1,"minor":0}}; AdapterInfoRequest from {} is Ok
#[test] fn worker_source_is_kind_tagged() // Bundle -> {"kind":"bundle","package":"my-bundle","host_path":"/c"};
    // Local { run_override: None } -> {"kind":"local","host_path":"/p","run_override":null}
#[test] fn worker_status_and_log_record_json() // WorkerStatus { state: Exited, exit_code: None, signal: Some(9), .. } -> "state":"exited",
    // "exit_code":null,"signal":9; LogRecord { Stderr, "x", 1, 7 } -> {"stream":"stderr","line":"x","ts_ms":1,"cursor":7}
#[test] fn worker_start_request_round_trips() // a full literal (owner {"kind":"compose","id":"my-proj","container":"my-bundle"},
    // env {"A":"1"}, no "resources" key) survives to_value(from_value::<WorkerStartRequest>(v)) == v
```

- [ ] **Step 2: Run them to verify they fail.** `cargo test -p iii-sandbox-contract --test wire_parity` → `error[E0432]: unresolved import` for `RouterInfo`, `WorkerSource`, ….
- [ ] **Step 3: Implement `info.rs` and `worker.rs`** with the fields, derives and serde attributes of registry §2 (spec 5.4, 5.5, 5.7); `CURRENT = ContractVersion { major: CONTRACT_MAJOR, minor: CONTRACT_MINOR }`. No `ArchiveRef`, `Effective`, caps, lease or `Resources` types and no `Local.sync` (D3). `lib.rs` adds both modules and their `pub use`.
- [ ] **Step 4: Run the tests to verify they pass.** `cargo test -p iii-sandbox-contract --test wire_parity` → `test result: ok. 19 passed`.
- [ ] **Step 5: Commit.** `git add crates/iii-sandbox-contract`; `git commit -m "feat(sandbox): add adapter info and worker-VM types to the contract"`

### Task 4: Operation table and adapter ids

**Files:**
- Create: `crates/iii-sandbox-contract/src/operations.rs` (with its `#[cfg(test)] mod tests`)
- Modify: `crates/iii-sandbox-contract/src/lib.rs`

**Interfaces:**
- Consumes: Task 2 types.
- Produces (registry §2, verbatim): `PUBLIC_PREFIX`, `DEFAULT_ADAPTER`, `ADAPTER_INFO_SUFFIX`, `ROUTER_INFO_ID`, `adapter_function_id(adapter: &str, id: &str) -> String`, `is_valid_adapter_name(name: &str) -> bool`, `Operation` with `public_id(&self) -> String`, `OPERATIONS: &[Operation]` (16 rows), `WORKER_PREPARE`, `WORKER_START`, `WORKER_STATUS`, `WORKER_LOGS`, `WORKER_STOP`, `WORKER_DESTROY`. P2a registers every row; P1 registers the rows and `WORKER_*` under its prefix; P2b calls `WORKER_*`.

- [ ] **Step 1: Write the failing tests.**

```rust
#[test] fn operations_follow_daemon_order() {
    let s: Vec<_> = OPERATIONS.iter().map(|o| o.suffix).collect();
    assert_eq!(s, ["create", "exec", "stop", "list", "catalog::list", "run", "fs::ls", "fs::stat", "fs::mkdir", "fs::rm",
        "fs::chmod", "fs::mv", "fs::grep", "fs::sed", "fs::write", "fs::read"]);
    assert_eq!([WORKER_PREPARE, WORKER_START, WORKER_STATUS, WORKER_LOGS, WORKER_STOP, WORKER_DESTROY],
        ["worker::prepare", "worker::start", "worker::status", "worker::logs", "worker::stop", "worker::destroy"]);
}
#[test] fn ids() {
    assert_eq!(OPERATIONS[0].public_id(), "sandbox::create");
    assert_eq!(ROUTER_INFO_ID, format!("{PUBLIC_PREFIX}{ADAPTER_INFO_SUFFIX}"));
    assert_eq!(adapter_function_id("sandbox-builtin", "sandbox::fs::ls"), "sandbox-builtin::fs::ls");
    assert_eq!(adapter_function_id("sandbox-builtin", "fs::ls"), "sandbox-builtin::fs::ls");
    assert_eq!(adapter_function_id("fake", ADAPTER_INFO_SUFFIX), "fake::adapter::info");
    assert_eq!(DEFAULT_ADAPTER, "sandbox-builtin");
}
#[test] fn adapter_names() {
    for ok in ["sandbox-builtin", "fake", "sandbox-mock", "a1"] { assert!(is_valid_adapter_name(ok)); }
    for bad in ["", "sandbox", "Sandbox", "a_b", "a::b", "a b"] { assert!(!is_valid_adapter_name(bad)); }
}
#[test] fn schemas_and_descriptions() {
    // every op: (op.request_schema)() and (op.response_schema)() are objects with a "$schema" key;
    // exec request schema "properties" keys == {sandbox_id, cmd, args, argv, stdin, env, timeout_ms, workdir};
    // its workdir description contains "`/` when omitted";
    // no op.description and no schema string contains "engine.workers.iii-sandbox"
}
```

- [ ] **Step 2: Run them to verify they fail.** `cargo test -p iii-sandbox-contract --lib operations` → `error[E0425]: cannot find value OPERATIONS`.
- [ ] **Step 3: Implement `operations.rs`.** Schemas through one private `fn schema<T: JsonSchema>() -> Value { serde_json::to_value(schemars::schema_for!(T)).unwrap() }` (as `engine/function-macros/src/lib.rs:350-360`). Request/response pairs per registry §3 table. Ephemeral descriptions copy today's `.description(...)` text (`sandbox_daemon/mod.rs:255-261, 289-296, 325-328, 364, 392-395`, `run.rs:377-383`, each `fs/*.rs` `register`), with `engine.workers.iii-sandbox.custom_images` replaced by "the adapter's `custom_images` config". `adapter_function_id` strips one leading `PUBLIC_PREFIX`.
- [ ] **Step 4: Run the tests to verify they pass.** `cargo test -p iii-sandbox-contract --lib` → `test result: ok. 4 passed`.
- [ ] **Step 5: Commit.** `git add crates/iii-sandbox-contract`; `git commit -m "feat(sandbox): add the contract's operation table and adapter ids"`

### Task 5: crates.io publish job

**Files:**
- Modify: `.github/workflows/release-iii.yml:574` (new job `sandbox-contract-rust` after `helpers-rust`, :560-573), `:855`, `:885`, `:918`, `:928` (`notify-complete` needs, env, Slack line, status loop)

**Interfaces:**
- Consumes: Task 1 `Cargo.toml` (`version.workspace = true`).
- Produces: `iii-sandbox-contract` on crates.io at each `iii/v*` stable or rc tag's version, never on `iii-alpha/v*` tags (D4: `alpha-release.yml` gets no job); the next patch tag publishes `0.24.7`, which P1 pins as `=0.24.7` (registry §1).

- [ ] **Step 1: Write the failing check.** `rg -n 'package_path: crates/iii-sandbox-contract' .github/workflows/release-iii.yml` prints nothing.
- [ ] **Step 2: Run it to verify it fails.** The command above exits 1.
- [ ] **Step 3: Add the job** as a copy of `helpers-rust` (`needs: [setup, create-iii-release]`, `uses: ./.github/workflows/_rust-cargo.yml`, `package_path: crates/iii-sandbox-contract`, `slack_label: Sandbox Contract Rust (cargo)`); in `notify-complete` add `sandbox-contract-rust` to `needs`, `SANDBOX_CONTRACT_RESULT: ${{ needs.sandbox-contract-rust.result }}`, the line `LINES="$LINES$(icon "$SANDBOX_CONTRACT_RESULT") Sandbox Contract Rust\n"`, and `"$SANDBOX_CONTRACT_RESULT"` in the `for r` list. No `create-tag.yml` or `bump_manifests.py` edit: the crate inherits the root version, which `bump_manifests.py:118-122` already bumps and `create-tag.yml:323` validates.
- [ ] **Step 4: Run the checks to verify they pass.**
  - `rg -n 'package_path: crates/iii-sandbox-contract' .github/workflows/release-iii.yml` prints one line. As `test-scripts.yml` runs on a `release-iii.yml` change: `python3 .github/scripts/validate_release_worker_matrix.py --repo-root .` exits 0 and `(cd .github/scripts && python -m pytest -q)` passes.
  - `cargo publish -p iii-sandbox-contract --dry-run --locked --allow-dirty` ends with `warning: aborting upload due to dry run` (packaging strips the path-only dev-dependency and finds `description` and `license`).
  - `bash .github/scripts/validate_rust_release_version.sh Cargo.toml iii/v0.24.6 iii-sandbox-contract` prints `validated iii-sandbox-contract v0.24.6 against iii/v0.24.6` (the root `Cargo.toml:25` version).
- [ ] **Step 5: Commit.** `git add .github/workflows/release-iii.yml`; `git commit -m "ci(release): publish iii-sandbox-contract to crates.io"`

### Task 6: Whole-plan verification and PR note

**Files:** none.

**Interfaces:**
- Consumes: Tasks 1-5.
- Produces: a branch ready for review.

- [ ] **Step 1: Run the full checks.**
  - `cargo fmt --all -- --check` → no output.
  - `cargo clippy --locked -p iii-sandbox-contract --all-targets -- -D warnings` → `Finished`, no warnings.
  - `cargo test --locked -p iii-sandbox-contract` → `ok` for the lib (4), `stream_channel_ref` (1), `wire_parity` (19) and doctests.
  - `cargo metadata --locked --format-version 1 > /dev/null` → exit 0 (`Cargo.lock` is consistent, as `create-tag.yml` checks).
  - Spec 11 checks owned here: "Error JSON matches `fixtures/sandbox_error_codes.json`" (`wire_parity::fixture_rows_match_error_payload`) and "The contract's `StreamChannelRef` serializes to the same JSON as iii-sdk's" (`stream_channel_ref`), both green above.
  - `git diff --stat a1d08d8b0 -- crates/iii-worker engine` → empty (spec 10, step 0: no behaviour change).
- [ ] **Step 2: PR-ready note.** Create the MOT ticket first. Title: `feat(sandbox): iii-sandbox-contract crate (MOT-<n>)`. Body: the new crate (spec 5) and its publish job; no runtime change; first crates.io release is the next engine patch (`0.24.7`), which unblocks P1; the license (Apache-2.0) needs the owner's confirmation before the first publish, and `CARGO_REGISTRY_TOKEN` needs crates.io's `publish-new` scope for this crate name. No AI co-author or "Generated with" line.
