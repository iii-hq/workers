# Engine Cleanup (VM substrate removal) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove every VM crate, firmware blob, script, CI/release job, installer path and `iii update` entry from the iii repo, and document the move to `sandbox-builtin`.

**Architecture:** Mostly deletion. First the users of the VM artifacts stop using them (`iii update`, `engine/install.sh`, release workflows, CI and Makefile); then the seven crates, firmware and scripts leave the workspace, and with them the `libcap-ng-dev` installs; then `docs/next/` and the top-level skills describe `engine.sandbox` and `sandbox-builtin`, with a new upgrade guide. That order keeps main green after every commit.

**Tech Stack:** Rust (engine CLI in `engine/src/cli`), POSIX sh + bats + shellcheck (installer), GitHub Actions YAML (actionlint, `.github/scripts` pytest), GNU Make, Mintlify MDX (`iii-skill-render`).

**Spec:** docs/superpowers/specs/2026-10-20-sandbox-worker-design.md, sections 2 (non-goals the docs state), 6.5 (the parts P2a does not own), 7.8-7.9 (upgrade-guide notes), 8.2 (moved files leave iii), 9 (user step 1, iii-repo docs), 10 (step 3), 11 (release check). Shared names: docs/superpowers/plans/2026-10-20-sandbox-shared-interfaces.md

**Repo and base:** iii repo. Citations are line numbers at `a1d08d8b0`; re-locate any line P0, P2a or P2b moved. Branch `sandbox/p3-engine-cleanup` from main after P2b merges. Prerequisites: P0, P2a and P2b merged. **This PR cannot merge before `sandbox-builtin` 1.0.0 is in the registry (P1); Task 7 holds the gate.**

## Global Constraints

- "Never release an engine without `iii-worker` before `sandbox-builtin` is in the registry." / "Steps 2 and 3 are separate PRs in one engine release." (10)
- "`.cargo/config.toml` stays." (6.5)
- "`publish-builtin-workers` keeps `iii-sandbox` with `worker_dir: engine/src/workers/sandbox`." (6.5; P2a set the row)
- "`engine-release` (:398-413) stays." (6.5, `alpha-release.yml`)
- "the SDK, lint, fmt, typecheck and console targets (:137-190) stay." (6.5, `Makefile`)
- Kept on purpose: `.config/nextest.toml` (D12); P2b's `compose-sandbox-e2e-linux` job (D5); `BUNDLE_NEEDS_A_VM` as the Windows guard (D11); P0's `<a id="S...">` anchors in `docs/next/creating-workers/sandboxes.mdx`; `_rust-binary.yml`'s generic inputs `features`, `targets`, `system_deps` (D12).
- `iii update` notes the leftover binaries and deletes nothing; no `iii worker` pointer, since clap has rejected `iii worker` since 0.23 (`engine/src/main.rs:680-686`) (D12). The `libcap-ng-dev` installs left in `ci.yml` go once `cargo metadata` lists no `capng` (D12).
- Release check: "The 0.25.0 GitHub release has no `iii-worker-*`, `iii-init-*` or `libkrunfw-*` assets." / "`cargo metadata` for the iii workspace lists no `msb_krun` package." / "`engine/install.sh` and `iii update` fetch neither `iii-worker` nor `iii-init`." (11)
- Anti-goal: "The engine depending on a workers-repo artifact at build time." (2)
- Non-goals the docs must state: "Musl hosts at launch", "Intel Mac and Windows hosts". (2)
- Docs: author in `docs/next/` only (D12), regenerate each `.skill.md` with `iii-skill-render --write`, no em dashes in prose (repo `AGENTS.md:62-76`).
- Commits: conventional, MOT ticket in the PR title, never an AI co-author or "Generated with" line.

## Review Focus

