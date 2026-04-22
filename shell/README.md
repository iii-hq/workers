# iii-shell

Unix shell execution worker for iii agents. Mike's priority-1 fundamental — every agent worker that needs to touch the OS (run a build, read a file via `cat`, list a directory, call a CLI) goes through this worker so there is a single place to enforce allowlists, timeouts, and output caps.

## Functions

| id | shape |
|----|-------|
| `shell::exec` | run to completion, return `{exit_code, stdout, stderr, duration_ms, timed_out, stdout_truncated, stderr_truncated}` |
| `shell::exec_bg` | spawn in background, return `{job_id, argv}` |
| `shell::kill` | kill a running `job_id` |
| `shell::status` | return `{job: JobRecord}` for a `job_id` |
| `shell::list` | return all jobs + counts |

## HTTP triggers

```
POST /api/shell/exec     → shell::exec
POST /api/shell/exec_bg  → shell::exec_bg
POST /api/shell/kill     → shell::kill
POST /api/shell/status   → shell::status
GET  /api/shell/list     → shell::list
```

## Safety

- `allowlist` — if non-empty, command (basename) must be present. Empty list = open.
- `denylist_patterns` — regex patterns tested against the full joined argv. Example: `rm\s+-rf\s+/`, `:()\s*\{\s*:\|` (fork bomb), `mkfs`, `shutdown`.
- `max_timeout_ms` — hard cap; per-call `timeout_ms` is clamped.
- `max_output_bytes` — stdout/stderr are truncated at this size, flagged via `*_truncated`.
- `inherit_env: false` by default. Only variables in `allowed_env` are forwarded.
- `working_dir` — pins cwd.
- `max_concurrent_jobs` — rejects new `exec_bg` requests past the cap.
- `job_retention_secs` — old finished jobs are pruned on every `shell::list` call.

## Example

```bash
curl -X POST localhost:3111/api/shell/exec -d '{
  "command": "ls",
  "args": ["-la", "/tmp"],
  "timeout_ms": 5000
}'
# → {"exit_code": 0, "stdout": "total …", "stderr": "", "duration_ms": 12, ...}

curl -X POST localhost:3111/api/shell/exec_bg -d '{
  "command": "cargo",
  "args": ["build", "--release"]
}'
# → {"job_id": "job-abc…", "argv": ["cargo", "build", "--release"]}

curl -X POST localhost:3111/api/shell/status -d '{"job_id": "job-abc…"}'
```

## Run locally

```bash
cargo run --release -- --config ./config.yaml --url ws://127.0.0.1:49134
```

## What this is NOT

- Not a PTY. Interactive shells, TUIs, password prompts all break.
- Not a remote executor. Runs on the worker's host only.
- Not a sandbox. For isolation use `sandbox-docker`/`sandbox-firecracker` and call through `shell` only for trusted commands.

## Deferred

- `shell::exec_stream` — live stdout/stderr via iii Streams (for long-running commands). Next iteration.
