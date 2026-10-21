# Sandbox worker: plan index

Index of the six implementation plans for `docs/superpowers/specs/2026-10-20-sandbox-worker-design.md`. Every plan uses the names in `2026-10-20-sandbox-shared-interfaces.md`. Read the spec, the registry and the plan before starting a task.

## Plans and order

| Order | Plan | Repo | Branch | Merges when |
|---|---|---|---|---|
| 1 | [P0 contract crate](2026-10-20-sandbox-0-contract.md) | iii | `sandbox/p0-contract` | any time (internal crate, no behaviour change) |
| 2 | [P1 sandbox-builtin](2026-10-20-sandbox-1-builtin-worker.md) | workers | `sandbox/p1-builtin` | any time; it copies P0's two fixtures and does not depend on the iii repo. Release Control then publishes `sandbox-builtin` 1.0.0 |
| 3 | [P2a engine router](2026-10-20-sandbox-2a-engine-router.md) | iii | `sandbox/p2a-router` | after P0. From this merge on, no stable tag from main until 0.25.0 |
| 4 | [P2b Compose](2026-10-20-sandbox-2b-compose.md) | iii | `sandbox/p2b-compose` | after P2a, with `sandbox-builtin` 1.0.0 in the registry (its e2e pulls it) |
| 5 | [P3 engine cleanup](2026-10-20-sandbox-3-engine-cleanup.md) | iii | `sandbox/p3-engine-cleanup` | after P2b, once its Task 7 registry gate passes, right before the 0.25.0 tag (`install.iii.dev` serves main's `install.sh`) |
| 6 | Release engine `iii/v0.25.0` (rollout step 4) | iii | | after P3; checklist below |
| 7 | [P5 consumers](2026-10-20-sandbox-5-consumers.md) | workers | `sandbox/p5-consumers` | after 0.25.0 ships (its CI pins move to `iii/v0.25.0`) |

All PRs can be developed in parallel; only the merges follow this order. Nothing is published to crates.io (spec Q5): P1 keeps its own copies of the wire types and round-trips P0's `sandbox_error_codes.json` and `wire_examples.json`. If P0 changes either fixture before merging, re-copy it into P1.

## Before each PR

- Create the MOT ticket in Linear first and put it in the PR title.
- Conventional commit messages. No AI co-author trailer and no "Generated with" line, in commits or PR bodies.

## Operational prerequisites

1. **Docker Hub:** the workers repo has `DOCKERHUB_USERNAME` and `DOCKERHUB_PASSWORD`. Do not dispatch `sandbox-builtin-images.yml` until P3 deletes iii's `docker-worker-images.yml` (D7).
2. **Branch protection (D5):** make `sandbox-builtin-vm-linux` a required check in the workers repo (it gates registry releases) and `compose-sandbox-e2e-linux` a required check in iii.
3. **Registry state:** `sandbox-builtin` 1.0.0 published for `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` before P2b's e2e and before P3 merges; the `onboarding` bundle at the version P2b's e2e pins (0.1.20) is published.
4. **Release runner:** the workers interface-capture runners execute the linux-gnu adapter, which links `libcap-ng0`; confirm it is installed on the release runner at the first release.

## Rollout step 4: release engine 0.25.0

- [ ] P2a, P2b and P3 are merged; P3's Task 7 registry gate passed.
- [ ] Cut `iii/v0.25.0` with `create-tag.yml`.
- [ ] `gh release view iii/v0.25.0 --repo iii-hq/iii --json assets --jq '.assets[].name' | grep -E '^(iii-worker|iii-init|libkrunfw)-'` prints nothing (P3 Task 7 Step 4).
- [ ] The release promotes `docs/next/` to Latest: `https://iii.dev/docs/upgrading/sandbox-worker` and the `#Sxxx` anchors on `https://iii.dev/docs/creating-workers/sandboxes` resolve.
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

## Known limits carried into v1

- `sandbox-builtin`'s copies of the wire types can drift from the contract. The two fixtures and the Compose e2e catch it, not the compiler (Q5).
- The router's forward is a fresh invocation: a consumer's timeout or cancellation does not stop adapter-side work (spec 12).
- A config change to `sandbox-builtin` applies on its next restart, which cycles every sandbox and VM container (spec 8.3).
- A stored `iii-sandbox` configuration entry is not migrated; the upgrade guide says how to re-enter it (spec 7.9).
- The docs anchor test runs only when CI runs; `ci.yml` ignores docs-only changes, so a docs-only PR that drops an `#Sxxx` anchor is not caught.
