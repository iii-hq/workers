# sandbox

Spawn ephemeral microVMs for isolated command execution and file ops.
Provided by the `iii-sandbox` worker (v0.11.x). Fourteen
`sandbox::*` / `sandbox::fs::*` functions cover the full lifecycle.

## When to reach for sandbox

- The user asks you to run untrusted code, build artifacts, or
  execute a command whose side effects you don't want on the host.
- A long-running process (server, watcher) needs isolation from the
  user's shell session.
- File-system writes need to be reversible — drop the sandbox and
  every change is gone.

For trusted local edits, use `shell::filesystem::*` / `shell::bash::*`
directly. The sandbox is heavier (microVM boot) and only worth it
when isolation matters.

## Lifecycle (always: create → exec/fs → stop)

```
sandbox::create  →  sandbox::exec / sandbox::fs::*  →  sandbox::stop
                 \                                  /
                  → sandbox::list (any time, read-only)
```

A sandbox handle (returned by `create`, listed by `list`) is the
identifier you pass to every other call. `stop` removes the VM and
frees the resources — call it when you're done; don't leak VMs.

## The 14 functions

| Function | Purpose |
|---|---|
| `sandbox::create` | Create an ephemeral sandbox VM from a preset image. |
| `sandbox::list` | List active sandboxes. |
| `sandbox::exec` | Execute a command inside a live sandbox. |
| `sandbox::stop` | Stop and remove a running sandbox. |
| `sandbox::ls` | List directory contents inside a sandbox. |
| `sandbox::fs::read` | Stream-download a file from a sandbox. |
| `sandbox::fs::write` | Stream-upload a file into a sandbox. |
| `sandbox::fs::stat` | Stat a path inside a sandbox. |
| `sandbox::fs::mkdir` | Create a directory inside a sandbox. |
| `sandbox::fs::rm` | Remove a file or directory inside a sandbox. |
| `sandbox::fs::mv` | Move or rename a path inside a sandbox. |
| `sandbox::fs::chmod` | Change file permissions inside a sandbox. |
| `sandbox::fs::grep` | Search for a pattern in files inside a sandbox. |
| `sandbox::fs::sed` | Search-and-replace in files inside a sandbox. |

## Read the schema before every call

The published `request_format` lives in the live function listing,
not in this skill. Before using any function id above:

```json
{ "function_id": "engine::functions::list",
  "payload": { "include_internal": false } }
```

Filter the response array to the entry whose `function_id` matches,
then read its `request_format` for the exact payload shape and
`response_format` for the return shape. This is the iii-engine
discovery pattern — see the `iii` skill for the full walkthrough.

## Common shape (use the schema; this is just orientation)

Every non-`create`/`list` function expects a sandbox handle. Most
use `id` or `sandbox_id` — check `request_format` to confirm. A
typical exec call looks like:

```json
{ "function_id": "sandbox::exec",
  "payload": {
    "id": "<handle from sandbox::create>",
    "command": ["bash", "-lc", "echo hi"]
  }
}
```

A typical filesystem read:

```json
{ "function_id": "sandbox::fs::read",
  "payload": {
    "id": "<handle>",
    "path": "/work/output.txt"
  }
}
```

If a call returns `function_not_found`, the worker isn't running.
The harness lists `iii-sandbox` in its expected workers — check
`engine::workers::list` to confirm it's connected.

## Gotchas

- Paths are **inside the sandbox**, not on the host. `/work` and
  `/tmp` exist; the host filesystem does not.
- `sandbox::stop` is destructive — every change to the VM's
  filesystem is gone. Read out anything you need with
  `sandbox::fs::read` first.
- `create` may take a few seconds (microVM cold start). Don't
  set a tight `timeout_ms`; let the engine default apply.
- Always pair `create` with `stop`. A failed task should still
  call `stop` so the VM doesn't linger.
