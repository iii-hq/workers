# Workers 0.5.0 integration candidate

## Pinned inputs and topology

- First parent (fetched main): `a2a801ceb0cca7269cc7473fadb1a8270365ee93`.
- Feature / workers#1690: `fcb5d188816c7b301f9de188c2e5777fc02a3f9f`.
- Second parent / workers#1765: `6c08fbacdc557d54992360eaff15526819414a4e`.
- Common ancestor: `0089495b938d77fb6f3ee0352b82e4b5a2e3de6d`.
- Hook ancestry: `fcb5d188` -> `faeef0773c0d6233fd0b7f324107835bb91210c4` -> `6c08fbac`.
- Owned branch: `fix/sandbox-050-main-reconcile`; worktree: `/home/doggao/iii/wt/workers.050-reconcile`.

One merge incorporates the final hook head, which already contains the feature.
No cherry-pick/reapplication of the intermediate isolated-shell implementation.
The final hook delta is **2 files, +375/-8**: local_worker.rs (+293/-8) and
HOOK-STATUS-FIX.md (+90), not +293 across both files.

Stable patch IDs (`git diff --binary A B | git patch-id --stable`):

| Range | Patch ID |
| --- | --- |
| common ancestor -> feature | `455487cab46ced16252a139393c4f876c13c690c` |
| feature -> final hook | `01d749c369c53ae0e3950f8670e1dfc84af15b51` |
| feature -> initial hook | `e3f7b1c84758e5c600f746394f47c044d756fe56` |
| initial -> shared-state hook | `7f7a431fd553102cafb2951d6b57921057968f72` |

## Resolution map

1. **Cargo.toml:** resolve 0.4.4 versus 0.5.0 to 0.5.0 and the contract-1.4
   comment/consumer range `^0.5`. Keep main's `serde_yaml_ng = "0.10"`.
2. **Cargo.lock:** keep main's complete dependency graph and checksums; change
   only the sandbox-libkrun package version to 0.5.0. No Cargo regeneration.
3. **Semantic overlap, local_worker.rs:** the feature introduced two
   `serde_yaml::` references in `validate_binary` after main migrated YAML.
   Use `serde_yaml_ng::{Value,from_str}` and rustfmt's wrapping. The same
   parser/type now feeds main's migrated `WorkerManifest::from_value`.
4. **Automatic merges:** keep main's YAML migration in bundle_download.rs,
   the existing local_worker.rs sites, and all other main paths. No wholesale
   ours/theirs selection. Contract fixtures, destroy-keeps-data, binary
   behavior, hook status/shared-state code and tests remain from final source.

## Checks actually performed

Pure Python/Git assertions verified all 18 feature/hook paths against their
source plus the explicit migration above, and 2,550 other main-changed paths
against main (including deletions). TOML parses; the lock graph equals main
except the package version. These are static checks, not Cargo validation.

Existing Rust 1.97.1 rustfmt `--edition 2024 --check` passed on local_worker.rs
and bundle_download.rs. The first check caught only the migrated expression's
line wrapping; it was corrected. Working/staged `git diff --check` passed.

No Cargo, compiler, VM, SDK/npm, provisioning, ELF/artifact read/copy/change,
release or remote mutation. Original worktrees, untracked smoke evidence and
artifact pins are untouched. Runtime executions: **0**.

## Minimum approval plan (not run)

Independent review must examine this merge's resolution map and final tree.
Under a separately granted heavy slot, existing Rust **1.97.1**, existing shared
**kan292-target**, jobs <=4, nice15/ionice-c3 and the coordinator's flock:

- `cargo +1.97.1 test --locked --offline -p sandbox-libkrun --lib --no-default-features a_binary_package_runs_its_executable_and_nothing_else -- --test-threads=4`
  must run its one selected test; it exercises the migrated binary manifest,
  install/env, name mismatch and setup rejection.
- `cargo +1.97.1 test --locked --offline -p sandbox-libkrun --no-default-features --test bundle_worker_integration -- --test-threads=4`
  covers the merged parser/manifest boundary. Missing cached dependencies or
  targets are a blocker, not permission to download or create a target tree.

Do not replay unchanged hook/destroy tests merely for this local reconciliation.
The prior 32-case Rust hook PASS/build evidence belongs to source 6c08, not this
new integrated commit. Preserve it without claiming a new compile/runtime PASS.

## CI, upstream dependency and publication path

Actual `.github/workflows/ci.yml` retains Rust 1.97.1, Linux libcap-ng,
fail-closed tests without embed, guest-init preprocessing, embedded workspace
clippy/tests, macOS checks and KVM/isolation. `scripts/build-init.sh` invokes
`rustup target add` and builds musl init; **do not run it under this brief**.
Frontend preprocessing is conditional; unused-dependency checks are warning-only.
No workflow or contract-drift bypass is part of this candidate.

The final KVM job compares three fixtures with **iii/main**. Source report
`results/smoke-12-impl` (release-path-unblock) attributes the prior failure to
contract drift, not KVM: workers wire1.4 versus main wire1.2; operation schemas
already equal. Other fixture differences match engine c5129b8. Snapshot here:
iii#2515 remains OPEN/CONFLICTING/DIRTY; checks API returned null, not PASS.
iii#2570 landing in the feature branch did not integrate the contract into main.

Recommended, only after explicit authorization: publish this owned branch as a
replacement integration PR into main, linking #1690/#1765 without rewriting
owners' branches. Coordinate iii#2515/contract1.4 into main, then obtain **fresh
CI on the approved integrated SHA**. Do not reuse prior CI as approval. Release
0.5.0 still requires the separate main descriptor/Build/Release Control flow
and new artifact verification; existing smoke pins are not publication pins.
Stop for parent review before any push, PR, merge, dispatch or publication.
