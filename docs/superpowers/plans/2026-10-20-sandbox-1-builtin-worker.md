# sandbox-builtin Adapter Worker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the libkrun VM substrate out of iii's `iii-worker` into a new workers-repo worker, `sandbox-builtin`, that serves `sandbox-builtin::*` (the ephemeral family, `adapter::info` and the worker-VM family) over the bus, and release it to the registry as 1.0.0.

**Architecture:** The `iii-worker` crate and its six inner crates are copied whole into `sandbox-builtin/`, the legacy CLI is deleted, and the daemon becomes the root command of one executable that also answers the hidden `__vm-boot`. It keeps the daemon's own request, response and error types and adds local copies of the contract's worker-VM types, `AdapterInfo` and S501-S506, pinned by the contract's two JSON fixtures (spec Q5); the new `worker::*` operations wrap today's `bundle_vm_command`/`local_vm_command`. Nothing consumes the worker in this step: a 0.24.x engine has no router.

**Tech Stack:** Rust 1.97.1 (edition 2024), iii-sdk/iii-helpers `=0.24.6`, msb_krun `=0.1.16`, libkrunfw 5.2.1, tokio, clap, serde, sha2; the Python release train (pytest); GitHub Actions; bash, jq and `iii trigger`.

**Spec:** docs/superpowers/specs/2026-10-20-sandbox-worker-design.md, sections 2, 5.4, 5.5, 5.6 (S501-S506), 5.7 (`AdapterInfo` values), 5.8, 7.6, 7.8, 8.1-8.7, 10 (step 1), 11 (error fixture, VM checks), 12. Shared names: docs/superpowers/plans/2026-10-20-sandbox-shared-interfaces.md

**Repo and base:** workers repo, branch `sandbox/p1-builtin` from `origin/main` (`8773952be` plus one harness-only commit; workers citations are at `8773952be`). `iii:` marks iii paths at `a1d08d8b0`. Files that Task 1 copies keep their iii line numbers, so `sandbox-builtin/src/x.rs:N` means `iii:crates/iii-worker/src/x.rs:N`. Prerequisites: none; it does not depend on the iii repo (spec Q5). Tasks 3 and 4 copy P0's two fixtures from `iii:crates/iii-sandbox-contract/tests/fixtures/` (P0's branch until it merges).

## Global Constraints

