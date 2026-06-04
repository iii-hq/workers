# shell

Run allowlisted Unix commands, background jobs, and structured filesystem operations from the iii engine, on the host or forwarded into a sandbox microVM.

## Install

```sh
iii worker add shell
```

Sandbox-targeted execution and `shell::fs::*` forwarding need the `iii-sandbox` worker; `iii worker add shell` does not pull it in. To surface `shell::*` to LLM agents, pair with `iii-directory`:

```sh
iii worker add iii-sandbox
iii worker add iii-directory
```

## Configure

Settings load from a YAML file passed with `--config <path>` (default `./config.yaml`). The worker refuses to start unless `fs.host_root` is set, or `fs.allow_unjailed: true` is explicitly opted in, because an unset root exposes the whole host filesystem behind only the advisory denylist.

```yaml
max_timeout_ms: 30000        # hard cap; per-call timeout_ms is clamped to this
default_timeout_ms: 10000    # applied when the caller omits timeout_ms
max_output_bytes: 1048576    # 1 MiB; stdout/stderr past this set *_truncated
inherit_env: false           # when false, only allowed_env keys are forwarded
allowed_env: [PATH, HOME, LANG, LC_ALL, TERM]

# exec gate. argv[0] is matched by basename or exact path; an empty
# allowlist means open. denylist_patterns are advisory regex over
# argv.join(" "), a tripwire for honest mistakes only.
allowlist: [ls, cat, pwd, echo, grep, wc, head, tail, sort, uniq, cut, date]
denylist_patterns:
  - "rm\\s+-rf\\s+/"
  - "mkfs"

max_concurrent_jobs: 16      # exec_bg past this is rejected
job_retention_secs: 3600     # finished jobs pruned after this

fs:
  host_root: /tmp            # jail root for shell::fs::*; required (see above)
  allow_unjailed: false      # opt-in to running with host_root unset
  max_read_bytes: 16777216   # 0 = unlimited
  max_write_bytes: 16777216  # 0 = unlimited
  denylist_paths: [/etc/passwd, /etc/shadow]

sandbox:
  enabled: true              # false -> every target: sandbox call returns S210
```

Host `shell::exec` is not a security boundary: any allowlisted interpreter (`sh`, `node`, `python3`) can construct a denylisted token at runtime and bypass the regex. Run untrusted input with `target: { kind: "sandbox", sandbox_id }`, which forwards through the `iii-sandbox` microVM. The allowlist and denylist still apply on top of either backend.

## Quick start

```ts
import { registerWorker } from 'iii-sdk'

const iii = registerWorker(process.env.III_URL ?? 'ws://127.0.0.1:49134')

const result = await iii.trigger({
  function_id: 'shell::exec',
  payload: { command: 'echo', args: ['hello'] },
})

console.log(result)
```

The example runs on the host. The same payload retargets at a microVM with `target: { kind: 'sandbox', sandbox_id: '<uuid>' }`. The other entry points are `shell::exec_bg`, `shell::status`, `shell::kill`, `shell::list`, plus the `shell::fs::*` family (`ls`, `stat`, `read`, `write`, `grep`, `sed`, `mkdir`, `rm`, `chmod`, `mv`).

## Functions

| Function | Purpose |
|---|---|
| `shell::exec` | Run an allowlisted command in the foreground; returns stdout, stderr, exit code, and timing. Blocks until exit or timeout. |
| `shell::exec_bg` | Spawn an allowlisted command as a background job; returns `{ job_id, argv }` immediately. Host-targeted jobs ignore `timeout_ms` (end via `shell::kill` or natural exit); sandbox jobs honor it. |
| `shell::status` | Fetch one job's full record: state, exit code, and captured stdout/stderr. `not_found` means the id never existed or aged out past `job_retention_secs`. |
| `shell::list` | Enumerate current jobs as lightweight summaries; argv, stdout, and stderr are redacted. |
| `shell::kill` | Terminate a running background job by `job_id`. Sandbox jobs cannot be hard-killed: the record flips to `killed` but the in-VM process runs until its `timeout_ms` (or `sandbox::stop`). |
| `shell::fs::ls` | List a directory's entries with structured metadata. |
| `shell::fs::stat` | Read one path's metadata (size, mode, symlink flag). |
| `shell::fs::mkdir` | Create a directory, optionally with missing parents. |
| `shell::fs::rm` | Remove a file or directory, optionally recursive. |
| `shell::fs::chmod` | Change a path's mode, and optionally its uid/gid. |
| `shell::fs::mv` | Rename or move one path within the jail. |
| `shell::fs::grep` | Recursive regex search across a tree; returns structured matches. Keys are singular `include_glob`/`exclude_glob`; the case flag is `ignore_case`. |
| `shell::fs::sed` | Regex find-and-replace across one file or many. |
| `shell::fs::write` | Stream bytes into a file through an SDK channel; writes via a temp file and renames atomically. No inline `content` field. |
| `shell::fs::read` | Stream a file's bytes out through an SDK channel. For an inline read on the web surface, use the `harness::fs::read_inline` wrapper instead. |

Every `shell::fs::*` call accepts the same optional `target` as `exec`, so host and sandbox share one wire shape.

## Errors

Returned error bodies carry a stable `code` field. Allowlist and denylist rejections come back as a plain message (`command '<x>' not in allowlist`, `command matches denylist: <pattern>`) rather than an S-code.

| Code | Meaning |
|---|---|
| `S200` | In-VM execution failure on a sandbox target. |
| `S210` | Invalid request: non-absolute path, empty command or pattern, bad octal mode, malformed payload, or `sandbox.enabled: false` on a sandbox-targeted call. |
| `S211` | Path not found. |
| `S212` | Wrong file type for the operation (for example, a file where a directory was expected). |
| `S213` | Path already exists. |
| `S214` | Directory not empty (non-recursive `rm`). |
| `S215` | Path escapes `host_root`, hits `fs.denylist_paths`, or permission denied. |
| `S216` | Generic shell-internal failure: host spawn error, channel error, or a bad engine response. |
| `S217` | Invalid regex passed to `grep`/`sed`. |
| `S218` | `fs.max_read_bytes` / `fs.max_write_bytes` cap exceeded. |
| `S300` | Sandbox VM boot failed (needs a virtualization host: Apple Silicon or `/dev/kvm`). |

## Troubleshooting

- **`fs.host_root is unset ... refusing to start unjailed`**: set `fs.host_root` to a directory, or set `fs.allow_unjailed: true`.
- **`command '<x>' not in allowlist`**: the basename of `argv[0]` is not in a non-empty `allowlist`. Add it, or empty the list to allow anything.
- **`S215 path escapes host_root` on a path inside the jail**: a symlink in the path resolves outside the jail. Resolve it yourself, or move the target inside `host_root`.
- **`S300` on a sandbox target**: the host cannot boot microVMs. Sandbox execution requires Apple Silicon or `/dev/kvm`.
- **Worker never connects**: the engine is not running or not bound on the configured `--url`. Start the engine first; the default WebSocket port is 49134.

For the threat model, streaming wire shapes, and contributor build steps, see [ARCHITECTURE.md](ARCHITECTURE.md).

## License

Apache 2.0 — see [LICENSE](https://github.com/iii-hq/workers/blob/main/LICENSE).
