---
name: shell
tags: shell, exec, filesystem, jobs, sandbox
description: >-
  Run Unix commands and structured filesystem ops from the iii engine: allowlisted
  exec, background jobs, and a host-jailed fs (ls/stat/mkdir/rm/chmod/mv/grep/sed/
  read/write), all forwardable into a sandbox microVM.
---

# shell

The shell worker is the single door every agent uses to touch the OS: run a
build, call a CLI, read a file, list a directory. Routing it all through
`shell::*` and `shell::fs::*` keeps allowlists, denylists, timeouts, output
caps, and a host-root jail in one enforceable place. Both surfaces take an
optional `target` field that forwards the call into a live `iii-sandbox`
microVM, so one allowlist policy gates host and sandbox execution alike.

Host-targeted `shell::exec` is not an isolation boundary. The denylist is a
regex tripwire on `argv.join(" ")`, and an allowlisted interpreter (`sh`,
`node`, `python3`) can construct any forbidden token at runtime to bypass it.
Run untrusted input with `target: { kind: "sandbox", sandbox_id }`. Prefer the
`shell::fs::*` backends over `exec`-ing `ls`/`stat`/`grep`/`rg`: they stay
in-process, respect the jail, and return structured results.

Sandbox forwarding (and `shell::fs::*` into a VM) requires the `iii-sandbox`
worker; `iii worker add shell` does not pull it in. To surface `shell::*` to LLM
agents, pair with the `skills` worker.

## When to Use

- Run a one-shot command and block for its full output: `git status`, `wc`,
  `head`, a quick compile probe (`shell::exec`).
- Kick off long work (build, watcher, wide grep) without blocking the turn,
  then poll for completion (`shell::exec_bg` + `shell::status`).
- Survey or terminate in-flight background jobs (`shell::list`, `shell::kill`).
- List, stat, or read files with structured output instead of shelling out to
  `ls`/`stat`/`cat` (`shell::fs::ls`, `shell::fs::stat`, `shell::fs::read`).
- Search or rewrite across a tree without spawning `rg`/`sed`
  (`shell::fs::grep`, `shell::fs::sed`).
- Create, move, remove, or re-permission paths inside the jail
  (`shell::fs::mkdir`, `shell::fs::mv`, `shell::fs::rm`, `shell::fs::chmod`).
- Persist a generated artefact, or bootstrap files into a sandbox, by streaming
  bytes to a path (`shell::fs::write` with a `target`).

## Boundaries

- Host `shell::exec` is not a security sandbox: the denylist is bypassable by
  any allowlisted interpreter. Run untrusted commands with `target: sandbox`
  (needs `iii-sandbox`).
- `shell::fs::*` is jailed to `cfg.fs.host_root` and refuses denylisted paths;
  paths must be absolute and symlinks are never followed.
- Sandbox-backed background jobs cannot be hard-killed: `shell::kill` flips the
  record but the in-VM process runs until its `timeout_ms` (or `sandbox::stop`).
- Not for inlining file bytes into an LLM tool result: `shell::fs::read`/
  `write` move bytes over channels; use the `harness` worker's
  `harness::fs::read_inline` wrapper for inline reads on the web surface.
- No batch or glob form for single-path ops (`mv`, `rm`, `stat`, …); loop in the
  caller.
- Not a package manager, editor, or migration tool; for SQL use the `database`
  worker.

## Functions

- `shell::exec`: run an allowlisted command in the foreground and return its
  stdout, stderr, exit code, and timing; blocks until exit or timeout.
- `shell::exec_bg`: spawn an allowlisted command as a background job and return
  a `job_id` immediately. Host-targeted jobs ignore `timeout_ms` (end via
  `shell::kill` or natural exit); sandbox jobs honor it.
- `shell::status`: fetch one job's full record: state, exit code, and captured
  stdout/stderr.
- `shell::list`: enumerate current jobs as lightweight summaries (no argv,
  stdout, or stderr).
- `shell::kill`: terminate a running background job by `job_id`.
- `shell::fs::ls`: list a directory's entries with structured metadata.
- `shell::fs::stat`: read one path's metadata (size, mode, symlink flag).
- `shell::fs::mkdir`: create a directory, optionally with missing parents.
- `shell::fs::rm`: remove a file or directory, optionally recursive.
- `shell::fs::chmod`: change a path's mode, and optionally its uid/gid.
- `shell::fs::mv`: rename or move one path within the jail.
- `shell::fs::grep`: recursive regex search across a tree, returning structured
  matches.
- `shell::fs::sed`: regex find-and-replace across one file or many.
- `shell::fs::write`: stream bytes into a file via a channel; writes through a
  temp file and renames atomically.
- `shell::fs::read`: stream a file's bytes out through a channel.

Every `shell::fs::*` call accepts the same optional `target` as `exec`, so host
and sandbox share one wire shape; reads and writes move bytes over SDK channels
rather than inlining them.
