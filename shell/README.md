# iii-shell

Unix shell + filesystem worker for iii agents. Every agent that needs to touch
the OS (run a build, read a file, list a directory, call a CLI) goes through
this worker so allowlists, timeouts, output caps, and jail/denylist enforcement
live in one place. Filesystem operations are exposed under `shell::fs::*` with
the same enforcement surface plus optional sandbox-target forwarding.

Current crate version: **0.3.1**. Wire shape last broken in 0.3.0 (channel-based
`shell::fs::write` / `shell::fs::read`). 0.3.1 adds the `target` field on
`shell::exec` and `shell::exec_bg` for sandbox-backed execution (additive,
backward-compatible).

## Contents

- [Quickstart](#quickstart) — go from nothing to a working `shell::exec` in 4 commands.
- [Functions](#functions) — the 15 function ids and their request/response shapes.
- [Sandbox-backed exec](#sandbox-backed-exec) — forwarding `shell::exec` / `shell::exec_bg` into a live microVM.
- [Filesystem operations: `shell::fs::*`](#filesystem-operations-shellfs)
- [Configuration](#configuration) — full defaults table + `fs` and `sandbox` sections.
- [Safety](#safety) — what is enforced, what is advisory, what the threat model is.
- [Troubleshooting](#troubleshooting)
- [Tests](#tests)
- [What this is NOT](#what-this-is-not)
- [Deferred](#deferred)

## Quickstart

```bash
# 1. Install the iii engine (drops the binary at $HOME/.local/bin/iii by default)
curl -fsSL https://install.iii.dev/iii/main/install.sh | sh

# 2. Build this worker
cargo build --release --bin iii-shell

# 3. Wire the binary where the engine looks (registered worker name = `shell`,
#    per iii.worker.yaml — fall back to ~/.iii/workers/iii-shell if your
#    engine resolves by binary name)
mkdir -p ~/.iii/workers
ln -sfn $(pwd)/target/release/iii-shell ~/.iii/workers/shell

# 4. Start the engine (it spawns the worker). Pin a host_root or set
#    fs.allow_unjailed: true in config.yaml first — the worker refuses to
#    start unjailed by default; see Configuration below.
iii -c ./config.yaml
```

> **If you plan to use sandbox targets:** `iii worker add shell` does not
> currently bring `iii-sandbox` along (the engine resolver doesn't yet
> short-circuit builtin names in `dependencies:`<!-- TODO: remove this
> caveat once the resolver supports builtins in iii.worker.yaml `dependencies:` -->),
> so run `iii worker add iii-sandbox` separately before using
> `shell::exec { target: sandbox }` or any `shell::fs::*` sandbox-target
> path. Plain host-targeted `shell::exec` works without it.

From a separate process (TS shown — Rust caller flow appears in the streaming
sections below):

```ts
import { registerWorker } from 'iii-sdk';

// `registerWorker` returns a dual handle: it can register your own functions
// AND trigger functions registered elsewhere. Here we use it client-side.
const iii = registerWorker('ws://127.0.0.1:49134');
const out = await iii.trigger({
  function_id: 'shell::exec',
  payload: { command: 'echo', args: ['hello'] },
});
// → { exit_code: 0, stdout: 'hello\n', stderr: '', duration_ms: 12,
//     timed_out: false, stdout_truncated: false, stderr_truncated: false }
```

End-to-end harness (`tests/e2e/run-tests.sh`) exercises every function and is
the canonical reference for wire shapes if anything below feels under-specified.

## Functions

15 function ids: 5 `shell::*` (process lifecycle), 10 `shell::fs::*`
(filesystem). The fs surface is documented in its own section below.

### `shell::*` — process lifecycle

| id | request | response |
|----|---------|----------|
| `shell::exec` | `{ command: string, args?: string[], timeout_ms?: number, target?: Target }` | `{ exit_code, stdout, stderr, duration_ms, timed_out, stdout_truncated, stderr_truncated }` |
| `shell::exec_bg` | `{ command: string, args?: string[], timeout_ms?: number, target?: Target }` | `{ job_id: string, argv: string[] }` |
| `shell::kill` | `{ job_id: string }` | `{ job_id, killed, status, reason? }` |
| `shell::status` | `{ job_id: string }` | `{ job: JobRecord }` (full record — argv + captured stdout/stderr) |
| `shell::list` | `{}` | `{ jobs: JobSummary[], count }` (see redaction note below) |

`command` accepts either a bare argv0 (`"ls"`) plus a separate `args` array, or
a shell-words string (`"ls -la /tmp"`) when `args` is omitted. `parse_argv`
splits the latter via `shell_words`; no expansion is performed (no `$VAR`,
globs, or command substitution).

`timeout_ms` is clamped to `max_timeout_ms` from config. Negative or non-numeric
values silently fall back to `default_timeout_ms`. `0` triggers an immediate
timeout (returns with `timed_out: true`).

#### `shell::list` returns summaries, not full records

`shell::list` returns a `JobSummary` per record:
```jsonc
{
  "id": "job-<uuid>",
  "status": "running",                 // running | finished | killed | failed
  "started_at_ms": 1714780800000,
  "finished_at_ms": 1714780801000,     // null while running
  "exit_code": 0,                      // null until terminated
  "stdout_truncated": false,
  "stderr_truncated": false
}
```

`argv`, `stdout`, and `stderr` are deliberately omitted: the global JOBS map is
process-wide and has no per-caller scope, so any caller could otherwise read
every other caller's command line and captured output (which may embed
credentials). The full record stays reachable via `shell::status <job_id>` —
the random `job_id` UUID acts as an unguessable per-record capability.

## Sandbox-backed exec

Every `shell::exec` and `shell::exec_bg` request accepts an optional
`target` field. The default (`{ "kind": "host" }`) runs on the host.
Setting `{ "kind": "sandbox", "sandbox_id": "<uuid>" }` forwards the
command to a live sandbox via the `sandbox::exec` trigger:

```ts
import { registerWorker } from 'iii-sdk';

const iii = registerWorker('ws://127.0.0.1:49134');

const { sandbox_id } = await iii.trigger({
  function_id: 'sandbox::create',
  payload: { image: 'python', cpus: 1, memory_mb: 512 },
  // `timeoutMs` is the SDK-level call timeout (camelCase). The
  // `timeout_ms` field that appears inside `payload` for shell::exec
  // is a separate, snake_case wire field consumed by the worker.
  timeoutMs: 300_000,
});

const out = await iii.trigger({
  function_id: 'shell::exec',
  payload: {
    command: 'python3',
    args: ['-c', 'print(2 + 2)'],
    target: { kind: 'sandbox', sandbox_id },
  },
});
console.log(out.stdout); // "4\n"

await iii.trigger({
  function_id: 'sandbox::stop',
  payload: { sandbox_id, wait: true },
});
```

The shell worker's allowlist still applies to sandbox-targeted calls —
the same rules that gate host commands gate sandbox commands. If you
want commands available only inside a sandbox, that's a separate
deployment decision (extend the allowlist or run a second shell worker
behind a different name).

### Caveats

- **`shell::kill` on a sandbox job is a status-level cancel, not a
  process-level cancel.** `sandbox::exec` has no cancel hook, so the
  in-VM process keeps running until its `timeout_ms` expires. The
  job's `JobRecord.status` flips to `Killed` immediately and
  `shell::status` / `shell::list` reflect the cancellation. The late
  trigger response still captures stdout/stderr into the record but
  does not overwrite the `Killed` status. If you need hard cancel,
  use a short `timeout_ms` on the original `exec_bg` request, or
  call `sandbox::stop` to tear down the whole microVM.
- **`shell::exec_bg` accepts `timeout_ms` differently per target.**
  Host: ignored (preserves today's unbounded host-bg behavior).
  Sandbox: clamped to `cfg.max_timeout_ms` and forwarded; defaults
  to `cfg.max_timeout_ms` (30s) when absent.
- **Host virtualization is required for the sandbox path.** Apple
  Silicon (macOS) or `/dev/kvm` (Linux). Intel Macs and Windows are
  unsupported. When the host can't boot microVMs,
  `shell::exec { target: sandbox }` returns `S300` (`VM boot
  failed`); shell does **not** silently fall back to host
  execution. See
  [`docs/api-reference/sandbox.mdx`](https://github.com/iii-hq/iii/blob/main/docs/api-reference/sandbox.mdx)
  for the full S-code matrix.
- **`shell::exec` host-side errors return `S216`, not `S300`.**
  S300 is reserved for sandbox VM-boot failures. A "command not
  found" or spawn error on the host returns S216 with a `host exec:`
  message prefix so callers can distinguish the two failure modes.

### `Target` shape

```ts
type Target =
  | { kind: 'host' }
  | { kind: 'sandbox'; sandbox_id: string /* uuid */ };
```

## Filesystem operations: `shell::fs::*`

Ten functions covering both the host filesystem and a sandbox VM via the same
trigger surface. Every request carries a `target` field selecting the backend;
`target` defaults to `{ "kind": "host" }` when omitted.

### Target envelope

```jsonc
// Host (default — equivalent to omitting `target`):
{ "target": { "kind": "host" }, "path": "/abs/path" }

// Sandbox (engine forwards to sandbox::fs::* via iii.trigger):
{ "target": { "kind": "sandbox", "sandbox_id": "<uuid>" }, "path": "/foo" }
```

`sandbox_id` comes from whatever started the sandbox VM — typically the engine
or a sibling `sandbox-*` worker. `shell` does not start sandboxes; it only
forwards into existing ones.

### Functions

- `shell::fs::ls` — list directory entries
- `shell::fs::stat` — file/dir metadata
- `shell::fs::mkdir` — create directory (`mode`, `parents`)
- `shell::fs::rm` — remove path (`recursive`)
- `shell::fs::chmod` — change mode (`mode`, `recursive`); supports `uid`/`gid` if running as root. Recursive walks **skip symlink entries** so chmod(2)/chown(2) don't deref to a target outside the walk root.
- `shell::fs::mv` — rename (`overwrite`); same-fs rename or cross-fs (EXDEV) copy+rename+unlink
- `shell::fs::grep` — regex search with gitignore-style globs, binary-file skip, line truncation
- `shell::fs::sed` — find-and-replace, atomic temp+rename per file, capture refs in regex mode; caller passes either `files` (explicit list) **or** `path` (walk a tree)
- `shell::fs::write` — write via `StreamChannelRef` (`mode`, `parents`; capped mid-stream by `max_write_bytes` when non-zero)
- `shell::fs::read` — read via `StreamChannelRef` (capped pre-flight by `max_read_bytes` when non-zero)

Response shapes and `S2xx` error codes match the engine daemon's
`sandbox::fs::*` exactly. Codes the host backend can produce: `S210`, `S211`,
`S212`, `S213`, `S214`, `S215`, `S216`, `S217`, `S218`, `S219`. Sandbox-target
calls additionally surface engine-level codes forwarded verbatim — most
commonly `S002` (sandbox not found) and `S001` (invalid request).

### Sandbox forwarding

Sandbox-target `shell::fs::write|read` are pure passthroughs. The worker
forwards `{ sandbox_id, path, mode, parents, content }` to the engine's
`sandbox::fs::write` (or `{ sandbox_id, path }` to `sandbox::fs::read`) via
`iii.trigger`. The caller's `StreamChannelRef` flows through the worker
verbatim — engine reads/writes the caller's channel directly, the worker is
never in the byte path. Caps configured under `fs.*` are NOT applied on the
sandbox path; engine policy applies instead.

### Wire: `shell::fs::write` and `shell::fs::read` (changed in 0.3.0)

Both ops use `iii_sdk::channels::StreamChannelRef` on the wire instead of a
base64 JSON field. The other 8 ops (`ls`, `stat`, `mkdir`, `rm`, `chmod`, `mv`,
`grep`, `sed`) are unchanged.

#### `shell::fs::write` — request

```jsonc
{
  "target": { "kind": "host" },          // default; or { "kind": "sandbox", "sandbox_id": "<uuid>" }
  "path": "/abs/path",
  "mode": "0644",
  "parents": false,
  "content": {                           // caller-allocated StreamChannelRef
    "channel_id": "...",
    "access_key": "...",
    "direction": "read"
  }
}
```

Caller flow (Rust):
```rust
let ch = iii.create_channel(Some(64)).await?;
let reader_ref = ch.reader_ref.clone();
let writer = ch.writer;
let bytes = my_bytes;
let writer_task = tokio::spawn(async move {
    writer.write(&bytes).await?;
    writer.close().await
});
let resp: WriteResponse = iii.trigger("shell::fs::write", json!({
    "target": { "kind": "host" },
    "path": "/tmp/x",
    "mode": "0644",
    "parents": false,
    "content": reader_ref,
})).await?;
writer_task.await??;
```

Response: `{ "bytes_written": N, "path": "/abs/path" }`.

#### `shell::fs::read` — response

Request: `{ "target": ..., "path": "/abs/path" }`.

Response:
```jsonc
{
  "content": { "channel_id": "...", "access_key": "...", "direction": "read" },
  "size": 1234,
  "mode": "0644",
  "mtime": 1714780800
}
```

Caller flow (Rust):
```rust
let resp: ReadResponse = iii.trigger("shell::fs::read", json!({
    "target": { "kind": "host" }, "path": "/tmp/x"
})).await?;
let reader = ChannelReader::new(iii.address(), &resp.content);
let bytes = reader.read_all().await?;
```

## Configuration

The CLI takes three flags:

| flag | default | purpose |
|------|---------|---------|
| `--config <path>` | `./config.yaml` | YAML config (shape below) |
| `--url <ws-url>` | `ws://127.0.0.1:49134` | iii engine WebSocket |
| `--manifest` | off | print the JSON function manifest and exit (use for tooling/introspection) |

### YAML defaults

| key | default | enforced where |
|-----|---------|----------------|
| `max_timeout_ms` | `30000` | hard cap; per-call `timeout_ms` clamped to this |
| `default_timeout_ms` | `10000` | applied when caller omits `timeout_ms` |
| `max_output_bytes` | `1048576` (1 MiB) | stdout/stderr truncated; `*_truncated` flagged |
| `working_dir` | `null` | pins cwd for spawned commands when set |
| `inherit_env` | `false` | when `false`, only `allowed_env` keys are forwarded |
| `allowed_env` | `[PATH, HOME, LANG, LC_ALL, TERM]` | env passthrough allowlist |
| `allowlist` | `[]` (open) | command basename allowlist; empty = open |
| `denylist_patterns` | `[]` | advisory regex tripwire on `argv.join(" ")` (see Safety) |
| `max_concurrent_jobs` | `16` | rejects new `exec_bg` past the cap |
| `job_retention_secs` | `3600` | finished jobs pruned on every `shell::list` |
| `fs.host_root` | `null` | jail root; required unless `fs.allow_unjailed: true` |
| `fs.allow_unjailed` | `false` | explicit opt-in to running with `host_root: null` |
| `fs.max_read_bytes` | `0` (unlimited) | pre-flight cap via `fs::metadata` (`S218`) |
| `fs.max_write_bytes` | `0` (unlimited) | mid-stream cap during write (`S218`) |
| `fs.denylist_paths` | `[]` | absolute-prefix denylist; rejected with `S215` |
| `sandbox.enabled` | `true` | `false` → every sandbox-target call returns `S210` |

### `fs` section

```yaml
fs:
  # SET this to a directory you intend to expose to shell::fs::*. The worker
  # refuses to start when host_root is null AND allow_unjailed is false —
  # the alternative is "the entire host filesystem is reachable behind only
  # the advisory denylist", which is rarely intended.
  host_root: /var/lib/iii-shell
  allow_unjailed: false

  max_read_bytes: 16777216    # 16 MiB; 0 = unlimited
  max_write_bytes: 16777216

  denylist_paths:
    - /etc/passwd
    - /etc/shadow
```

`max_write_bytes` is enforced **mid-stream**: the worker aborts and unlinks the
temp file when the running byte total would exceed the cap (→ `S218`).
`max_read_bytes` is enforced **pre-flight** via `fs::metadata` — `S218` is
returned before any channel is allocated.

`denylist_paths` is enforced even with `host_root` set: a denylisted absolute
prefix is always rejected with `S215`. Paths are canonicalized through every
existing ancestor before the prefix check, so symlinks pointing at denylisted
targets — or symlinks whose targets escape `host_root` — are blocked.

### `sandbox` section

```yaml
sandbox:
  enabled: true                 # set false to refuse target.kind == sandbox
```

When `sandbox.enabled = false`, every sandbox-target request returns
`S210 "sandbox target disabled in config"`.

## Safety

- `allowlist` — if non-empty, command (basename) must be present. Empty list = open.
- `denylist_patterns` — regex patterns tested against `argv.join(" ")`. **Advisory tripwires, not a security boundary**: any allowlisted shell or interpreter (`sh`, `node`, `python3`, …) can construct the forbidden token at runtime via shell variables, `eval`, `${IFS}`, base64, etc., evading any literal-form pattern. Useful for catching honest typos (operator runs `rm -rf /` directly), useless against a determined caller. The real boundary is the sandbox backend.
  ```yaml
  denylist_patterns:
    - "rm\\s+-rf\\s+/"     # honest-typo guard, not enforcement
    - ":\\(\\)\\s*\\{\\s*:\\|"  # fork bomb
    - "mkfs"
  ```
- `max_timeout_ms` — hard cap; per-call `timeout_ms` is clamped.
- `max_output_bytes` — stdout/stderr truncated at this size, flagged via `*_truncated`.
- `inherit_env: false` by default. Only variables in `allowed_env` are forwarded.
- `working_dir` — pins cwd.
- `max_concurrent_jobs` — rejects new `exec_bg` requests past the cap.
- `job_retention_secs` — old finished jobs are pruned on every `shell::list` call.
- `shell::list` returns summaries only (no argv / stdout / stderr); full records require the `job_id` capability via `shell::status`.

### Threat model

The host backend's path-validation gate is check-then-use: there is a TOCTOU
window between validation and the `std::fs::*` call. Validation walks to the
longest existing ancestor, canonicalizes that (resolving symlinks in the
existing portion), and then lexically collapses the non-existent tail before
the `starts_with(host_root)` check — so a symlink whose target escapes the
jail can't slip through the lexical fallback. But the worker is intended for
trusted caller pipelines; for untrusted input, use the sandbox backend.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `fs.host_root is unset and fs.allow_unjailed is false — refusing to start unjailed` | Default config no longer permits running unjailed. | Set `fs.host_root` to a directory, OR set `fs.allow_unjailed: true` to accept the unjailed surface. |
| `command 'xyz' not in allowlist` | `allowlist` is non-empty and doesn't include the binary's basename. | Add it to `allowlist`, or empty the list to allow anything. |
| Worker never connects to engine | Engine isn't running or isn't bound on the URL the worker is configured for. | Start the engine first; check `--url` matches. The default WS port is 49134. |
| Engine started but doesn't see the worker | Binary isn't symlinked at `~/.iii/workers/shell` (or `~/.iii/workers/iii-shell` if your engine resolves by binary name). | `ln -sfn $(pwd)/target/release/iii-shell ~/.iii/workers/shell` |
| `S215 path escapes host_root` on a path inside the jail | A symlink in the path resolves outside the jail. | Resolve the symlink yourself, or move the target inside `host_root`. |

## Tests

- `tests/e2e/` — TypeScript harness, **143 default cases + 1 jailed case**. The default suite (`run-tests.sh`) covers happy paths, safety guardrails, jobs lifecycle, fs across host and sandbox targets, adversarial protocol-break suites for streaming/exec/jobs/encoding/concurrency, plus 4 vulnerability-regression cases under `cases-vuln-repro.ts`. The jailed suite (`run-tests-jailed.sh`, against `config-jailed.yaml`) covers the symlink-parent jail-escape regression.
- `tests/*.rs` — Rust integration tests (`jobs_lifecycle`, `host_fs_branches`, `sandbox_dispatch`, `function_handlers`) covering the host backend branches, sandbox forwarder, and every typed-registration handler. Run with `cargo test`.
- Line coverage measured with `cargo tarpaulin` sits around 65%; `jobs.rs` is at 100% and the sandbox dispatch path is fully exercised.

## What this is NOT

- **Not a PTY.** Interactive shells, TUIs, password prompts all break.
- **Not an isolation boundary itself.** Host-targeted calls (`target` omitted or `{ kind: 'host' }`) run with the shell worker's OS permissions. For process isolation, set `target: { kind: 'sandbox', sandbox_id }` on `shell::exec` / `shell::exec_bg` (see [Sandbox-backed exec](#sandbox-backed-exec)) — that path forwards through `iii-sandbox`'s microVM. The allowlist + denylist still apply on top of either backend.
- **Not a streaming surface.** Foreground `shell::exec` returns once the process exits and stdout/stderr are captured whole. Live streaming is `shell::exec_stream` (deferred).
- **Not per-caller-isolated.** The JOBS registry is a process-wide singleton. `shell::list` redacts argv/stdout/stderr to limit the blast radius; full records are cap-gated by `job_id`.

## Deferred

- `shell::exec_stream` — live stdout/stderr via iii Streams (for long-running commands). Next iteration.
- Per-caller JOBS scoping (would replace the redact-summary-only mitigation in `shell::list` with proper isolation).