1. A 0.24 user with `~/.local/bin/iii-worker` runs `iii update`: the file is not updated or downloaded, and one note names it for deletion. Pin: `cli::update::tests::retired_binaries_lists_leftover_vm_binaries` (Task 1).
2. `iii update iii-worker` (or `worker`, `iii-init`): non-zero exit with the existing unknown-command error and `--list-targets` hint, never a download. Pin: `cli::registry::tests::vm_binaries_are_no_longer_managed` (Task 1).
3. Installing on Alpine or an Intel Mac: `iii` installs, exit 0, and no `iii-worker`/`iii-init` download, warning or note appears. Pin: bats `install.sh downloads no VM companion binaries` (Task 2).
4. A 0.25 error's `docs_url` (`DOCS_BASE` + code) after the 0.25.0 rotation promotes `docs/next/` to Latest: the anchor still exists in the rewritten page. Pin: P0's `every_code_has_a_docs_anchor`, run in Task 6 Step 4.
5. A release run after the jobs go: `notify-complete` must not turn red from an empty `*_RESULT` of a deleted job, and no `needs:` names a deleted job. Pin: the `rg` + `actionlint` check in Task 3 Step 4.

---

### Task 1: `iii update` stops managing `iii-worker` and `iii-init`

**Files:**
- Modify: `engine/src/cli/registry.rs:47-61` (delete `iii-init` spec), `:100-117` (delete `iii-worker` spec), tests module
- Modify: `engine/src/cli/platform.rs:54-73` (delete `binary_target`, `linux_worker_target`), `:90`, `:158`, `:215` (call `current_target()`), `:243-283` (delete the three worker/libc tests)
- Modify: `engine/src/cli/update.rs:256`, `:366` (`platform::current_target()`), `:430` (drop the `iii-init` example), tests module
- Modify: `engine/src/cli/mod.rs:208-212` (`handle_update` prints the note once, after `ensure_dirs`)
- Test: `engine/src/cli/registry.rs`, `engine/src/cli/update.rs` (unit tests in the `iii` bin)

**Interfaces:**
- Consumes: none cross-plan.
- Produces (P3-internal, `engine/src/cli/update.rs`): `fn retired_binaries() -> Vec<std::path::PathBuf>`, `pub fn print_retired_binaries_note()`.

- [ ] **Step 1: Write the failing tests**
```rust
// engine/src/cli/registry.rs, mod tests
#[test]
fn vm_binaries_are_no_longer_managed() {
    for target in ["iii-worker", "iii-init", "worker"] {
        assert!(resolve_binary_for_update(target).is_err(), "{target}");
    }
}
// engine/src/cli/update.rs, mod tests; HOME swap and restore exactly as
// platform.rs test_ensure_dirs_creates_bin_dir (:311-349) does:
// collect both results, restore HOME, then assert
#[cfg(not(target_os = "windows"))]
#[test]
#[serial_test::serial]
fn retired_binaries_lists_leftover_vm_binaries() {
    // HOME = tempdir; bin = HOME/.local/bin, created empty
    let before = retired_binaries();
    std::fs::write(bin.join("iii-worker"), "").unwrap();
    let after = retired_binaries();
    // restore HOME here
    assert!(before.is_empty());
    assert_eq!(after, vec![bin.join("iii-worker")]);
}
```
- [ ] **Step 2: Run them to verify they fail**
`cargo test -p iii --bin iii -- cli::registry cli::update` → compile error E0425 (no function `retired_binaries`); with that test commented out, `vm_binaries_are_no_longer_managed` panics on `iii-worker`.
- [ ] **Step 3: Implement**
Delete both `BinarySpec`s and the libc choice; every `binary_target(x)` caller becomes `current_target()`. `retired_binaries()` returns `platform::binary_path(name)` for `"iii-worker"` and `"iii-init"`, kept when the file exists. `print_retired_binaries_note()` prints nothing for an empty list, else (styled like the restart note at `cli/mod.rs:252-256`):
```text
  note: iii-worker and iii-init are no longer used: sandboxes and VM workers run in the sandbox-builtin worker, which iii compose installs. You can delete:
    <one path per line>
```
`handle_update` calls it once, right after `ensure_dirs` and before the target match, so every target prints it, an unknown one included. Nothing is deleted (spec 9 step 1, D12).
- [ ] **Step 4: Run the tests to verify they pass**
`cargo test -p iii --bin iii -- cli::` → `test result: ok`. `cargo build -p iii 2>&1 | grep -E 'never used|unused'` → no output.
- [ ] **Step 5: Commit**
`git add engine/src/cli/registry.rs engine/src/cli/platform.rs engine/src/cli/update.rs engine/src/cli/mod.rs`
`git commit -m "feat(cli): stop installing iii-worker and iii-init from iii update"`

