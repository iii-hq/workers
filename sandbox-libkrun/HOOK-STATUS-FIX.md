# HTTPS18-CODE42-1: preserve build hook status

## Scope and cause

Base: `fcb5d188816c7b301f9de188c2e5777fc02a3f9f`.
Own worktree: `/home/doggao/iii/wt/workers.smoke-hook-status`.
Branch: `fix/smoke-adapter-hook-status-impl-4c7e`.

`build_image_script` previously inserted setup/install directly into its
outer `set -e` script. A failure in a non-final command of an AND-list
(`begin && npm && finish`) skips the remaining list but does not terminate
that shell. The following `scripts done` echo returns zero and masks the
hook's total nonzero status. This is a composition defect, not evidence of
a wait/Compose defect. See `results/smoke-18-https-adoption-reviewer`, phase
`code42-build-failure-diagnosis`, and
`orchestration/smoke-18-adapter-hook-status-finding`.

## Shell contract

Each nonempty setup/install runs as one single-quoted command argument in
an independent child: `bash -e -c <hook> || exit $?`, falling back to
`sh -e -c <hook>` when bash is unavailable. This matches the existing
`RUN_SCRIPT_CMD` preference without adding a custom shell setting. The
existing private `shell_quote` escapes single quotes; newlines, double
quotes, dollars and backticks remain hook text until the child parses them.
The explicit child `-e` remains effective despite the parent's OR-list;
a bare `false` followed by another command still aborts. Shell AND/OR-list
semantics inside the hook remain unchanged; the unit's total status is
propagated, not an inferred status of every internal command.

Each hook inherits `/workspace` and exported VM environment, without leaking
its own cwd, variables, options or `exit` into the next hook. Setup failure
prevents install. Empty hooks remain skipped; manifest omission/empty
resolution is unchanged. No API, wire, configuration, copy, ownership,
init/wait, image writer or Compose change is included. Source review checked
`iii-init::supervisor::run_worker` (export only on status zero),
`sync_and_exit`, and `worker_vm::run_build_vm`/`BuildJob::run` (wait status and
image-write gate); these paths are untouched.

## Evidence and pending validation

- Added `the_build_script_preserves_each_hook_status_and_shell_semantics`
  next to the existing build-script tests. It calls the real
  `build_image_script`, replaces only preparation/copy/env infrastructure
  with scratch/no-ops, and runs the production `/bin/sh -c RUN_SCRIPT_CMD`
  entrypoint with real host Bash and sh. Fake executables only model hook
  commands, never the shell status. Export/image-write are offline guarded
  scratch sentinels, not actual init/export/writer calls.
- Its 32 subcases cover both copy modes and shells: install42 with exactly
  BEGIN/FAILED and no npm/finish/done/export/write; rc0 ordering; setup42
  preventing install; multiline quoting, literal/active dollar/backticks,
  cwd/env isolation; explicit child errexit; empty, setup-only, install-only.
- **Rust test PENDING: not compiled or executed.** No Cargo/rustc invocation.
- Ran 12 lightweight offline controls with the hook template extracted from
  the source and the existing entrypoint, real Bash/sh, fake hook commands,
  and scratch sentinels: install42, success0, setup42, multiline quoting,
  child-errexit1, empty0 for each shell. All exact statuses, event sequences
  and sentinel checks passed. This is NOT execution of `build_image_script`
  or a Rust-test PASS; Python only extracts/interpolates and launches shells.
- Focal existing-toolchain `rustfmt --edition 2024 --check
  sandbox-libkrun/src/cli/local_worker.rs`: PASS after formatting edits.
- `git diff --check`: PASS.

Runtime/VM executions: **0**. Builds: **0**. No network, Node/npm/SDK,
cache/proc/ELF inspection, binary access/change, driver/fixture edit,
merge/push/PR/release, retry or admission action. Candidate `734803...` and
product `544470...` remain untouched. No integrated S507, retention, retry
or scenario acceptance claim is made.

Stop for independent source review and separate parent build admission.
Any future heavy work requires that admission and the shared target
`/home/doggao/iii/wt/kan292-target`, `nice -n 15`, `ionice -c 3`,
`CARGO_BUILD_JOBS=4` and `flock /tmp/iii-agents-heavy.lock`; none was used here.
