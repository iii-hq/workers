# HTTPS18-CODE42-1 / HOOK-COMPAT-1: build hook status and shared state

## Scope and cause

Original base: `fcb5d188816c7b301f9de188c2e5777fc02a3f9f`.
Previous commit (preserved): `faeef0773c0d6233fd0b7f324107835bb91210c4`.
Own worktree: `/home/doggao/iii/wt/workers.smoke-hook-status`.
Branch: `fix/smoke-adapter-hook-status-impl-4c7e`.

The base concatenated setup/install into its outer `set -e` script. A failed
non-final AND-list command (`begin && npm && finish`) skips the remaining
list without terminating that shell. The later `scripts done` echo returned
zero and masked the hook's total nonzero status (HTTPS18-CODE42-1).

The previous fix isolated hooks in child shells. Source review approved its
status propagation but identified HOOK-COMPAT-1: callers could no longer
share setup's cwd, unexported variables, functions, exported PATH, sourced
activation, umask/options or traps with install. This revision removes that
compatibility regression, not merely documents/accepts it. The previous
commit remains an ancestor; no fixture/failpoint was changed.

## Shared-shell and exit contract

Each nonempty hook is one safely quoted argument to an unconditional simple
command in the existing outer shell:

```sh
eval '<whole hook, quoted with the existing shell_quote helper>'
case $? in 0) ;; *) exit $? ;; esac
```

The helper escapes single quotes, preserving multiline text, heredocs,
quotes, dollars and backticks until eval parses the hook once. The outer
shell is still the existing `RUN_SCRIPT_CMD` Bash preference / sh fallback;
there is no new hook subprocess, forced exec, temp-script mechanism, shell
setting or public helper/API. Setup and install share the same process,
cwd, shell variables/functions, exports, sourced state, positional arguments,
umask, options and traps, as at the base. Initial cwd is `/workspace`.
Empty hooks remain skipped and manifest omission/empty resolution is unchanged.

Eval is deliberately NOT an if/AND/OR-tested command: outer errexit therefore
still applies to an internal standalone `false; echo ...`, producing rc1
before the echo. A final failed AND-list returns the hook's total rc42;
with errexit enabled eval may terminate immediately with that status, which
is correct. If the hook intentionally disables errexit, the immediate case
still propagates its total nonzero status before install/done/export. Case
pattern matching leaves that status available to `exit $?`; no rc variable,
positional-argument assignment or trap replacement can collide with caller
state. Hook `exit` retains its original whole-shell effect.

Ordinary internal shell exceptions remain: AND/OR lists, conditional tests,
and pipelines follow the chosen shell/options. This does not attempt to
capture every failed internal command when the whole hook succeeds.
Copy, ownership, init/wait, image writer, boot-without-image and Compose are
untouched. Prior source inspection established export only on status zero
and the host wait/image-write gate; no integrated S507/P8 claim is made.

## Current evidence and pending validation

- The existing focal Rust test still calls REAL `build_image_script` and
  the SAME `/bin/sh -c RUN_SCRIPT_CMD`, with real Bash/sh and scratch/no-op
  preparation/copy/env. It retains 8 cases x 2 shells x 2 copy modes = 32.
- Replaced isolated-cwd/env expectations with shared-state assertions: cd,
  unexported variable (also absent from child environment), function, sourced
  activation, exported PATH/env, positional args, umask, noglob and exit trap.
  The same multiline case covers literal/active quotes/dollars/backticks and
  a literal heredoc. Other cases cover begin42 AND-list, rc0 order, setup42
  blocking install (including set +e), standalone false/echo rc1, empty hooks,
  setup-only and install-only `set +e; false` rc1. Exact event comparison
  excludes npm/finish/done/export/image-write after failure. Export/write
  remain offline scratch sentinels, not real init/export/writer execution.
- **Rust test PENDING: not compiled or run.** No Cargo/rustc invocation.
- Executed 16 lightweight source-extracted controls (8 cases x Bash/sh),
  decoding the producer eval template, entrypoint and actual Rust fixture
  strings. Real shell statuses were 42/0/42/0/1/0/0/1 respectively for each
  shell. All exact-event, shared-state and export/write sentinel assertions
  passed. These are NOT execution of `build_image_script` or Rust-test PASS.
  Scratch is retained, without cleanup/deletion, at
  `.hook-shared-state-controls/` in this worktree; receipt: `receipt.json`.
- Existing rustfmt 1.97.1 `--edition 2024 --check` on only
  `sandbox-libkrun/src/cli/local_worker.rs`: PASS.
- `git diff --check`: PASS. Neither check entails compilation.

Runtime/VM executions: **0**. Builds: **0**. No network, Node/npm/SDK,
cache/proc/ELF inspection, binary access/change, driver/fixture edit,
prune/deletion/signals, merge/push/PR/release, retry, measurement or admission.
Candidate `734803...` and engine `544470...` remain completely untouched.
Stop for focused independent review. Rust validation/build require separate
parent admission; any future heavy work must use the prescribed shared
`kan292-target`, nice15/ionice-c3/CARGO_BUILD_JOBS4/flock, not performed here.