- Folder, manifest name, Cargo package and binary are all `sandbox-builtin`. The function prefix is `sandbox-builtin::` (8.1).
- `Cargo.toml`: `1.0.0 (major = CONTRACT_MAJOR, S2); [workspace] members=["crates/*"]; iii-sdk/iii-helpers "=0.24.6"; msb_krun "=0.1.16"; release profile from iii Cargo.toml:62-66` (`strip = true`, `lto = true`, `codegen-units = 1`, `opt-level = 2`).
- `Cargo.lock` is seeded from the iii lock, which keeps msb_krun_devices 0.1.16 (0.1.39 breaks musl).
- Firmware is the libkrunfw blobs as-is (5.2.1, ABI 5, kernel 6.12.68), committed without LFS. `.gitattributes` gets `sandbox-builtin/firmware/** binary`.
- Inner crates keep their names under `crates/` with `publish = false`, so the guest ABI is unchanged.
- No `iii-sandbox-contract` dependency (spec 5.1, Q5): the adapter keeps its own types, named as in registry §2, and round-trips the contract's `sandbox_error_codes.json` and `wire_examples.json`.
- There is one executable. The root command serves the adapter, and the hidden `__vm-boot` re-executes `current_exe()`.
- Never change a `sandbox::*` request or response shape, or the S-code wire format. The wire stays `ErrorBody{code: "invocation_failed", message: <SandboxErrorWire JSON>}`.
- The 5.8 guarantees are unchanged: exec is argv-only; the timeout is clamped and reported as S200; the whole process group is killed on timeout; 1 MiB cap per stream; ENOENT → 127, EACCES → 126; uid 0; the request's `workdir`, else `/`.
- Config is read once at startup through `configuration::get` and fails closed. A change applies on the next adapter restart. `--config` and `III_ENGINE_URL` are dropped.
- The guest env goes to `__vm-boot` as today (`--env`, then the kernel cmdline). The `localhost` → gateway rewrite is kept.
- Guards: `III_SANDBOX_WORKER_TOKEN` must match `auth` on every `worker::*` call, else S505. `host_path` must lie under a root in `III_SANDBOX_HOST_ROOTS`, else S501 naming the roots (D11).
- Worker VMs take resources from their manifest within today's caps (`parse_bundle_resources`, `parse_manifest_resources`), and local sources always sync, as today. There are no `worker_vm` config keys and no request `resources` or `sync` (D3).
- Handles: `sandbox_id` stays a UUID string. `vm_id` is opaque and stable per `state_key`. `instance` increments on every boot.
- `worker::{exec, sync, wait, list, renew}` are reserved and not registered. `prepare` is long-running (Compose gives it 20 minutes); `stop` is idempotent and `grace_ms: 0` kills; `destroy` is refused while the VM is running.
- Release entry: `kind: rust-binary` on Rust 1.97.1, `publish: true`, first release 1.0.0. Targets are `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, with `windows_exception: "microVMs need KVM or Hypervisor.framework"`, `features: [embed-init, embed-libkrunfw]`, `prebuild: [scripts/build-init.sh]` and `macos_entitlements: entitlements.plist`.
- The macOS binary is ad-hoc signed at build time with `com.apple.security.hypervisor` and `com.apple.security.cs.disable-library-validation`.
- `sandbox-builtin-vm-linux` is never `continue-on-error`. Making it a required check that gates every registry release is a branch-protection step the plan index lists (D5).
- Commits use conventional commits, and the PR title carries the MOT ticket. Never add an AI co-author or a "Generated with" line.

## Review Focus

1. Compose passes a `host_path` under a root that is itself a symlink, such as macOS `/tmp` → `/private/tmp` or `/var/folders`. Both sides must be canonicalized, and the path is accepted.
2. The adapter restarts (a config change, a crash) and Compose starts the same `state_key` again. The new boot gets an `instance` above every earlier one, so a stale `stop{instance: 1}` gets S503 and never kills the new VM.
3. The configuration service is unreachable at startup. The adapter must exit non-zero and never serve with defaults. A missing entry (no stored value) still means serde defaults.
4. `worker::logs{follow: true}` is called on an instance that already exited. It drains the file, closes the channel and returns, and must not wait forever.
5. A VMM ignores SIGTERM. `stop{grace_ms}` must SIGKILL after `grace_ms` and report `state: Stopped` with `signal: 9`.

Pinned tests:
- 1 → Task 6: `host_roots_accept_a_path_under_a_symlinked_root`
- 2 → Task 7: `instance_survives_an_adapter_restart`
- 3 → Task 5: `config_fetch_failure_exits_2` in `tests/sandbox_daemon_exit_integration.rs`
- 4 → Task 8: `follow_on_an_exited_instance_drains_and_returns`
- 5 → Task 7: `stop_escalates_to_sigkill_after_grace`

---

### Task 1: Import the VM substrate and cut the legacy CLI

**Files:**
- Create by copy: `sandbox-builtin/{build.rs, src/, tests/, examples/, vendor/, images/}` from `iii:crates/iii-worker/`; `sandbox-builtin/crates/{iii-init, iii-network, iii-filesystem, iii-supervisor, iii-shell-client, iii-shell-proto}/` from `iii:crates/`; `sandbox-builtin/firmware/libkrunfw-*` from `iii:engine/firmware/`; `sandbox-builtin/.cargo/config.toml` from `iii:.cargo/config.toml`; `sandbox-builtin/tests/vm_platform_e2e.rs` from `iii:engine/tests/vm_platform_e2e.rs`; `sandbox-builtin/Cargo.lock` from `iii:Cargo.lock`
- Create: `sandbox-builtin/Cargo.toml`
- Not copied: `iii:scripts/{build-worker-musl.sh, worker-musl.md}` (D8: no musl in v1).
- Modify: `.gitattributes:3` (append the firmware rule); the four inner `crates/*/Cargo.toml` that lack `publish = false` (the two `iii-shell-*` crates have it)
- Modify: `sandbox-builtin/src/{main.rs, lib.rs, cli/mod.rs, cli/app.rs:13-586}`; `src/cli/worker_manager/libkrun.rs:104-135`; `src/cli/local_worker.rs:848-1584`; `build.rs:24-26` (path only); `tests/musl_firmware.rs:10-12`; `tests/vm_integration.rs:354,413`; `tests/sandbox_daemon_exit_integration.rs:36,103,267`; `tests/vm_platform_e2e.rs:98`; `tests/{bundle_vm_command.rs, local_vm_command.rs}` (drop `config_dir` from the `VmOverride` literals, and the config-mount asserts)
- Delete in the copy: `src/core/`; `src/cli/{gen_docs, init, builtin_defaults, bundle_prepare, sandbox, host_shim, source_watcher, sync, lockfile, spinner, status, stderr_sink}.rs`, and every further module the dead-code sweep in Step 3 reports. Tests `binary_worker_integration`, `common_fixtures_integration`, `config_*_integration` (6 files), `local_add_*_integration` (2), `local_worker_integration`, `sync_drift_adversarial`, `worker_init_e2e`, `worker_integration`, `worker_list_discovery_integration`, `watch_source_exits_when_its_declared_engine_dies` (`sandbox_daemon_exit_integration.rs:351-398`), `tests/README.md`, `tests/fixtures/templates/`, and every `bundle_worker_integration` case except the manifest and resource-cap ones (spec 8.7)

**Interfaces:**
- Consumes: nothing from other plans.
- Produces: lib `sandbox_builtin`; `sandbox_builtin::cli::local_worker::{bundle_vm_command(worker_name: &str, install_dir: &Path, over: VmOverride<'_>) -> Result<VmCommand, String>, local_vm_command(worker_name: &str, worker_dir: &Path, run_override: Option<&str>, over: VmOverride<'_>) -> Result<VmCommand, String>}`; `VmOverride { state_dir: PathBuf, engine_url: &'a str, extra_env: HashMap<String, String> }` (`config_dir` and `GUEST_CONFIG_DIR` are deleted: their only caller was `__bundle-prepare`); `async fn start_worker_impl(worker_name: &str, worker_path: &str, is_bundle: bool, over: VmOverride<'_>, run_override: Option<&str>) -> Result<VmCommand, String>`. The `VmStart` enum and the `port` and `Option<VmOverride>` paths (`run_dev`, `kill_stale_worker`, `~/.iii/managed`, `config_file` env) are gone. Commands left: `sandbox-daemon` (unchanged until Task 5) and hidden `__vm-boot`.

- [ ] **Step 1: Write the failing test.** Copy the tree (Files) and add to the tests module of `src/cli/worker_manager/libkrun.rs`:
```rust
#[test]
fn vm_boot_program_is_the_running_binary() {
    assert_eq!(vm_boot_program().unwrap(), std::env::current_exe().unwrap());
}
```
- [ ] **Step 2: Run it to verify it fails.** `cd sandbox-builtin && cargo test --lib vm_boot_program_is_the_running_binary`. Expected: it fails, since `sandbox-builtin/Cargo.toml` does not exist yet.
- [ ] **Step 3: Implement.**
  - Write `Cargo.toml` with the Global Constraints values: `[[bin]] name = "sandbox-builtin" path = "src/main.rs"`, `license = "Elastic-2.0"`, `publish = false`, and features `embed-libkrunfw = []`, `embed-init = ["iii-filesystem/embed-init"]`, `integration-vm = []`, `integration-oci = []`. Dependencies are iii-worker's list minus `iii-clap-docs` and `scaffolder-core` (and any crate whose last user is deleted), inner crates by path.
  - Rename the lib: `rg -l 'iii_worker::' | xargs sed -i '' 's/iii_worker::/sandbox_builtin::/g'`. Replace `CARGO_BIN_EXE_iii-worker` and `CARGO_BIN_EXE_iii` with `CARGO_BIN_EXE_sandbox-builtin`, and the `ps` needle at `sandbox_daemon_exit_integration.rs:36` with `sandbox-builtin`. `Cli` is named `sandbox-builtin`. Point `build.rs` and `musl_firmware.rs` at `firmware/`.
  - `vm_boot_program()` returns `std::env::current_exe()`, mapping the error to today's text "cannot locate the running binary: {e}".
  - Each former `eprintln!` plus `VmStart::Exit(1)` in `start_worker_impl` becomes `Err(<the same text>)`.
  - Find the dead code: make every `cli` module `pub(crate)`, delete everything `cargo clippy --workspace --all-targets -- -D warnings` reports dead, then restore `pub` only for items a kept integration test imports.
  - Run `cargo build` once without `--locked` to trim the copied lock. Fix new 1.97.1 clippy lints in place.
- [ ] **Step 4: Run the tests to verify they pass.**
  - `cd sandbox-builtin && cargo fmt --all -- --check && cargo clippy --locked --workspace --all-targets -- -D warnings && cargo test --locked --workspace`: all pass.
  - `grep -A1 'name = "msb_krun_devices"' Cargo.lock` prints `version = "0.1.16"`.
  - `rg -n 'scaffolder_core|iii_clap_docs|\.\./engine/' --glob '!Cargo.lock' .` prints nothing.
  - `./target/debug/sandbox-builtin sandbox-daemon --help` succeeds, and `./target/debug/sandbox-builtin sandbox list` fails with `unrecognized subcommand` (both kept commands are hidden).
- [ ] **Step 5: Commit.** `git add .gitattributes sandbox-builtin && git commit -m "feat(sandbox-builtin): import the VM substrate from iii and drop the legacy worker CLI"`

### Task 2: Embedded firmware and init fail closed

**Files:**
- Create: `sandbox-builtin/build/firmware_check.rs`, `sandbox-builtin/firmware/{README.md, SHA256SUMS}`, `sandbox-builtin/scripts/build-init.sh` (mode 755), `sandbox-builtin/tests/firmware_check.rs`
- Modify: `sandbox-builtin/build.rs:13-37`, `Cargo.toml` (`[build-dependencies] sha2 = "0.10"`), `src/cli/firmware/download.rs:24-365`
- Test: `tests/firmware_check.rs`, `src/cli/firmware/download.rs` tests

**Interfaces:**
- Produces: `pub fn check_firmware(file_name: &str, bytes: &[u8], sha256sums: &str) -> Result<(), String>`, loaded by `build.rs` and the test through `include!("build/firmware_check.rs")`. It accepts an ELF header (`\x7fELF`) for `.so` and a 64-bit Mach-O header (`cf fa ed fe`) for `.dylib`, and the file's sha256 must equal its `SHA256SUMS` line. `scripts/build-init.sh` builds `crates/iii-init` for `<arch of $BUILD_TARGET>-unknown-linux-musl` into `sandbox-builtin/target/<musl triple>/release/iii-init`, the path `crates/iii-filesystem/build.rs:20-26` reads. `ensure_libkrunfw()` and `ensure_init_binary()` keep their signatures; the download fallbacks are gone.

- [ ] **Step 1: Write the failing test.**
```rust
// tests/firmware_check.rs
include!("../build/firmware_check.rs");
#[test] fn firmware_check_rejects_an_lfs_pointer_and_a_wrong_sum() {
    let sums = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/firmware/SHA256SUMS")).unwrap();
    let real = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/firmware/libkrunfw-linux-x86_64.so")).unwrap();
    assert!(check_firmware("libkrunfw-linux-x86_64.so", &real, &sums).is_ok());
    let lfs = b"version https://git-lfs.github.com/spec/v1\n";
    assert!(check_firmware("libkrunfw-linux-x86_64.so", lfs, &sums).unwrap_err().contains("not an ELF"));
    let mut tampered = real.clone(); *tampered.last_mut().unwrap() ^= 1;
    assert!(check_firmware("libkrunfw-linux-x86_64.so", &tampered, &sums).unwrap_err().contains("SHA256SUMS"));
}
```
  Replace the two tests at `download.rs:341-364` (the first fails once the firmware is embedded) with these, under `TEST_HOME_LOCK`, with `HOME` set to an empty tempdir and `III_LIBKRUNFW_PATH` and `III_INIT_PATH` unset:
  - `ensure_libkrunfw_fails_closed_without_an_embed` (`#[cfg(not(has_libkrunfw))]`): when `resolve_libkrunfw_dir()` finds nothing, the error contains "not embedded".
  - `ensure_init_binary_fails_closed_without_an_embed` (`#[cfg(not(feature = "embed-init"))]`, because `has_init_binary` is a cfg of `iii-filesystem` only): the error contains "not embedded".
- [ ] **Step 2: Run it to verify it fails.** `cargo test --locked --test firmware_check && cargo test --locked --lib fails_closed`. Expected: `build/firmware_check.rs` and `SHA256SUMS` not found, then both `ensure_*` tests fail on the download attempt.
- [ ] **Step 3: Implement.**
  - Write `SHA256SUMS` with `cd firmware && shasum -a 256 libkrunfw-* > SHA256SUMS`.
  - Under `embed-libkrunfw`, `build.rs` panics with `check_firmware`'s error. Without the feature it keeps the 1-byte placeholder.
  - Delete `download_libkrunfw` and `download_init_binary` and their calls. The exact error texts:
    - `libkrunfw is not embedded in this build and was not found locally; rebuild with --features embed-libkrunfw`
    - `iii-init is not embedded in this build and was not found locally; run scripts/build-init.sh and rebuild with --features embed-init`
  - `build-init.sh`: `set -euo pipefail`, `cd "$(dirname "$0")/.."` (so `.cargo/config.toml`'s `rust-lld` applies), `rustup target add`, `cargo build --locked -p iii-init --release --target "$triple"`, then `test -s` the output.
  - `firmware/README.md`: version 5.2.1, ABI 5, kernel 6.12.68, the rebuild recipe, a "never LFS" note and how to refresh `SHA256SUMS`.
- [ ] **Step 4: Run the tests to verify they pass.**
  - `cargo test --locked --test firmware_check && cargo test --locked --lib fails_closed`: pass.
  - On Linux x86_64: `BUILD_TARGET=x86_64-unknown-linux-gnu scripts/build-init.sh && cargo build --locked --features embed-init,embed-libkrunfw`: succeeds.
  - On macOS: `BUILD_TARGET=aarch64-apple-darwin scripts/build-init.sh`: succeeds.
- [ ] **Step 5: Commit.** `git add sandbox-builtin/build sandbox-builtin/build.rs sandbox-builtin/Cargo.toml sandbox-builtin/firmware sandbox-builtin/scripts/build-init.sh sandbox-builtin/src/cli/firmware/download.rs sandbox-builtin/tests/firmware_check.rs && git commit -m "feat(sandbox-builtin): embed checked firmware and init and fail closed without them"`

### Task 3: S501-S506 and the iii.dev docs base

**Files:**
- Modify: `sandbox-builtin/src/sandbox_daemon/errors.rs:18-19` (`DOCS_BASE`), `:22-95` (`SandboxErrorCode` gains S501-S506), `:97-238` (variants, `code()`), `:293-367` (`fix_hint`), `tests/sandbox_error_codes_integration.rs:51-84, 135-186`
- Replace: `tests/fixtures/sandbox_error_codes.json` with P0's copy (`iii:crates/iii-sandbox-contract/tests/fixtures/sandbox_error_codes.json`)
- Delete: `tests/sandbox_docs_anchor_stability.rs` (it reads the README that P2a moves to the router; P0's anchor test covers the iii.dev page)

**Interfaces:**
- Consumes (P0): the fixture, copied.
- Produces: `DOCS_BASE = "https://iii.dev/docs/creating-workers/sandboxes#"` (registry §2); `SandboxErrorCode::{S501, S502, S503, S504, S505, S506}` with the `type` and retryable flags of the registry §2 table. New `SandboxError` variants, with the Display text in quotes:
  - `SourceRejected(String)` → S501 "worker source rejected: {0}"
  - `StateBusy(String)` → S502 "VM state {0} is busy: its VM is running"
  - `InstanceMismatch { vm_id: String, expected: u64, found: u64 }` → S503 "VM {vm_id} is on instance {found}, not {expected}"
  - `VmNotFound(String)` → S504 "no VM {0} on this adapter"
  - `Unauthorized` → S505 "worker::* needs the project token this adapter was started with"
  - `PrepareFailed(String)` → S506 "worker prepare failed: {0}"
- `to_payload()` keeps today's shape (`errors.rs:263-277`); the `RunStepFailed` fix context is unchanged.

- [ ] **Step 1: Write the failing test.**
  - Copy P0's fixture over the local one. It has `docs_base` on iii.dev and rows for S301-S304 (router codes, `"rust_variant": null`) and S501-S506.
  - In `sandbox_error_codes_integration.rs`, `representative()` gains `"S501" => SandboxError::SourceRejected("x".into())` through `"S506" => SandboxError::PrepareFailed("x".into())`, and `ALL_VARIANTS`/`enforce_exhaustive` (:135-186) gain the six codes.
  - Add:
```rust
#[test] fn docs_url_uses_the_iii_dev_docs_base() {
    let p = SandboxError::Unauthorized.to_payload();
    assert_eq!(p["docs_url"], "https://iii.dev/docs/creating-workers/sandboxes#S505");
    assert_eq!((p["type"].as_str(), p["retryable"].as_bool()), (Some("validation"), Some(false)));
}
```
- [ ] **Step 2: Run it to verify it fails.** `cargo test --locked --test sandbox_error_codes_integration`. Expected: `no variant named SourceRejected`, and the fixture rows fail on `docs_url`.
- [ ] **Step 3: Implement.** Set `DOCS_BASE`, add the six codes and the variants above. Fix-note texts:
  - `ImageNotInCatalog`: "set `image` to a listed value or add it to the adapter's custom_images config"
  - S504: "the adapter restarted; start the VM again"
  - S505: "call worker::* through Compose, which passes the token"
  - S506: "transient: retry after a short delay"
  - S501-S503: none
- [ ] **Step 4: Run the tests to verify they pass.** `cargo test --locked --test sandbox_error_codes_integration && cargo test --locked --lib sandbox_daemon::errors`: pass, including `sandbox_error_wire_display_matches_to_payload_json`.
- [ ] **Step 5: Commit.** `git add -A sandbox-builtin/src/sandbox_daemon/errors.rs sandbox-builtin/tests && git commit -m "feat(sandbox-builtin): add S501-S506 and point docs_url at iii.dev"`

### Task 4: Local copies of the contract's info and worker-VM types

**Files:**
- Create: `sandbox-builtin/src/wire.rs` (`pub mod wire;` in `src/lib.rs`), `sandbox-builtin/tests/wire_examples.rs`
- Create: `sandbox-builtin/tests/fixtures/wire_examples.json`, a copy of P0's (`iii:crates/iii-sandbox-contract/tests/fixtures/wire_examples.json`)

**Interfaces:**
- Consumes (P0): the fixture, copied.
- Produces, in `sandbox_builtin::wire`, with the names, fields and serde attributes of registry §2 (copies, not a dependency, spec Q5): `CONTRACT_MAJOR`, `CONTRACT_MINOR`, `ContractVersion` (`CURRENT`), `AdapterInfoRequest`, `AdapterInfo`, `WorkerPrepareRequest`, `WorkerPrepareResponse`, `WorkerStartRequest`, `WorkerSource`, `Owner`, `WorkerStartResponse`, `WorkerStatusRequest`, `WorkerStatus`, `State`, `WorkerLogsRequest`, `WorkerLogsResponse`, `LogRecord`, `LogStream`, `WorkerStopRequest`, `WorkerDestroyRequest`, `WorkerDestroyResponse`, and the six `WORKER_*` suffix constants. `WorkerLogsRequest.channel` is `iii_sdk::channels::StreamChannelRef`, whose JSON P0 pins to the contract's. `RouterInfo` is not copied: only the router answers it.

- [ ] **Step 1: Write the failing test.**
```rust
// tests/wire_examples.rs
#[test] fn wire_examples_round_trip() {
    // fixture {"<TypeName>": <example>}: for each type in Produces,
    // assert_eq!(to_value(from_value::<T>(example.clone()).unwrap()).unwrap(), example, "{name}");
    // every fixture key except "RouterInfo" is checked
}
```
- [ ] **Step 2: Run it to verify it fails.** `cargo test --locked --test wire_examples`. Expected: `unresolved import sandbox_builtin::wire`.
- [ ] **Step 3: Implement `src/wire.rs`** per registry §2, with the derives the daemon's request and response types use today.
- [ ] **Step 4: Run the tests to verify they pass.** `cargo test --locked --workspace`: pass.
- [ ] **Step 5: Commit.** `git add sandbox-builtin/src/wire.rs sandbox-builtin/src/lib.rs sandbox-builtin/tests/wire_examples.rs sandbox-builtin/tests/fixtures/wire_examples.json && git commit -m "feat(sandbox-builtin): copy the contract's info and worker-VM types, pinned by its wire fixture"`

### Task 5: Adapter entry point, `sandbox-builtin::*` ids, `adapter::info` and config over the bus

**Files:**
- Create: `sandbox-builtin/iii.worker.yaml`, `sandbox-builtin/README.md`
- Delete: `src/sandbox_daemon/{README.md, iii.worker.yaml, skills/}` (P2a moves them to the router, which owns the `sandbox::*` docs and skill; D8, D10); `src/cli/sandbox_daemon.rs` (its fail-closed load moves into `serve`)
- Modify: `Cargo.toml` (`iii-config-client = { path = "../crates/config-client" }`); `src/main.rs`; `src/cli/app.rs` (`Cli { command: Option<Commands> }` with only `VmBoot`; delete `SandboxDaemonArgs` and `daemon_engine_arg_tests`, :343-354 and :588-630); `src/sandbox_daemon/mod.rs:113-416`; the `register` functions in `fs/*.rs` and `run.rs:330`; `src/sandbox_daemon/config.rs:1-164`; `tests/sandbox_daemon_exit_integration.rs:97-117, 254-275`
- Test: `src/sandbox_daemon/mod.rs`, `src/sandbox_daemon/config.rs`, `tests/sandbox_daemon_exit_integration.rs`

**Interfaces:**
- Consumes (Task 4): `AdapterInfo`, `AdapterInfoRequest`, `ContractVersion::CURRENT`, `CONTRACT_MAJOR`. Also `iii_config_client::fetch(&IIIClient, &str) -> Result<Option<Value>, String>` (`crates/config-client/src/lib.rs:93`) and `iii_sdk::register_worker_from_env`.
- Produces:
  - `pub fn register_all(iii: &IIIClient, config: Arc<SandboxConfig>, registry: Arc<SandboxRegistry>) -> Vec<FunctionRef>`. Each `register_*` returns its `FunctionRef`. The ids are the literal `"sandbox-builtin::<suffix>"`, and each description is today's, with `engine.workers.iii-sandbox.custom_images` replaced by "the adapter's `custom_images` config", as P0 does.
  - `pub fn adapter_info() -> AdapterInfo` = `{ adapter: env!("CARGO_PKG_NAME"), adapter_version: env!("CARGO_PKG_VERSION"), contract: ContractVersion::CURRENT }`, registered as `sandbox-builtin::adapter::info`.
  - `pub fn parse_config(raw: Option<serde_json::Value>) -> anyhow::Result<SandboxConfig>`. `None` gives `SandboxConfig::default()`; it keeps `reject_legacy_wrapped_shape` and `validate_custom_images`.
  - `pub async fn serve() -> anyhow::Result<()>`:
    1. Arm `ExitWatch` first.
    2. `register_worker_from_env` with `WorkerMetadata.name = "sandbox-builtin"` (`III_WORKER_NAME` overrides it).
    3. Race `fetch(III_CONFIG_NAME, else "sandbox-builtin")` against the exit watch. If the watch fires first, shut down as in step 4. On a fetch or parse error, log `failed to load sandbox-builtin config; aborting (fail-closed)` and exit 2.
    4. `register_all`, then the reaper, then wait on the exit watch.
  - CLI: `sandbox-builtin` (no args) serves; `__vm-boot` stays hidden.

- [ ] **Step 1: Write the failing test.**
```rust
#[tokio::test] async fn registered_ids_are_the_contract_operations_under_the_package_name() {
    let iii = iii_sdk::IIIClient::new("ws://127.0.0.1:9");
    let ids: BTreeSet<String> = register_all(&iii, Arc::new(SandboxConfig::default()), Arc::new(SandboxRegistry::default()))
        .into_iter().map(|r| r.id).collect();
    let want: BTreeSet<String> = ["create", "exec", "stop", "list", "catalog::list", "run", "fs::ls", "fs::stat", "fs::mkdir",
        "fs::rm", "fs::chmod", "fs::mv", "fs::grep", "fs::sed", "fs::write", "fs::read", "adapter::info"]
        .iter().map(|s| format!("sandbox-builtin::{s}")).collect();
    assert_eq!(ids, want);
}
#[test] fn adapter_info_names_the_package_and_the_contract() {
    let i = adapter_info();
    assert_eq!((i.adapter.as_str(), i.adapter_version.as_str(), i.contract.major), ("sandbox-builtin", "1.0.0", CONTRACT_MAJOR));
}
#[test] fn parse_config_fills_defaults_and_fails_closed() {
    assert!(parse_config(None).unwrap().image_allowlist.is_empty());
    assert_eq!(parse_config(Some(json!({"image_allowlist": ["python", "node"]}))).unwrap().image_allowlist, ["python", "node"]);
    assert!(parse_config(Some(json!({"sandbox": {}}))).is_err());
    assert!(parse_config(Some(json!({"custom_images": {"python": "ghcr.io/x/y:1"}}))).is_err());
}
```
  `config_fetch_failure_exits_2`: `sandbox_command` and the orphan test's shell line (:264) are rewritten to run the bare binary with `III_URL=ws://127.0.0.1:1`. With `III_ENGINE_PID=<a live sleep>`, the process exits with code 2 within 60 s, and its log contains `aborting (fail-closed)`. The five existing exit-watch tests keep passing: the three 5 s config RPC attempts outlast their 2.5 s survival windows.
- [ ] **Step 2: Run it to verify it fails.** `cargo test --locked --lib -- registered_ids adapter_info parse_config && cargo test --locked --test sandbox_daemon_exit_integration`. Expected: `cannot find function register_all`.
- [ ] **Step 3: Implement** the signatures above.
  - `iii.worker.yaml` holds `iii: v1`, `name: sandbox-builtin`, `language: rust`, `deploy: binary`, `manifest: Cargo.toml`, `bin: sandbox-builtin`, `license: Elastic-2.0`, a description, `tags: [sandbox, microvm, libkrun, vm]`, and `config:` = the old `src/sandbox_daemon/iii.worker.yaml` config.
  - `README.md` (the adapter's own; no skill, D8) covers what the adapter serves, its config keys and the four `III_VM_*` env vars, host prerequisites (spec 8.6), the worker-VM family with its two guards (spec 7.8), and a link to the S-codes at `https://iii.dev/docs/creating-workers/sandboxes`.
- [ ] **Step 4: Run the tests to verify they pass.** `cargo clippy --locked --workspace --all-targets -- -D warnings && cargo test --locked --workspace`: pass.
- [ ] **Step 5: Commit.** `git add -A sandbox-builtin && git commit -m "feat(sandbox-builtin): serve sandbox-builtin::* and adapter::info with config from the bus"`

### Task 6: `worker::prepare` behind the project token and host roots

**Files:**
- Create: `sandbox-builtin/src/worker_vm.rs` (`pub mod worker_vm;` in `src/lib.rs`)
- Modify: `src/cli/local_worker.rs:909-1166` (extract `validate_source`; `start_worker_impl`'s `is_bundle: bool` becomes `package: Option<&str>`, `Some` for bundles; `bundle_vm_command` gains `package`); `src/sandbox_daemon/mod.rs` (`register_all` registers `worker::prepare`); `tests/bundle_vm_command.rs` (the `package` argument)
- Test: `src/worker_vm.rs`

**Interfaces:**
- Consumes (Task 4): `WorkerPrepareRequest`, `WorkerPrepareResponse`, `WorkerSource::{Bundle{package, host_path}, Local{host_path, run_override}}`, `WORKER_PREPARE`.
- Produces:
```rust
pub struct WorkerVms { /* base, token, roots, live map (Task 7) */ }
impl WorkerVms {
    pub fn new(base: PathBuf, token: Option<String>, roots: Vec<PathBuf>) -> Self;
    pub fn from_env() -> Self; // ~/.iii/sandbox/workers, III_SANDBOX_WORKER_TOKEN, split_paths(III_SANDBOX_HOST_ROOTS)
    pub async fn prepare(&self, req: WorkerPrepareRequest) -> Result<WorkerPrepareResponse, SandboxError>;
}
pub fn vm_id(state_key: &str) -> String; // first 32 hex chars of SHA-256(state_key)
pub(crate) fn validate_source(worker_dir: &Path, package: Option<&str>, run_override: Option<&str>)
    -> Result<(ProjectInfo, String /* kind */), String>; // local_worker.rs: steps 1-2, the bundle kill switch, validate_compose_base_image
