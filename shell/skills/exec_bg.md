# shell::exec_bg

Spawn a command as a background job and return immediately with a `job_id`.

`({ command, args?, timeout_ms?, target? }) → { job_id, argv }` — same payload shape as `shell::exec`. `command` is the program name as a **string**; split arguments into `args`. The job runs in the background; poll with `shell::status` and terminate with `shell::kill`. `job_id` is `job-<uuid>`. `argv` echoes the resolved argument vector.

The wire shape is published as a JSON schema via `ExecBgRequest` in `functions::types`.

## When to use

- Builds, long greps, watchers — anything that doesn't fit inside `max_timeout_ms`.
- You want to keep working on other tool calls while the command runs.
- "Run cargo build and show me the result when it's done" → `exec_bg` then poll `shell::status`.

## Notes

- Wrong-type fields are caught up front by the same per-field deserializers `shell::exec` uses; sending `command: ["sh", "-lc", "..."]` returns the actionable shape hint instead of a misleading "missing 'command'".
- Allowlist + denylist gate the spawn the same way they gate `shell::exec`. A blocked argv comes back as the trigger `Err`; the job is never inserted into the table.
- Host-targeted background jobs IGNORE `timeout_ms` (preserves the unbounded host-bg semantic). Only `shell::kill` or natural exit ends them.
- Sandbox-targeted background jobs DO honour `timeout_ms`: it's clamped through `cfg.resolve_timeout` (default 10 s, max 30 s) and forwarded to `sandbox::exec`.
- Concurrency cap: `cfg.max_concurrent_jobs` (default 16). If exceeded, the call returns `Err` from `try_reserve_and_insert` and the spawned child is killed before the call returns — wait for an existing job to finish, then retry.
- Sandbox-backed jobs cannot be hard-killed. `shell::kill` flips the record to `killed` immediately, but the in-VM process keeps running until its `timeout_ms` expires (or until `sandbox::stop` tears down the microVM).
- Sandbox-target requires libkrun support; otherwise the engine returns `S300` and the job is never recorded.
