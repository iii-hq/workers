# Sandbox worker: plan index

Index of the six implementation plans for `docs/superpowers/specs/2026-10-20-sandbox-worker-design.md`. Every plan uses the names in `2026-10-20-sandbox-shared-interfaces.md`. Read the spec, the registry and the plan before starting a task.

## Plans and order

| Order | Plan | Repo | Branch | Merges when |
|---|---|---|---|---|
| 1 | [P0 contract crate](2026-10-20-sandbox-0-contract.md) | iii | `sandbox/p0-contract` | any time (internal crate, no behaviour change) |
| 2 | [P1 sandbox-builtin](2026-10-20-sandbox-1-builtin-worker.md) | workers | `sandbox/p1-builtin` | after P0 merges: it copies P0's merged fixtures from iii main and uses no iii code. Release Control then publishes `sandbox-builtin` 1.0.0 |
| 3 | [P2a engine router](2026-10-20-sandbox-2a-engine-router.md) | iii | `sandbox/p2a-router` | after P0, once `sandbox-builtin` 1.0.0 resolves for the three targets (the P3 Task 7 Step 3 curl) and P2b is approved, so P2a, P2b and P3 merge back to back. From this merge on, no stable tag from main until 0.25.0, a window that now spans only those three merges |
| 4 | [P2b Compose](2026-10-20-sandbox-2b-compose.md) | iii | `sandbox/p2b-compose` | right after P2a; reviewed on a branch stacked on P2a, so it is approved before P2a merges (its e2e pulls `sandbox-builtin` 1.0.0) |
| 5 | [P3 engine cleanup](2026-10-20-sandbox-3-engine-cleanup.md) | iii | `sandbox/p3-engine-cleanup` | right after P2b, once its Task 7 registry gate passes; the 0.25.0 tag follows |
| 6 | Release engine `iii/v0.25.0` (rollout step 4) | iii | | after P3; checklist below |
| 7 | [P5 consumers](2026-10-20-sandbox-5-consumers.md) | workers | `sandbox/p5-consumers` | after 0.25.0 ships (its CI pins move to `iii/v0.25.0`) |

All PRs can be developed in parallel; only the merges follow this order. Nothing is published to crates.io (spec Q5): P1 keeps its own copies of the wire types and round-trips P0's `sandbox_error_codes.json` and `wire_examples.json`, copied from iii main after P0 merges. P1's `sandbox-builtin-vm-linux` job diffs both copies against iii main on every run, so a later fixture change on either side fails it.

Until P3 merges, any iii change to the paths P1 copied (`crates/iii-{worker,init,network,filesystem,supervisor,shell-proto,shell-client}`, `engine/firmware`, `engine/tests/vm_platform_e2e.rs`, `scripts/sandbox-isolation-e2e.sh`) is ported to `sandbox-builtin` alongside it and released as 1.0.x. P3 Task 5 Step 1 checks that `git log a1d08d8b0..HEAD` on those paths lists only P0, P2a and P2b commits.

## Before each PR

- Create the MOT ticket in Linear first and put it in the PR title.
- Conventional commit messages. No AI co-author trailer and no "Generated with" line, in commits or PR bodies.

## Operational prerequisites

1. **Docker Hub:** the workers repo has `DOCKERHUB_USERNAME` and `DOCKERHUB_PASSWORD`. Do not dispatch `sandbox-builtin-images.yml` until P3 deletes iii's `docker-worker-images.yml` (D7).
2. **Branch protection (D5):** make `sandbox-builtin-vm-linux` a required check in the workers repo right after P1 merges (it gates registry releases), and `compose-sandbox-e2e-linux` a required check in iii right after P2b merges. Not earlier: until its job exists on main no workflow reports it, and every PR would wait on "Expected".
3. **Registry state:** `sandbox-builtin` 1.0.0 published for `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` before P2a merges; the `onboarding` bundle at the version P2b's e2e pins (0.1.20) is published.
4. **Release runner:** the workers interface-capture runners execute the linux-gnu adapter, which links `libcap-ng0`; confirm it is installed on the release runner at the first release.