pub async fn bundle_vm_command(package: &str, worker_name: &str, install_dir: &Path, over: VmOverride<'_>) -> Result<VmCommand, String>;
```
- Decisions:
  - `prepare` runs: the token check, then `admit` (the canonical `host_path` must `starts_with` a canonical root, else S501 naming the roots), then `validate_source` (Err → S501), then `oci::prepare_rootfs(kind, base_image)` (Err → S506). `start_worker_impl` calls `validate_source` instead of its inline steps.
  - The bundle manifest name is checked against `package`, not the container key, which fixes the coupling at `iii:crates/iii-worker/src/cli/bundle_prepare.rs:29-31`.
  - Token comparison is constant-time: equal length, then an XOR fold over the bytes. A missing token or an empty root list rejects every call (S505 or S501).

- [ ] **Step 1: Write the failing test.** Use `tokio::test` with a tempdir `t`; `vms(token, roots)` builds `WorkerVms::new(t/"workers", token, roots)`.
  - `worker_ops_answer_s505_without_the_token`: `vms(None, [t])` and `vms(Some("t"), [t])` with `auth: "u"` both give code S505.
  - `host_paths_outside_the_roots_answer_s501`: `/etc`, `t/../etc`, a symlink `t/esc` → `/etc`, and roots `[]` all give S501, and the message names `t`.
  - `host_roots_accept_a_path_under_a_symlinked_root`: root `t/link` → `t/real`. Both `t/real/w` and `t/link/w` are admitted, and on macOS so is a root under `/tmp`.
  - `a_bundle_naming_another_package_is_s501`: manifest `name: other`, `package: "probe"` gives S501.
  - `vm_id_is_stable_and_32_hex`: `vm_id("a") == vm_id("a")`, `len() == 32`, and `vm_id("a") != vm_id("b")`.
  - Extend `registered_ids_…` with `"sandbox-builtin::worker::prepare"`.
- [ ] **Step 2: Run it to verify it fails.** `cargo test --locked --lib worker_vm`. Expected: `unresolved module worker_vm`.
- [ ] **Step 3: Implement** the signatures and decisions above. `register_all` builds `WorkerVms::from_env()`.
- [ ] **Step 4: Run the tests to verify they pass.** `cargo test --locked --workspace`: pass, including `bundle_vm_command` and `local_vm_command`.
- [ ] **Step 5: Commit.** `git add sandbox-builtin/src sandbox-builtin/tests && git commit -m "feat(sandbox-builtin): add worker::prepare behind the project token and host roots"`

### Task 7: `worker::start`, `status`, `stop` and `destroy`

**Files:**
- Modify: `src/worker_vm.rs`, `src/sandbox_daemon/mod.rs` (register the four ids)
- Create: `tests/worker_vm_e2e.rs` (`#![cfg(feature = "integration-vm")]`)
- Test: `src/worker_vm.rs`, `tests/worker_vm_e2e.rs`

**Interfaces:**
- Consumes (Task 4): `WorkerStartRequest`, `WorkerStartResponse`, `WorkerStatusRequest`, `WorkerStatus`, `State`, `WorkerStopRequest`, `WorkerDestroyRequest`, `WorkerDestroyResponse`, `Owner`, `WORKER_{START, STATUS, STOP, DESTROY}`. From Task 1: `daemon_exit::attach_lifeline(&mut tokio::process::Command) -> io::Result<Lifeline>`.
- Produces:
```rust
impl WorkerVms {
    pub async fn start(&self, req: WorkerStartRequest) -> Result<WorkerStartResponse, SandboxError>;
    pub fn status(&self, req: WorkerStatusRequest) -> Result<WorkerStatus, SandboxError>;
    pub async fn stop(&self, req: WorkerStopRequest) -> Result<WorkerStatus, SandboxError>;
    pub fn destroy(&self, req: WorkerDestroyRequest) -> Result<WorkerDestroyResponse, SandboxError>;
    pub(crate) fn launch(&self, state_key: &str, cmd: tokio::process::Command) -> Result<WorkerStartResponse, SandboxError>;
}
```
- Decisions:
  - State lives in `<base>/<vm_id>/`, which is also `VmOverride.state_dir`. `state.json` is `{"instance": n}`, rewritten before each spawn with `n` = the stored value + 1 (the first boot is 1).
  - `start` checks the token and admits the source. `network: false` gives S501, since a worker VM must reach the engine (5.8). It answers S502 when the `vm_id` is present and not exited, and marks the entry `Preparing` before it builds. It calls `bundle_vm_command(package, worker_name, host_path, over)` or `local_vm_command(worker_name, host_path, run_override, over)` with `over = VmOverride { state_dir, engine_url: &req.engine_url, extra_env: req.env }`. A build error is S300. Then `launch`.
  - `launch` truncates `<state>/console.log`, sets stdin to null and pipes stdout and stderr into a pump. The pump appends one JSON line `{stream, line, ts_ms}` per output line (lossy UTF-8) to `console.log`. The `Lifeline` is held with the child. A waiter task records `exit_code` (`code()`), `signal` (`ExitStatusExt::signal()`) and `ended_at_ms`.
  - `status`: an unknown `vm_id` gives S504 and a different `instance` gives S503. The state is `Running` until exit, then `Stopped` if `stop` caused the exit, else `Exited`.
  - `stop`: same S504 and S503 checks. An exited VM returns its final status. Otherwise send SIGTERM, wait up to `grace_ms`, then SIGKILL; `grace_ms: 0` sends SIGKILL at once. (`__vm-boot` turns SIGTERM into a clean VMM exit, `vm_boot.rs:1137-1176`.)
  - `destroy`: S502 while running; otherwise remove `<base>/<vm_id>` and the map entry. `destroyed` is whether the directory existed.

- [ ] **Step 1: Write the failing test.** `launch` takes `sh` commands, so there is no VM.
  - `start_status_stop_destroy_cycle`:
    1. Launch `sh -c 'echo hi; sleep 30'`.
    2. `status` is `Running` with `instance == 1`.
    3. `stop{grace_ms: 2000}` gives `state == Stopped` and `signal == Some(15)`.
    4. `destroy` gives `destroyed == true`, and `<base>/<vm_id>` no longer exists.
  - `instance_survives_an_adapter_restart`: launch, stop, drop `WorkerVms`, then `new` on the same base. `launch(same key)` gives `instance == 2`, and `status{instance: 1}` gives S503.
  - `stop_escalates_to_sigkill_after_grace`: `sh -c 'trap "" TERM; sleep 30'` with `stop{grace_ms: 300}` returns within 5 s with `signal == Some(9)` and `state == Stopped`.
  - `a_running_vm_refuses_start_and_destroy`: a second `launch` and a `destroy` both give S502.
  - `unknown_vm_is_s504`.
  - `exit_code_is_recorded`: `sh -c 'exit 7'` reaches `Exited` with `exit_code == Some(7)`.
  - `start_refuses_network_false`: `start{network: false}` gives S501 before any build.
  - `tests/worker_vm_e2e.rs::a_local_worker_vm_boots_restarts_and_is_destroyed`. Skip with `[skip]` if `/dev/kvm` is not read-write. Use a local dir with `runtime.base_image: docker.io/iiidev/python:latest` and `run_override: "sh -c 'echo worker-vm-ready; sleep 600'"`. Then:
    1. `prepare`.
    2. `start` reaches `Running`.
    3. `console.log` contains `worker-vm-ready` within 120 s.
    4. `stop{grace_ms: 5000}` gives `Stopped`.
    5. A second `start` gives `instance == 2` and reaches `Running`; stop it.
    6. `destroy` gives `destroyed == true`.
  - Extend `registered_ids_…` with `WORKER_{START, STATUS, STOP, DESTROY}`.
- [ ] **Step 2: Run it to verify it fails.** `cargo test --locked --lib worker_vm`. Expected: `no method named launch`.
- [ ] **Step 3: Implement** the signatures and decisions above.
- [ ] **Step 4: Run the tests to verify they pass.** `cargo test --locked --workspace` passes. On a KVM host, run `BUILD_TARGET=x86_64-unknown-linux-gnu scripts/build-init.sh && cargo test --locked --features embed-init,embed-libkrunfw,integration-vm --test worker_vm_e2e`; it prints `test result: ok. 1 passed`.
- [ ] **Step 5: Commit.** `git add sandbox-builtin/src sandbox-builtin/tests/worker_vm_e2e.rs && git commit -m "feat(sandbox-builtin): run worker VMs with worker::start, status, stop and destroy"`

### Task 8: `worker::logs`

**Files:**
- Modify: `src/worker_vm.rs`, `src/sandbox_daemon/mod.rs` (register `worker::logs`)
- Test: `src/worker_vm.rs`

**Interfaces:**
- Consumes (Task 4): `WorkerLogsRequest`, `WorkerLogsResponse`, `LogRecord`, `LogStream`, `WORKER_LOGS`; `req.channel` is already `iii_sdk::channels::StreamChannelRef`.
- Produces:
```rust
pub(crate) fn read_records(path: &Path, from: u64) -> std::io::Result<(Vec<LogRecord>, u64)>;
impl WorkerVms {
    pub async fn logs(&self, iii: &IIIClient, req: WorkerLogsRequest) -> Result<WorkerLogsResponse, SandboxError>;
    pub(crate) async fn pump_logs<F, Fut>(&self, req: &WorkerLogsRequest, sink: F) -> Result<(), SandboxError>
        where F: FnMut(LogRecord) -> Fut, Fut: Future<Output = Result<(), String>>;
}
```
- Decisions:
  - `LogRecord.cursor` is the byte offset just past that record, the resume point; `cursor: None` reads from 0.
  - With `follow`, the pump polls every 200 ms until that instance has exited and the file is drained.
  - `logs` checks the token, gives S504 for an unknown `vm_id`, spawns the pump with a `ChannelWriter::new(iii.address(), &req.channel)` sink (one `send_message(<LogRecord JSON>)` per record), and returns. The pump closes the writer at the end. A sink error ends the pump.

- [ ] **Step 1: Write the failing test.**
  - `records_resume_from_a_cursor`: a file of three JSON lines.
    - `read_records(p, 0)` gives 3 records with strictly increasing cursors, and the last cursor equals the file length.
    - `read_records(p, recs[0].cursor)` gives the last 2 records.
  - `follow_streams_until_the_instance_ends`: launch `sh -c 'echo a; sleep 0.3; echo b >&2'`, then `pump_logs(follow: true)` into a `Vec`. The lines are `["a", "b"]`, the streams are `[Stdout, Stderr]`, and it returns after exit.
  - `follow_on_an_exited_instance_drains_and_returns`: launch `sh -c 'echo a'`, wait for `Exited`. `pump_logs(follow: true)` returns within 2 s with `["a"]`.
  - `registered_ids_…` now expects exactly the 16 ephemeral suffixes, `adapter::info` and the six `WORKER_*` ids.
- [ ] **Step 2: Run it to verify it fails.** `cargo test --locked --lib -- read_records pump_logs`. Expected: `cannot find function read_records`.
- [ ] **Step 3: Implement** the signatures and decisions above.
- [ ] **Step 4: Run the tests to verify they pass.** `cargo test --locked --workspace`: pass.
- [ ] **Step 5: Commit.** `git add sandbox-builtin/src && git commit -m "feat(sandbox-builtin): stream worker VM logs with worker::logs"`

### Task 9: Isolation e2e through `iii trigger`

**Files:**
- Create: `sandbox-builtin/scripts/sandbox-isolation-e2e.sh` (mode 755), from `iii:scripts/sandbox-isolation-e2e.sh`

**Interfaces:**
- Consumes: Task 5 ids and `configuration::register` (a mandatory engine worker); `iii trigger <fn> --json <payload> --address <host> --port <port> --timeout-ms <ms>` (`iii:engine/src/cli_trigger/mod.rs:29-76`; the two address flags are deprecated but work on every 0.24 engine, and their warning goes to stderr).
- Produces: env `III_BIN`, `ADAPTER_BIN` (registry §6; replaces `WORKER_BIN`) and `SANDBOX_ID_PREFIX` (default `sandbox-builtin`; P5 sets `sandbox`), plus `IMAGE_REF`, `PORT` and `WORK` as today.

- [ ] **Step 1: Write the failing test.** Copy the script unchanged.
- [ ] **Step 2: Run it to verify it fails.** On a KVM host: `III_BIN=$(command -v iii) WORKER_BIN=target/debug/sandbox-builtin bash scripts/sandbox-isolation-e2e.sh`. Expected: `sandbox daemon did not register` (there is no `sandbox-daemon` subcommand).
- [ ] **Step 3: Implement** the rewrite. `trig <secs> <fn> <json>` runs `"$III_BIN" trigger` under `run timeout`, with `--address 127.0.0.1 --port "$PORT" --timeout-ms $((secs*1000))`. `P=$SANDBOX_ID_PREFIX`. Require `jq`. The usage header names `III_BIN`, `ADAPTER_BIN` and `SANDBOX_ID_PREFIX` (P5 reads it). Everything else (checks, manifests, cleanup, the engine `config.yaml` heredoc) is unchanged apart from the binary name.

| Today | Through the bus |
|---|---|
| `sandbox-daemon --config sandbox.yaml --engine …` | `trig 30 configuration::register` `{id: "sandbox-builtin", name, description, schema: {"type": "object"}, initial_value: <the old sandbox.yaml as JSON>}`, then `III_URL=ws://127.0.0.1:$PORT "$ADAPTER_BIN"` in `RUN_ENV` |
| `sandbox list` | `trig 10 "$P::list" '{}'` |
| `sandbox create e2e-image` | `trig 900 "$P::create" '{"image":"e2e-image"}' \| jq -r .sandbox_id` (retry on `timed out` as today) |
| `sandbox exec -e K=V id -- /bin/sh -c S` | `trig 300 "$P::exec"` `{sandbox_id, argv: ["/bin/sh","-c",S], env: {K: V}, timeout_ms: 290000}`; print `.stdout` and `.stderr`, return `.exit_code` |
| `sandbox upload id file /usr/bin/x` | `trig 60 "$P::fs::write"` `{sandbox_id, path, content: <file text>}` |
| `sandbox stop id` | `trig 60 "$P::stop" '{"sandbox_id": …, "wait": true}'` |
| `"$WORKER_BIN __vm-boot"` (cleanup `ps` match) | `"$ADAPTER_BIN __vm-boot"` |

- [ ] **Step 4: Run the tests to verify they pass.** On a Linux KVM host:
```
BUILD_TARGET=x86_64-unknown-linux-gnu scripts/build-init.sh
cargo build --locked --features embed-init,embed-libkrunfw
III_BIN=$(command -v iii) ADAPTER_BIN=target/debug/sandbox-builtin bash scripts/sandbox-isolation-e2e.sh
```
  The last line is `===== result: 0 failure(s)`.
- [ ] **Step 5: Commit.** `git add sandbox-builtin/scripts/sandbox-isolation-e2e.sh && git commit -m "test(sandbox-builtin): drive the isolation e2e through iii trigger"`

### Task 10: CI jobs

**Files:**
- Modify: `.github/workflows/ci.yml:264-279` (rust job), `:986-989` (interface-smoke); add jobs `sandbox-builtin-macos` and `sandbox-builtin-vm-linux` after `iii-directory-minilm-targets` (:303-351)
- Test: `.github/scripts/tests/test_rust_ci_workflows.py`

**Interfaces:**
- Consumes: `scripts/build-init.sh` (Task 2), `tests/worker_vm_e2e.rs` (Task 7), `scripts/sandbox-isolation-e2e.sh` (Task 9).
- Produces: CI job names `sandbox-builtin-macos` and `sandbox-builtin-vm-linux` (registry). Both run when `contains(needs.discover.outputs.rust, '"sandbox-builtin"')`.
- Decisions:
  - rust job: the generic clippy and test steps get `&& matrix.worker != 'sandbox-builtin'`. Add three steps:
    - `Install libcap-ng (sandbox-builtin)`
    - `cargo clippy --locked --workspace --all-targets --features embed-init,embed-libkrunfw -- -D warnings`
    - `cargo test --locked --workspace --features embed-init,embed-libkrunfw`
  - interface-smoke: the same libcap-ng step before `Build worker binary`.
  - `sandbox-builtin-macos` runs on `macos-latest` with toolchain `1.97.1` (`build-init.sh` adds the musl target). Steps:
    - `BUILD_TARGET=aarch64-apple-darwin scripts/build-init.sh`
    - the clippy and test lines above without `--workspace`
    - `cargo test --locked -p iii-filesystem`
  - `sandbox-builtin-vm-linux` runs on `ubuntu-24.04` with `timeout-minutes: 60` and no `continue-on-error`. Steps:
    1. The "Enable KVM" step from `iii:.github/workflows/ci.yml:680-687` (fail if `/dev/kvm` is missing).
    2. libcap-ng.
    3. The pinned engine install (as `ci.yml:690-706`).
    4. `scripts/build-init.sh` with `BUILD_TARGET=x86_64-unknown-linux-gnu`.
    5. `cargo test --locked --features embed-init,embed-libkrunfw,integration-vm`.
    6. `cargo build --locked --features embed-init,embed-libkrunfw`.
    7. The isolation e2e with `III_BIN=$III_ENGINE_BIN ADAPTER_BIN=target/debug/sandbox-builtin WORK=$RUNNER_TEMP/sandbox-isolation`.
    8. On failure, upload `$RUNNER_TEMP/sandbox-isolation/logs`.
  - Every `uses:` is the SHA already pinned in `ci.yml`.

- [ ] **Step 1: Write the failing test.**
```python
def test_sandbox_builtin_vm_linux_is_a_required_shaped_kvm_job() -> None:
    job = workflow("ci.yml")["jobs"]["sandbox-builtin-vm-linux"]
    assert "continue-on-error" not in job and job["runs-on"] == "ubuntu-24.04"
    body = str(job)
    for needle in ("/dev/kvm", "scripts/build-init.sh", "--features embed-init,embed-libkrunfw,integration-vm",
                   "scripts/sandbox-isolation-e2e.sh", "libcap-ng-dev"):
        assert needle in body
    assert "sandbox-builtin-macos" in workflow("ci.yml")["jobs"]
```
- [ ] **Step 2: Run it to verify it fails.** `pytest .github/scripts/tests/test_rust_ci_workflows.py -q`. Expected: `KeyError: 'sandbox-builtin-vm-linux'`.
- [ ] **Step 3: Implement** the jobs and steps above.
- [ ] **Step 4: Run the tests to verify they pass.** `pytest .github/scripts/tests/ -q` passes, and `actionlint -color` reports nothing.
- [ ] **Step 5: Commit.** `git add .github/workflows/ci.yml .github/scripts/tests/test_rust_ci_workflows.py && git commit -m "ci(sandbox-builtin): lint, test and boot real VMs on every sandbox-builtin change"`

### Task 11: Release pipeline and catalog entry

**Files:**
- Modify:
  - `.deploy/workers.yaml` (insert `sandbox-builtin:` before `sandbox-code-runner:` at :2089)
  - `.github/scripts/deployment_compiler.py:264-276` (allowed fields) and `:332-345` (validation beside `companions`)
  - `.github/contracts/deployment-descriptor.schema.json:131-144` (`rustArtifact`)
  - `.github/scripts/deployment_train.py:510-518`
  - `.github/workflows/_deploy-build.yml:108-125`
  - `.github/scripts/tests/test_deployment_compiler.py:266-280`
- Create: `sandbox-builtin/entitlements.plist` (the two entitlements, as `iii:crates/iii-worker/src/cli/worker_manager/platform.rs:41-49`)
- Test: `.github/scripts/tests/{test_deployment_compiler.py, test_deployment_train.py}`

**Interfaces:**
- Consumes: `scripts/build-init.sh` (`BUILD_TARGET`), the `embed-init` and `embed-libkrunfw` features.
- Produces: the catalog entry with the Global Constraints values, under `artifact:`.
- Decisions:
  - Compiler:
    - `features` is a non-empty list of `^[a-z0-9_-]+$`.
    - `prebuild` is a non-empty argv whose first element is a file under the worker dir (`safe_relative`).
    - `macos_entitlements` is a file under the worker dir.
    - Each error names its field. The schema adds the same three properties.
  - Train:
    - Before `cargo build`, run `run(*prebuild, cwd=source_dir, env={**os.environ, "BUILD_TARGET": target})`, then append `--features <comma-joined>`.
    - On `-apple-darwin`, after the build, run `run("codesign", "--sign", "-", "--entitlements", str(source_dir / plist), "--force", str(binary_path))`.
    - Then `subprocess.check_output(["codesign", "-d", "--entitlements", "-", str(binary_path)], stderr=subprocess.STDOUT, text=True)` must contain `com.apple.security.hypervisor`, else `SystemExit`.
    - The runtime self-sign (`platform.rs:18-68`) then returns early on every release build, so nothing in the Compose cache is re-signed (spec 8.6). It still signs unsigned dev builds in place.
  - `_deploy-build.yml`: add step `Install libcap-ng (sandbox-builtin)`, run `if: inputs.worker == 'sandbox-builtin' && runner.os == 'Linux'`.
    - On `x86_64-unknown-linux-gnu`: `apt-get install -y libcap-ng-dev`.
    - On `aarch64-unknown-linux-gnu`: the arm64 multiarch recipe of `iii:.github/workflows/ci.yml:451-468`, then `libcap-ng-dev:arm64` and `echo "LIBRARY_PATH=/usr/lib/aarch64-linux-gnu" >> "$GITHUB_ENV"`.

- [ ] **Step 1: Write the failing test.**
```python
def test_sandbox_builtin_build_extras_are_validated_and_reach_the_schema():
    jsonschema = pytest.importorskip("jsonschema")
    schema = json.loads((ROOT / ".github" / "contracts" / "deployment-descriptor.schema.json").read_text(encoding="utf-8"))
    catalog = deployment_compiler.read_yaml(ROOT / ".deploy" / "workers.yaml")["workers"]
    compiled = deployment_compiler.compile_worker(ROOT, "sandbox-builtin", catalog["sandbox-builtin"], "a" * 40, "b" * 64)
    art = compiled["artifact"]
    assert art["features"] == ["embed-init", "embed-libkrunfw"] and art["prebuild"] == ["scripts/build-init.sh"]
    assert art["macos_entitlements"] == "entitlements.plist"
    assert art["targets"] == ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"]
    jsonschema.Draft202012Validator(schema).validate(compiled)
    for field, broken in (("features", []), ("features", ["Bad Feature"]), ("prebuild", []),
                          ("prebuild", ["../escape.sh"]), ("macos_entitlements", "missing.plist")):
        entry = json.loads(json.dumps(catalog["sandbox-builtin"])); entry["artifact"][field] = broken
        with pytest.raises(ValueError, match=field):
            deployment_compiler.compile_worker(ROOT, "sandbox-builtin", entry, "a" * 40, "b" * 64)
```
  - `test_rust_prebuild_features_and_entitlements_drive_the_build` follows `test_rust_companions_ship_beside_the_binary_as_plain_files` (`test_deployment_train.py:279`), for `aarch64-apple-darwin` with `run` recording `(command, kwargs)`:
    1. The prebuild runs first, with `cwd == source dir` and `env["BUILD_TARGET"] == target`.
    2. The cargo command ends with `--features embed-init,embed-libkrunfw`.
    3. `codesign --sign - --entitlements <source>/entitlements.plist --force <bin>` runs last.
    4. With a `check_output` stub that answers `git rev-parse` with the source SHA and `codesign -d` with `""`, `build` raises `SystemExit`.
  - Edit `test_binary_catalog_restores_intel_macos_except_the_microvm_sandbox` to read `(worker not in {"sandbox-code-runner", "sandbox-builtin"})`, and add `"sandbox-builtin"` to the checked set.
- [ ] **Step 2: Run it to verify it fails.** `pytest .github/scripts/tests/test_deployment_compiler.py .github/scripts/tests/test_deployment_train.py -q`. Expected: `KeyError: 'sandbox-builtin'`.
- [ ] **Step 3: Implement** the decisions above.
- [ ] **Step 4: Run the tests to verify they pass.** All of these pass:
  - `pytest .github/scripts/tests/ -q`
  - `python3 .github/scripts/validate_worker.py --worker sandbox-builtin --base-ref origin/main --source-changed '["sandbox-builtin"]'`
  - `actionlint -color`
- [ ] **Step 5: Commit.** `git add .deploy/workers.yaml .github sandbox-builtin/entitlements.plist && git commit -m "ci(sandbox-builtin): release it as a rust binary with prebuild, features and build-time signing"`

### Task 12: Guest base images workflow

**Files:**
- Create: `.github/workflows/sandbox-builtin-images.yml`, from `iii:.github/workflows/docker-worker-images.yml`

**Interfaces:**
- Consumes: `sandbox-builtin/images/{node,python}/Dockerfile` (Task 1). Secrets `DOCKERHUB_USERNAME` and `DOCKERHUB_PASSWORD`.
- Produces: the workflow `sandbox-builtin-images.yml` (registry). Same matrix, smoke test and tags (`iiidev/<language>:latest|builder`) as the iii workflow.
- Decisions:
  - The only trigger is `workflow_dispatch` (D7): no `pull_request` or `push`, so it never pushes while iii's `docker-worker-images.yml` exists. P3 deletes that one.
  - Both `context:` values point at `sandbox-builtin/images/${{ matrix.language }}`.
  - Checkout is `actions/checkout@fbc6f3992d24b796d5a048ff273f7fcc4a7b6c09` with `persist-credentials: false`.
  - `docker/setup-qemu-action`, `setup-buildx-action` and `login-action` reuse the v4 SHAs already pinned at `_deploy-build.yml:231, 237` and `build.yml:488`. `docker/build-push-action` is pinned to its v6 commit (`gh api repos/docker/build-push-action/commits/v6 --jq .sha`) with a `# v6` comment.

- [ ] **Step 1: Write the failing test.** Add the file with the iii tags unpinned (`@v4`, `@v3`).
- [ ] **Step 2: Run it to verify it fails.** `pytest .github/scripts/tests/test_rust_ci_workflows.py::test_all_external_actions_are_pinned_by_commit_sha -q`. Expected: `mutable action reference actions/checkout@v4`.
- [ ] **Step 3: Implement** the decisions above.
- [ ] **Step 4: Run the tests to verify they pass.** `pytest .github/scripts/tests/ -q && actionlint -color`: clean.
- [ ] **Step 5: Commit.** `git add .github/workflows/sandbox-builtin-images.yml && git commit -m "ci(sandbox-builtin): build and publish the node and python guest base images"`

### Task 13: Whole-plan verification and PR-ready note

**Files:** none changed.

- [ ] **Step 1: Run the full checks.** In `sandbox-builtin/`:
  - `cargo fmt --all -- --check`
  - `cargo clippy --locked --workspace --all-targets --features embed-init,embed-libkrunfw -- -D warnings`
  - `cargo test --locked --workspace --features embed-init,embed-libkrunfw`

  At the repo root:
  - `pytest .github/scripts/tests/ -q`
  - `python3 .github/scripts/validate_worker.py --worker sandbox-builtin --base-ref origin/main --source-changed '["sandbox-builtin"]'`
  - `actionlint -color`

  All pass with no warnings.
- [ ] **Step 2: Run the spec 11 checks this slice owns.**
  - The error JSON matches `tests/fixtures/sandbox_error_codes.json` (`--test sandbox_error_codes_integration`).
  - The local wire types round-trip `tests/fixtures/wire_examples.json` (`--test wire_examples`), and both fixtures equal P0's merged copies (`diff` against `iii:crates/iii-sandbox-contract/tests/fixtures/`).
  - The registered ids are exactly the 16 ephemeral suffixes, `adapter::info` and the six `WORKER_*` ids under `sandbox-builtin::` (`registered_ids_…`).
  - On a Linux KVM host:
    - `cargo test --locked --features embed-init,embed-libkrunfw,integration-vm` passes, including `worker_vm_e2e`.
    - The isolation e2e ends with `0 failure(s)`.
  - On Apple Silicon, after `cargo build --release` and the train's `codesign --sign - --entitlements entitlements.plist --force target/release/sandbox-builtin`, `codesign -d --entitlements - target/release/sandbox-builtin` lists `com.apple.security.hypervisor`.
- [ ] **Step 3: Write the PR-ready note** in the PR body. Do not push or open the PR from this plan.
  - The PR title carries the MOT ticket.
  - Prerequisites: none (spec Q5). Its two fixtures are copies of P0's; re-copy them if P0 changes them before merging.
  - Operational, outside the diff:
    - Mark `sandbox-builtin-vm-linux` as a required check (branch protection, D5).
    - Add `DOCKERHUB_USERNAME` and `DOCKERHUB_PASSWORD` to the workers repo. Do not dispatch `sandbox-builtin-images.yml` before P3 deletes `iii:.github/workflows/docker-worker-images.yml` (D7).
    - After merge, Release Control publishes 1.0.0. P3 cannot merge before it is in the registry.
  - Nothing consumes `sandbox-builtin::*` until engine 0.25.0.