### Task 2: `engine/install.sh` stops downloading the VM companions

**Files:**
- Modify: `engine/install.sh:11-12` (two `*_install_failed` vars), `:168-222` (`resolve_companion_url`, `install_companion_from_tarball`, `worker_target_for_host`: all dead afterwards), `:535` (comment: "exclude .sha256 and other binaries' assets"), `:622-652` and `:654-677` (keep `main_tarball=` at `:653`), `:679-682` (comment becomes "Download the main binary."), `:686-687`, `:700-725`, `:726` (comment becomes "Validate: the main binary is critical."; keep `:727-729`), `:730-739`, `:798-815`, `:842-864` (keep the `fi` at `:865`)
- Modify: `engine/tests/install_sh_unit.bats:217-264` (delete the seven `worker_target_for_host` cases), `engine/tests/install_sh_integration.sh:6`
- Test: `engine/tests/install_sh_unit.bats`

**Interfaces:**
- Consumes: none. Produces: none.

- [ ] **Step 1: Write the failing test** (in place of the deleted cases)
```bash
@test "install.sh downloads no VM companion binaries" {
  run grep -nE 'iii-worker|iii-init|libkrunfw|worker_target_for_host' "$INSTALL_SH"
  [ "$status" -eq 1 ]
}
```
- [ ] **Step 2: Run it to verify it fails**
`bats engine/tests/install_sh_unit.bats` → `not ok ... install.sh downloads no VM companion binaries`.
- [ ] **Step 3: Delete the listed lines** (the main-binary download, validation, extract, install, telemetry and PATH guidance stay byte-for-byte)
- [ ] **Step 4: Run the tests to verify they pass**
`bats engine/tests/install_sh_unit.bats` → all `ok`. `shellcheck -s sh engine/install.sh` → no output. `sh -n engine/install.sh` → exit 0.
- [ ] **Step 5: Commit**
`git add engine/install.sh engine/tests/install_sh_unit.bats engine/tests/install_sh_integration.sh`
`git commit -m "feat(install): stop downloading iii-worker and iii-init"`

### Task 3: Release workflows stop building `iii-init`, `iii-worker` and firmware

**Files:**
- Modify: `.github/workflows/release-iii.yml:125-210` (`init-build` + header), `:211-279` (`firmware-upload` + header), `:305-333` (`worker-release` + header), `:650-652` (`needs: [setup, create-iii-release, engine-release]`, drop the comment), `:701`, `:755`, `:840-841`, `:843`, `:870-871`, `:873`, `:903-904`, `:906`, `:928` (drop `INIT_RESULT`, `FIRMWARE_RESULT`, `WORKER_RESULT` from the loop)
- Modify: `.github/workflows/alpha-release.yml:10-11` (binaries: `iii` only), `:333-397` (`init-build`), `:415-435` (`worker-release`), `:438` (drop `worker-release` from `needs`)
- Modify: `.github/workflows/_rust-binary.yml:48-52` (`init_artifacts` input), `:221-233` (two `iii-init` downloads), `:293-309` (musl worker build and upload), `:311` (drop the `if:`; the upload step always runs)
- Modify: `.github/workflows/WORKFLOWS.md:43`, `:203` (alpha ships `iii` only), `:61` (`engine-coverage` tests `iii` and `iii-sandbox-contract` with `--all-features`, as `ci.yml` does after P0); `.github/workflows/_publish-engine-workers.yml:66-67`, `:230-233` (comments that say install bundles `iii-worker` / `iii-sandbox` spawns it; re-locate `:230-233` by text, since P2a Task 7 inserts lines at `:151-162`)

**Interfaces:**
- Consumes: P2a's matrix row `- worker: iii-sandbox` / `worker_dir: engine/src/workers/sandbox` in both files and its `WORKFLOWS.md:381` edit (left as is).
- Produces: none.

