# shell

Unix shell + filesystem worker. Runs allowlisted commands on the host (foreground or background) and reads/writes files inside an optional `host_root` jail. Every function may also be retargeted at a microVM via `target: { kind: "sandbox", sandbox_id }`.

- [`shell`](iii://shell)
  - [`shell::exec`](iii://shell/exec) — run an allowlisted command, return full stdout/stderr/exit
  - [`shell::exec_bg`](iii://shell/exec_bg) — start a background job, get a `job_id` back
  - [`shell::status`](iii://shell/status) — fetch a job's full record
  - [`shell::kill`](iii://shell/kill) — terminate a running job
  - [`shell::list`](iii://shell/list) — summary of every known job

  - [`shell::fs::ls`](iii://shell/fs/ls) — list a directory inside the jail
  - [`shell::fs::stat`](iii://shell/fs/stat) — stat one path
  - [`shell::fs::read`](iii://shell/fs/read) — open a stream channel to read a file
  - [`shell::fs::grep`](iii://shell/fs/grep) — recursive regex search

  - [`shell::fs::write`](iii://shell/fs/write) — open a stream channel to write a file
  - [`shell::fs::sed`](iii://shell/fs/sed) — find-and-replace, single file or directory walk
  - [`shell::fs::mkdir`](iii://shell/fs/mkdir) — create a directory
  - [`shell::fs::rm`](iii://shell/fs/rm) — unlink a file or directory
  - [`shell::fs::chmod`](iii://shell/fs/chmod) — change permissions (and optionally owner)
  - [`shell::fs::mv`](iii://shell/fs/mv) — move/rename inside the jail

Live job table: [`iii://fn/shell/list`](iii://fn/shell/list).

## Invariants (read once, applies to every call)

- **Paths must be absolute.** Relative paths are rejected by every `fs::*` backend before the call dispatches.
- **Filesystem ops are jailed to `fs.host_root`** when configured. Anything outside the configured root is refused. The path denylist (`fs.denylist_paths`) refuses inside the jail too. Operators may opt into running unjailed via `fs.allow_unjailed: true`; in that mode the worker logs a warning at boot and the entire host filesystem is reachable through `fs::*` (denylist still applies).
- **Host-targeted `exec` / `exec_bg` are NOT a sandbox.** The allowlist gates `argv[0]` (matched by basename or exact path); the denylist is a regex tripwire over `argv.join(" ")` for honest-mistake patterns. A caller that can run an allowlisted interpreter (`sh`, `node`, `python`, …) can bypass denylist patterns by construction. For real isolation, pass `target: { kind: "sandbox", sandbox_id: "<uuid>" }` on `exec` / `exec_bg` / any `fs::*` request — that forwards to a live microVM via `iii-sandbox`. Allowlist + denylist still apply on top of the sandbox path. Caller manages the sandbox via `sandbox::create` / `sandbox::stop`.
- **Sandbox targeting requires libkrun support.** When the host can't boot microVMs (no Apple Silicon and no `/dev/kvm`), sandbox-targeted calls return `S300`; shell does NOT fall back to host execution.
- **Errors arrive as JSON-encoded strings.** Backend failures serialize an `FsError` / `ExecError` as the `Err` payload of the trigger result; `S2xx` codes are policy/wire issues, `S3xx` is sandbox/VM. Don't retry the same payload after a policy refusal.