## Rollout step 4: release engine 0.25.0

- [ ] P2a, P2b and P3 are merged; P3's Task 7 registry gate passed.
- [ ] Cut `iii/v0.25.0` with `create-tag.yml`.
- [ ] `gh release view iii/v0.25.0 --repo iii-hq/iii --json assets --jq '.assets[].name' | grep -E '^(iii-worker|iii-init|libkrunfw)-'` prints nothing (P3 Task 7 Step 4).
- [ ] The release promotes `docs/next/` to Latest: `https://iii.dev/docs/upgrading/sandbox-worker` and the `#Sxxx` anchors on `https://iii.dev/docs/creating-workers/sandboxes` resolve.
- [ ] The release notes say: the first `iii update` from 0.24 still runs the 0.24 updater, so it updates `iii`, then reports "asset not found" for `iii-init` and `iii-worker` and exits 1; a second `iii update` (now 0.25) is clean and prints the leftover-binaries note (spec 9 step 1).
- [ ] Start P5.

## Decisions taken while planning

The plan drafts raised about sixty open questions. The coordinator settled them as D1-D14; the user then decided Q5. The spec (tag `[Designer, plan triage]`, rows T1 and Q5) and the registry (`(Dn)`, `Q5`) already reflect them.

- **Q5 (user)** `iii-sandbox-contract` is internal (`publish = false`) and never published to crates.io. `sandbox-builtin` keeps its own copies of the wire types, pinned by the contract's two fixtures, and the Compose e2e runs the real adapter. Publish when a second adapter lives outside the iii repo.
- **D1** Superseded by Q5: the crate keeps `version.workspace = true`, which no longer matters.
- **D2** Superseded by Q5: the contract crate is Elastic-2.0 like the other iii crates, and `sandbox-builtin` keeps Elastic-2.0, the moved code's license. Nothing is relicensed.
- **D3** No `Family` enum; `OPERATIONS` holds only the 16 ephemeral ids, and the worker ops are `WORKER_*` suffix constants. No `WorkerStartRequest.resources`/`Resources` (the adapter reads the manifest) and no `WorkerSource::Local.sync` (local sources always sync). No new `worker_vm` adapter config keys.
- **D4** Superseded by Q5: nothing is published.
- **D5** The spec-11 acceptance e2e lives in iii (`compose-sandbox-e2e-linux`, P2b), pinning `sandbox-builtin` and `onboarding`. P1's `sandbox-builtin-vm-linux` runs the VM tests, the adapter-level worker VM e2e and the isolation e2e.
- **D6** Builtin stop is SIGTERM, wait `grace_ms`, then SIGKILL, as today.
- **D7** `sandbox-builtin-images.yml` has only a `workflow_dispatch` trigger; P3 deletes iii's images workflow.
- **D8** No musl build scripts are copied; the adapter ships no skill (the router owns it).
- **D9** P2a removes `iii-sandbox` from `CONFIGURABLE_ENGINE_WORKERS` with the Compose fixtures in the same PR; P2b owns the Compose texts, `SANDBOX_CONFIG_MOVED` and the `compose::add iii-sandbox` rewrite.
- **D10** Router: `function_not_found` on forward is S301; structural binding errors may surface as the config parse error; the handshake cache clears only on S301 or `invocation_stopped` (a documented limit); the README moves in P2a; the prefetch deletion stays in P2a.
- **D11** Compose: `compose::add iii-sandbox` is rewritten before planning, so `Need` is `{VmBundle, VmLocal, Explicit}`. `/resolve` `unsupported_platform` maps to `UNSUPPORTED_PLATFORM`. A generated adapter is removed when an update or remove edit makes `needs(file)` go from non-empty to empty. `BundleNeedsAVm` stays as the Windows guard. Attached projects use `DEFAULT_ADAPTER` and never query `adapter::info`. No persisted log cursor. Host roots are the compose dir and the package cache only. `iii compose build` also writes the adapter.
- **D12** `iii worker` has been rejected by clap since 0.23, so no pointer is added. `iii update` notes leftovers and deletes nothing. The `libcap-ng-dev` installs leave `ci.yml`; `_rust-binary.yml` keeps its generic inputs; docs are edited in `docs/next` only; `.config/nextest.toml` stays.
- **D13** The ide e2e binds `sandbox: {adapter: sandbox-mock, namespace: ide-e2e}` and stays standalone; `run-tests.sh` stops setting `III_SANDBOX_FS_NAMESPACE`, while ide's support for it stays. The isolation e2e runs on the script's own engine with an explicit binding. Only text that tells users to configure `engine.workers.iii-sandbox` (or iii-sandbox's own configuration) changes. `template/sync.sh` exempts `sandbox-builtin`. The template declares `sandbox-builtin` as an operator container (no marker), so `up` on a host with no adapter build fails with `UNSUPPORTED_PLATFORM`; its README says to remove the container there. Harness launchers that call `iii worker` or `worker::add` are out of scope.
- **D14** A name goes in the registry only when another plan uses it.

## Adversarial plan review (A1)

An adversarial review of the plans confirmed 46 findings, and all of them are applied. The spec marks its changes `[Designer, adversarial review]` (row A1); the registry cites `(A1)`. Where findings overlapped, these choices won:

- **C1** (iii-code-1, sequencing-1, spec-fidelity-3, executability-1) P2a Task 4 deletes the "Sandbox isolation end to end" step and its log upload from iii's `sandbox-isolation-linux`: once `sandbox::*` is reserved, the old daemon cannot register and the step can only fail. P3 Task 4 deletes the rest of the job, located by text.
- **C2** (workers-code-1, executability-2, spec-fidelity-8) P1's rust job runs `BUILD_TARGET=x86_64-unknown-linux-gnu scripts/build-init.sh` before the sandbox-builtin clippy and test steps (Task 13 Step 1 too), keeps `--features embed-init,embed-libkrunfw`, and adds a featureless `cargo test --locked --lib fails_closed`.
- **C3** (workers-code-2) P1 Task 1 gates `vm_boot_subprocess_receives_correct_rootfs_argument` with `#[cfg(not(feature = "embed-init"))]`; Task 7 Step 4, Task 13 and the vm-linux job run the same command.
- **C4** (workers-code-5, sequencing-4, spec-fidelity-4, executability-4) P1's copied error-code test reads `rust_variant: Option<String>`, skips an explicit `ROUTER_ONLY` list (S301-S304) and asserts `representative()` is None exactly for those, and asserts the iii.dev `docs_base`. Registry §8: P1 adds S501-S506 and copies both fixtures.
- **C5** (security-failure-5, sequencing-2) P3 Task 2 keeps `install.sh`'s companion download, keyed off the resolved release's assets and silent when they are absent: a pinned 0.24 install still gets `iii-worker` and `iii-init`, and a 0.25 install fetches and prints nothing. No workers PR has to land before P3.
- **C6** (sequencing-3, sequencing-6) P1 merges after P0 and copies its merged fixtures from iii main. P2a merges once `sandbox-builtin` 1.0.0 resolves for the three targets and P2b is approved, so P2a, P2b and P3 land back to back.
- **C7** (spec-fidelity-7) `Owner`, `WorkerStartRequest.owner` and `WorkerStartRequest.network` are dropped from the spec, the registry, P0, P1 and P2b.
- **C8** (iii-code-5) The 404 of 0.24.x `docs_url` links is accepted in P2a's PR note and the upgrade guide; no stub README.
- **C9** (spec-fidelity-5, executability-6, spec-fidelity-6) Cheap, test-related optional parts are applied: a source-sync check in `worker_vm_e2e`, two pure Compose helpers with unit tests, and the extended Compose e2e.

The other findings, by plan:

- **P0:** where `iii-skill-render` comes from, and the fallback without it (executability-8).
- **P1:** keep `src/core/{mod,error,types}.rs` (workers-code-4) and `source_watcher` (spec-fidelity-5); the VM share is `<base>/<vm_id>/root`, with `state.json` and `console.log` beside it, opened with O_NOFOLLOW and mode 0600 (workers-code-3); `console.log` rotates at 10 MiB (security-failure-8); `start` clears its `Preparing` entry on any error, and S502/S503 carry the live `{vm_id, instance}` in `fix` (security-failure-4); S505 is tested on every `worker::*` op before any lookup (security-failure-3); interface-smoke starts the worker with `III_ENGINE_PID` (executability-3); `wire.rs` derives Debug, Clone, Serialize, Deserialize, JsonSchema on every type (workers-code-7, executability-9); Task 5 Step 2 runs without `--locked` (executability-7); vm-linux diffs both fixtures against iii main (sequencing-5); the README states the trusted-bus assumption (security-failure-2).
- **P2a:** `is_iii_builtin_function_id` counts `sandbox::` (iii-code-2); the publish workflow's heredoc runs once against a real boot listing (executability-10).
- **P2b:** `allow_remove` limits the adapter's removal to `update` and `remove` (spec-fidelity-1); the replacement YAML has no marker (spec-fidelity-2); `adapter_env` sets `III_DISABLE_TRACE_PAYLOADS=1` (security-failure-1); S502/S503 stop the live instance and retry once, and a bus error on stop is retried for up to 60 s, then reported as a warning, never taken as stopped at once (security-failure-4); a closed log channel is re-subscribed from the last cursor while the VM runs (security-failure-6); `process/unix.rs:256-262` and `project.rs:1412` handle the new types (iii-code-4); the destroy warning names the container, the adapter and the vm_id (security-failure-7); the constraints state the trusted-bus assumption (security-failure-2); Task 1 Step 2 runs without `--locked` (executability-7).
- **P3:** the upgrade guide covers the first `iii update` from 0.24 (iii-code-3), both orphaned-upper cases (security-failure-7) and the 0.24 `docs_url` links (C8); Task 5 Step 1 checks the moved paths with `git log` (sequencing-7); `iii-skill-render` source and fallback (executability-8).
- **Index and registry:** release-notes item for the first `iii update` (iii-code-3), the port rule for moved paths (sequencing-7), required-check timing (sequencing-8), floating images (sequencing-9), fixture-drift wording (sequencing-5), the `III_VM_*` note (spec-fidelity-9).

## Known limits carried into v1

- `sandbox-builtin`'s copies of the wire types can drift from the contract, and the compiler does not catch it (Q5). The two fixtures do: `sandbox-builtin-vm-linux`, which gates every adapter release, diffs its copies against iii main's. The pinned Compose e2e covers only iii against 1.0.0.
- 0.24.x error `docs_url` links point at the GitHub README that P2a moves, so they stop resolving; the S-codes live at `https://iii.dev/docs/creating-workers/sandboxes` (P2a PR note, upgrade guide; no stub README).
- iii's required `compose-sandbox-e2e-linux` boots floating images: `iiidev/node:latest` (onboarding's `base_image`, P2b's fixture) and the `python` preset `iiidev/python:latest`. After P3, only a `sandbox-builtin-images.yml` dispatch republishes them, so dispatch it only with its smoke test green, and re-run iii's job after a push.
- The router's forward is a fresh invocation: a consumer's timeout or cancellation does not stop adapter-side work (spec 12).
- A config change to `sandbox-builtin` applies on its next restart, which cycles every sandbox and VM container (spec 8.3).
- A stored `iii-sandbox` configuration entry is not migrated; the upgrade guide says how to re-enter it (spec 7.9).
- From the 0.25.0 release until P5 merges, the workers `template/worker-compose.yaml` still declares `engine.workers.iii-sandbox`, so its `up` on 0.25 fails with `SANDBOX_CONFIG_MOVED`; start P5 right after the release (rollout step 4). The repo's CI keeps its 0.24 pin until P5, and `install.sh` still gives that pin its companions (C5).
- The docs anchor test runs only when CI runs; `ci.yml` ignores docs-only changes, so a docs-only PR that drops an `#Sxxx` anchor is not caught.