- [ ] **Step 1: Write the failing check**
`rg -n 'iii-init|init-build|firmware-upload|worker-release|libkrunfw|INIT_RESULT|FIRMWARE_RESULT|WORKER_RESULT|init_artifacts|build-worker-musl|iii-worker([^-s]|$)' .github/workflows/{release-iii,alpha-release,_rust-binary,_publish-engine-workers}.yml .github/workflows/WORKFLOWS.md`
- [ ] **Step 2: Run it to verify it fails** → dozens of hits.
- [ ] **Step 3: Delete and edit the listed lines** (no other job, input or matrix row changes)
- [ ] **Step 4: Run the checks to verify they pass**
The Step 1 `rg` → no output. `actionlint .github/workflows/release-iii.yml .github/workflows/alpha-release.yml .github/workflows/_rust-binary.yml` → no output. `python3 .github/scripts/validate_release_worker_matrix.py --repo-root .` → `::notice::release worker matrix validation passed`. `(cd .github/scripts && python3 -m pytest -q)` → `passed`, no failures.
- [ ] **Step 5: Commit**
`git add .github/workflows/release-iii.yml .github/workflows/alpha-release.yml .github/workflows/_rust-binary.yml .github/workflows/WORKFLOWS.md .github/workflows/_publish-engine-workers.yml`
`git commit -m "ci(release): stop building iii-init, iii-worker and firmware assets"`

### Task 4: CI and Makefile stop building and testing the VM crates

**Files:**
- Modify: `.github/workflows/ci.yml:65-87` (`worker-musl`; the Engine Build header at `:61-63` stays), `:163-169` (iii-worker install step), `:180-182` (drop the VM-crate sentence) and `:186` (`cargo test ... -p iii-worker ...` line; P0's `-p iii -p iii-sandbox-contract` line after it stays), `:351-357` (compose libcap step "through iii-worker"; `iii-compose` never links cap-ng), `:407-759` (`worker-build-matrix`, `worker-test-matrix`, `worker-test-{vm,oci}-{linux,macos}`, `sandbox-isolation-{linux,macos}` with their headers). P2b's `compose-sandbox-e2e-linux` stays (D5).
- Delete: `.github/workflows/docker-worker-images.yml` (D7)
- Modify: `Makefile:12` (`LOCAL_BIN`, now unused), `:16-20` (`.PHONY`), `:50-54` (delete `install-iii-worker`), `:56-67` (`engine-test`/`coverage` lose the prerequisite and the `cargo test -p iii-worker ...` line and keep P0's `-p iii -p iii-sandbox-contract`; comment says it mirrors Engine Coverage), `:86-134` (delete), `:191-199` (`build: build-console` running `cargo build --release -p iii`; drop the worker echo lines; PATH line is `$(CURDIR)/target/release` only)

**Interfaces:**
- Consumes: P2b's `compose-sandbox-e2e-linux` job (kept). Produces: none.

- [ ] **Step 1: Write the failing check**
`rg -n 'iii-worker([^-s]|$)|iii-init|iii-filesystem|iii-network|sandbox-isolation|build-worker-musl|WORKER_TARGET' .github/workflows/ci.yml Makefile` (no `libkrun`: P2b's job keeps the KVM step's warning text)
- [ ] **Step 2: Run it to verify it fails** → hits in both files.
- [ ] **Step 3: Delete and edit the listed lines** (the other `libcap-ng-dev` installs go in Task 5)
- [ ] **Step 4: Run the checks to verify they pass**
The `rg` → no output. `rg -c '^  compose-sandbox-e2e-linux:' .github/workflows/ci.yml` → `1`. `actionlint .github/workflows/ci.yml` → no output. `make engine-test` → `test result: ok` for every package.
- [ ] **Step 5: Commit**
`git add .github/workflows/ci.yml .github/workflows/docker-worker-images.yml Makefile`
`git commit -m "ci: drop the iii-worker build, test and image jobs"`

### Task 5: Delete the VM crates, firmware and scripts

**Files:**
- Delete: `crates/{iii-worker,iii-init,iii-network,iii-filesystem,iii-supervisor,iii-shell-proto,iii-shell-client}/`, `engine/firmware/`, `scripts/{build-worker-musl.sh,worker-musl.md,sandbox-isolation-e2e.sh}`, `engine/tests/vm_platform_e2e.rs` (P1 copies the test, the isolation script and the firmware; D8 drops the musl script and note)
- Modify: `Cargo.toml:13-19` (members), `Cargo.lock` (pruned, never upgraded), `.gitignore:92-97`
- Modify: `.github/scripts/test_validate_rust_release_version.bats:15-19` (workspace-inherited fixture becomes `crates/scaffolder-core/Cargo.toml`, package `scaffolder-core`)
- Modify: `engine/src/workers/configuration/store.rs:49` (the comment names `crates/iii-worker/src/cli/config_file.rs`; reword without the path), `crates/iii-compose/src/lib.rs:12` and `crates/iii-compose/src/manifest.rs:12` (module docs name `crates/iii-worker`; say "the legacy worker CLI" instead). Step 4's path `rg` fails without these three
- Modify: `.github/workflows/ci.yml`, the `libcap-ng-dev` installs left after Task 4 (D12): delete the steps at `:100-106` (`engine-build`), `:227-233` (`engine-benches`, whose workspace-wide `cargo bench` compiled cap-ng until now) and `:277-283` (`cli-docs-built`); `:160-161` (`engine-coverage`) installs `lld` only

**Interfaces:**
- Consumes: P2a removed `KNOWN_EXTERNAL` (`engine/src/workers/external.rs`) and the `__pull-images` prefetch (`engine/src/cli/project/mod.rs`); P2b removed `vm_command`, `prepare_vm`, `worker_binary` (`crates/iii-compose/src/lifecycle.rs:1199-1355`) and every exec of `iii-worker` in `crates/iii-compose/src`. Nothing may still exec `iii-worker`.
- Produces: a workspace without `msb_krun` or `capng` (spec 11, D12).

- [ ] **Step 1: Write the failing checks**
Precondition: `rg -n '"iii-worker"|iii-worker\{|find_existing_binary\("iii-worker' engine/src crates/iii-compose/src -g '!**/observability/**'` → no output (observability keeps `"iii-worker"` as a test scope name) (else stop: P2a/P2b are not on main).
Check: `cargo metadata --format-version 1 | jq -e '[.packages[].name | select(test("^(capng|msb_krun|iii-(worker|init|network|filesystem|supervisor|shell-proto|shell-client))$"))] | length == 0'`
- [ ] **Step 2: Run it to verify it fails** → `false`.
- [ ] **Step 3: Delete** with `git rm -r`, edit the members and the comment, run `cargo metadata --format-version 1 >/dev/null` to prune `Cargo.lock`, then delete the `ci.yml` installs listed above.
- [ ] **Step 4: Run the checks to verify they pass**
Step 1 check with `--locked` → `true`. `git diff Cargo.lock | grep '^+version'` → no output (packages only removed). `cargo test --locked -p iii --all-features` and `cargo test --locked -p iii-compose` → ok. `bats .github/scripts/test_validate_rust_release_version.bats` → 3 `ok`. `cargo fmt --all -- --check` → no output. `rg -n 'libcap-ng-dev' .github/workflows/ci.yml` → no output; `actionlint .github/workflows/ci.yml` → no output. `rg -n --hidden -g '!.git' -g '!Cargo.lock' -g '!docs/**' -g '!website/**' -g '!tech-specs/**' 'crates/iii-(worker|init|network|filesystem|supervisor|shell-proto|shell-client)|engine/firmware|build-worker-musl|sandbox-isolation-e2e|vm_platform_e2e'` → no output. `ls .cargo/config.toml .config/nextest.toml` → both present.
- [ ] **Step 5: Commit**
`git add -A crates engine/firmware scripts engine/tests/vm_platform_e2e.rs Cargo.toml Cargo.lock .gitignore .github/scripts/test_validate_rust_release_version.bats .github/workflows/ci.yml engine/src/workers/configuration/store.rs` (`crates` covers the two `iii-compose` comments)
`git commit -m "refactor(sandbox): delete the VM crates, firmware and scripts moved to sandbox-builtin"`

### Task 6: Docs and skills describe `engine.sandbox` and `sandbox-builtin`

**Files:**
- Create: `docs/next/upgrading/sandbox-worker.mdx` (+ rendered `.mdx.skill.md`)
- Modify: `docs/docs.json:92-93` (add `"next/upgrading/sandbox-worker"` to the Next "Upgrading" group)
- Modify: `docs/next/creating-workers/sandboxes.mdx:8-23`, `docs/next/creating-workers/observability.mdx:96-103`, `docs/next/using-iii/engine.mdx:27-33`, `docs/next/using-iii/compose.mdx:674-675`, `:733-739` (`:741-743`, Windows and `BUNDLE_NEEDS_A_VM`, stay: D11), `docs/next/using-iii/workers.mdx:114-115`, `docs/next/upgrading/workers-to-compose.mdx:30`, `:35`, `:71`, `:80`, `:103-104`, and each page's `.mdx.skill.md`. `docs/next/cli-reference/index.mdx` needs no edit (no clap text changes; Task 7 Step 1 proves it).
- Modify: `skills/iii-engine-config/SKILL.md:36-37`, `:60`; `skills/iii-core-primitives/SKILL.md:50`

**Interfaces:**
- Consumes: P0's `<a id="Sxxx"></a>` anchors in `docs/next/creating-workers/sandboxes.mdx` and its test `every_code_has_a_docs_anchor`; P2a `SandboxRouterConfig { adapter, namespace }` (top-level `sandbox:` in an engine config) and the S303 text; P2b `SandboxConfigMoved` replacement YAML, `SANDBOX_ADAPTER_NOT_DECLARED`, `UNSUPPORTED_PLATFORM`, the kept `BundleNeedsAVm`, `engine.sandbox`, the two host roots (registry 5); P1 targets `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` (spec 8.5) and config keys (registry 6).
- Produces: the page behind `https://iii.dev/docs/upgrading/sandbox-worker` once 0.25.0 rotates `docs/next/` to Latest.

- [ ] **Step 1: Write the failing checks**
`rg -n 'iii-sandbox:|engine\.workers\.iii-sandbox|iii-worker([^-s"]|$)|iii-init' docs/next skills -g '!**/sandbox-worker.mdx*'`; `test -f docs/next/upgrading/sandbox-worker.mdx`
- [ ] **Step 2: Run them to verify they fail** → hits in the pages above; the file is missing.
- [ ] **Step 3: Edit the pages** (decisions, not prose):
  - `sandboxes.mdx`: the `iii-sandbox` router serves `sandbox::*`; the `sandbox-builtin` worker runs the VMs and Compose installs it (`engine.sandbox`, a VM container, or `compose::add iii-sandbox`); allowlist and images go under `containers.sandbox-builtin.config_override`; link the upgrade guide; keep P0's `## Error codes` section with every `<a id="S...">` anchor.
  - `observability.mdx`: the example becomes `engine: { sandbox: { adapter: sandbox-builtin } }`, which makes Compose install the adapter.
  - `engine.mdx`, `compose.mdx:674`, `workers.mdx:114`, `workers-to-compose.mdx:30,35,71,80,103`, `skills/iii-engine-config`: `iii-sandbox` leaves the `engine.workers` lists (four workers remain); an engine no Compose file manages sets top-level `sandbox: {adapter, namespace}` or its calls get S303.
  - `skills/iii-core-primitives/SKILL.md:50`: `iii-sandbox` stays engine-owned; the `sandbox-builtin` adapter is a registry worker that Compose installs.
  - `compose.mdx:733-739`: bundle and `runtime.base_image` VMs run in `sandbox-builtin`, built for the three P1 targets only; the Linux user needs read/write on `/dev/kvm` and `libcap-ng0`; musl and Intel Mac hosts fail with `UNSUPPORTED_PLATFORM`.
  - `sandbox-worker.mdx` sections: what moved; replace `engine.workers.iii-sandbox` with the YAML `SANDBOX_CONFIG_MOVED` prints; re-enter a stored entry with `iii trigger configuration::get --json '{"id": "iii-sandbox", "raw": true}'`, then `configuration::set` with `id: <namespace>-sandbox-builtin` (Compose's `<namespace>-<key>` config name); External-mode engines set `sandbox:`; `--frozen` refuses with `SANDBOX_ADAPTER_NOT_DECLARED` (run `up` once without it); a `path://` VM worker must live under the compose dir or the package cache, else S501 names the roots (7.8); delete the old VM state `<state dir>/vm/<key>/` (`<compose-dir>/.iii/compose/<ns>/`, or under `$III_COMPOSE_STATE_DIR`; first boot reinstalls dependencies); delete `~/.local/bin/iii-worker` and `~/.local/bin/iii-init`; supported hosts as above, with musl, Intel Mac and Windows hosts unsupported (2).
  Then `iii-skill-render --write docs/next/<page>.mdx` for each edited or created page.
- [ ] **Step 4: Run the checks to verify they pass**
Step 1 `rg` → no output; file exists. `cargo test --locked -p iii-sandbox-contract --test wire_parity every_code_has_a_docs_anchor` → `1 passed`. `python3 .github/scripts/pin_docs.py validate --docs-dir docs` → `pin_docs: Next docs ready`. `git status --porcelain docs/next | grep -c '\.skill\.md'` → `7` (six edited pages and the new one).
- [ ] **Step 5: Commit**
`git add docs/docs.json docs/next skills/iii-engine-config/SKILL.md skills/iii-core-primitives/SKILL.md`
`git commit -m "docs(sandbox): document engine.sandbox, sandbox-builtin and the 0.25 upgrade"`

### Task 7: Whole-plan verification and PR note

**Files:** none changed.

**Interfaces:**
- Consumes: P1 `sandbox-builtin` 1.0.0 published to `https://api.workers.iii.dev` for the three targets.

- [ ] **Step 1: Full suite for the touched packages**
`cargo fmt --all -- --check && cargo test --locked -p iii -p iii-sandbox-contract --all-features && cargo test --locked -p iii-compose` → ok.
`bats engine/tests/install_sh_unit.bats .github/scripts/test_validate_rust_release_version.bats && shellcheck -s sh engine/install.sh` → all `ok`, no shellcheck output.
`actionlint .github/workflows/{ci,release-iii,alpha-release,_rust-binary,_publish-engine-workers}.yml` → no output. `python3 .github/scripts/validate_release_worker_matrix.py --repo-root . && (cd .github/scripts && python3 -m pytest -q)` → passed.
`./scripts/generate-cli-docs.sh && git diff --exit-code docs/next/cli-reference` → exit 0 (Task 1 changed no clap text).
- [ ] **Step 2: Spec 11 release check, pre-merge form**
`cargo metadata --locked --format-version 1 | jq -e '[.packages[].name | select(startswith("msb_krun"))] | length == 0'` → `true`.
`rg -n 'iii-worker([^-s]|$)|iii-init|libkrunfw' .github/workflows/release-iii.yml .github/workflows/alpha-release.yml .github/workflows/_rust-binary.yml engine/install.sh engine/src/cli/registry.rs` → no output.
`cargo test -p iii --bin iii -- cli::registry::tests::vm_binaries_are_no_longer_managed` → ok.
- [ ] **Step 3: Merge gate (must pass before merge)**
```bash
for t in aarch64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
  curl -fsS https://api.workers.iii.dev/resolve -H 'content-type: application/json' \
    -d "{\"worker\":\"sandbox-builtin\",\"version\":\"^1\",\"target\":\"$t\"}" \
  | jq -e '.graph[] | select(.name == "sandbox-builtin") | .version | startswith("1.")'
done
```
Expected: `true` three times. Any failure blocks the merge.
- [ ] **Step 4: Post-release check (rollout step 4; listed here, run by the release owner)**
`gh release view iii/v0.25.0 --repo iii-hq/iii --json assets --jq '.assets[].name' | grep -E '^(iii-worker|iii-init|libkrunfw)-'` → no output.
- [ ] **Step 5: PR note** (do not push or open the PR from this plan)
Title: `refactor(sandbox): remove the VM substrate from the iii repo (MOT-<n>)`. Body: depends on P2a and P2b; merge only after Step 3 passes (paste its output), right before the 0.25.0 tag, because `https://install.iii.dev/iii/main/install.sh` serves main's installer, so any install (a pinned 0.24.x one too) gets no `iii-worker` once this merges; ships with step 2 in engine 0.25.0; never release an engine without `iii-worker` before `sandbox-builtin` is in the registry; `.cargo/config.toml` and `.config/nextest.toml` intentionally kept. No AI co-author or "Generated with" line.
