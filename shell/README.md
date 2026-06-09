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

## Skills

Install the `shell` agent skill for Claude Code, Cursor, and 30+ other agents:

```bash
npx skills add iii-hq/workers --skill shell
```

Browse or install every worker skill at once:

```bash
npx skills add iii-hq/workers --list
npx skills add iii-hq/workers --all
```

## Configure

Settings are managed through the central `configuration` worker. On boot, the shell worker registers its schema (id `shell`) and fetches the live value over RPC — that live value is the authoritative config, not a local file. The optional `--config <path>` flag (default `./config.yaml`) provides the `initial_value` sent on first registration only; once registered, subsequent boots pull the stored value from the `configuration` worker. When the config changes, the worker hot-reloads the security policy and fs backend automatically (see [Hot-reload](#hot-reload)).

The worker refuses to start unless `fs.host_root` is set, or `fs.allow_unjailed: true` is explicitly opted in, because an unset root exposes the whole host filesystem behind only the advisory denylist.

By default, `mkdir`/`chmod`/`write` reject modes carrying setuid/setgid/sticky bits (the top octal digit, e.g. `4755`) with `S210`, since they are a privilege-escalation primitive when the worker runs as root inside the jail. Set `fs.allow_special_bits: true` only if your workload genuinely needs them.

```yaml
max_timeout_ms: 30000        # hard cap; per-call timeout_ms is clamped to this
default_timeout_ms: 10000    # applied when the caller omits timeout_ms
max_output_bytes: 1048576    # 1 MiB; stdout/stderr past this set *_truncated
inherit_env: false           # when false, only allowed_env keys are forwarded
allowed_env: [PATH, HOME, LANG, LC_ALL, TERM]  # also gates per-call `env` (dangerous keys never settable)

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
  allow_special_bits: false  # permit setuid/setgid/sticky bits in mode (default false)

sandbox:
  enabled: true              # false -> every target: sandbox call returns S210
```

Host `shell::exec` is not a security boundary: any allowlisted interpreter (`sh`, `node`, `python3`) can construct a denylisted token at runtime and bypass the regex. Run untrusted input with `target: { kind: "sandbox", sandbox_id }`, which forwards through the `iii-sandbox` microVM. The allowlist and denylist still apply on top of either backend.

### Per-call `cwd` and `env` (host target)

`shell::exec` and `shell::exec_bg` each accept two optional fields so an agent can scope a single command to a directory and set specific env values without wrapping everything in `sh -lc` (which would defeat the argv allowlist):

- **`cwd`** (string): the working directory for this one call. It is confined to the fs jail **exactly** like `shell::fs::*` paths — jail-relative when `fs.host_root` is set (else absolute), canonicalized, and required to resolve inside `host_root` and miss `denylist_paths`. A `cwd` that escapes the jail returns `S215`; one that doesn't exist or isn't a directory returns `S211`/`S210`. Omit it to use the configured `working_dir` (unchanged default).
- **`env`** (object of string→string): per-call environment values. A key may be set **only** if the operator already listed it in `allowed_env`, and **never** for an exec-hijacking key — `PATH`, `IFS`, and every `LD_*`/`DYLD_*` variant are on a hardcoded denylist that **wins over** `allowed_env`. Supplying a key that is not in `allowed_env`, or any dangerous key, rejects the **whole call** with `S210` (the offending key is named and the permitted keys are listed); the env is never silently dropped. A permitted per-call value overrides the value that would otherwise be forwarded for that key. So an agent can do `NODE_ENV=test` only if the operator put `NODE_ENV` in `allowed_env`, and can never inject `PATH` or `LD_PRELOAD`.

Both fields are **host-only**. The `sandbox::exec` protocol does not forward `cwd`/`env`, so a sandbox-targeted call that supplies either is rejected with `S210` rather than silently ignoring it. Omit both fields and behaviour is identical to prior versions.

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

The example runs on the host. The same payload retargets at a microVM with `target: { kind: 'sandbox', sandbox_id: '<uuid>' }`. The other entry points are `shell::exec_bg`, `shell::status`, `shell::kill`, `shell::list`, `shell::config-status`, plus the `shell::fs::*` family (`ls`, `stat`, `read`, `write`, `grep`, `sed`, `mkdir`, `rm`, `chmod`, `mv`).

## Functions

| Function | Purpose |
|---|---|
| `shell::exec` | Run an allowlisted command in the foreground; returns stdout, stderr, exit code, and timing. Blocks until exit or timeout. Accepts optional host-only `cwd` (jail-confined) and `env` (gated by `allowed_env` + a dangerous-key denylist) — see [Per-call `cwd` and `env`](#per-call-cwd-and-env-host-target). |
| `shell::exec_bg` | Spawn an allowlisted command as a background job; returns `{ job_id, argv }` immediately. Host-targeted jobs ignore `timeout_ms` (end via `shell::kill` or natural exit); sandbox jobs honor it. Same optional host-only `cwd`/`env` as `shell::exec`. |
| `shell::status` | Fetch one job's full record: state, exit code, and captured stdout/stderr. `not_found` means the id never existed or aged out past `job_retention_secs`. |
| `shell::list` | Enumerate current jobs as lightweight summaries; argv, stdout, and stderr are redacted. |
| `shell::kill` | Terminate a running background job by `job_id`. Sandbox jobs cannot be hard-killed: the record flips to `killed` but the in-VM process runs until its `timeout_ms` (or `sandbox::stop`). |
| `shell::config-status` | Report the last hot-reload outcome: `last_outcome` (`applied`/`rejected`), `last_error`, and `rejected_reloads` (count since boot). A rejected outcome or non-zero count means a stored config was refused and shell is enforcing an older policy than the central store. Takes no arguments. |
| `shell::fs::ls` | List a directory's entries with structured metadata. |
| `shell::fs::stat` | Read one path's metadata (size, mode, symlink flag). |
| `shell::fs::mkdir` | Create a directory, optionally with missing parents. Returns `{ created, path, already_existed }`. |
| `shell::fs::rm` | Remove a file or directory, optionally recursive. Returns `{ removed, path, was_present }`. |
| `shell::fs::chmod` | Change a path's mode, and optionally its uid/gid. Returns `{ entries_changed, path, recursive }`. (**Breaking**: field renamed from `updated` to `entries_changed`.) |
| `shell::fs::mv` | Rename or move one path within the jail. Returns `{ moved, src, dst, overwrote }`. |
| `shell::fs::grep` | Recursive regex search across a tree; returns structured matches. Keys are singular `include_glob`/`exclude_glob`; the case flag is `ignore_case`. |
| `shell::fs::sed` | Regex find-and-replace across one file or many. |
| `shell::fs::write` | Stream bytes into a file through an SDK channel; writes via a temp file and renames atomically. No inline `content` field. |
| `shell::fs::read` | Stream a file's bytes out through an SDK channel. For an inline read on the web surface, use the `harness::fs::read_inline` wrapper instead. |

Every `shell::fs::*` call accepts the same optional `target` as `exec`, so host and sandbox share one wire shape.

## Hot-reload

When the `configuration` worker pushes an updated config, the shell worker swaps in the new security policy and fs backend atomically. A few things to know:

- Each call executes against one consistent runtime snapshot; there is no mid-call config change.
- Already-running background jobs are **not** retroactively re-checked when the policy tightens — they continue under the policy that was active when they were spawned.
- A reload that widens the jail (for example, clearing `host_root`) succeeds but is logged as a privilege change.
- If the incoming config is invalid or unsafe, the worker keeps the last-good runtime and logs an error. The rejection is also surfaced through `shell::config-status` (a `rejected` outcome with a non-zero `rejected_reloads` count), so the divergence between the central store and the policy shell is actually enforcing is detectable instead of silent. Rejections are kept last-good and not retried (re-fetching returns the same bad value), so they will not retry-storm.
- At boot the reconcile against the configuration worker is **fail-closed**: the worker refuses to start (and exposes no functions) if it cannot confirm the authoritative config, so it never serves a possibly stale security policy.

## Errors

Returned error bodies carry a stable `code` field. Allowlist and denylist rejections come back as a plain message (`command '<x>' not in allowlist`, `command matches denylist: <pattern>`) rather than an S-code.

| Code | Meaning |
|---|---|
| `S200` | In-VM execution failure on a sandbox target. |
| `S210` | Invalid request: non-absolute path, empty command or pattern, bad octal mode, malformed payload, `sandbox.enabled: false` on a sandbox-targeted call, a `cwd` that is not a directory, an `env` key outside `allowed_env` or in the dangerous-key denylist, or `cwd`/`env` supplied on a sandbox target (host-only). |
| `S211` | Path not found (including a `cwd` that does not exist). |
| `S212` | Wrong file type for the operation (for example, a file where a directory was expected). |
| `S213` | Path already exists. |
| `S214` | Directory not empty (non-recursive `rm`). |
| `S215` | Path (or a per-call `cwd`) escapes `host_root`, hits `fs.denylist_paths`, or permission denied. |
| `S216` | Generic shell-internal failure: host spawn error, channel error, or a bad engine response. |
| `S217` | Invalid regex passed to `grep`/`sed`. |
| `S218` | `fs.max_read_bytes` / `fs.max_write_bytes` cap exceeded. |
| `S300` | Sandbox VM boot failed (needs a virtualization host: Apple Silicon or `/dev/kvm`). |

## Upgrading to 0.4.0

0.4.0 is a breaking release. Migrating from 0.3.x:

- **`shell::fs::chmod` response field renamed** `updated` → `entries_changed`; read `entries_changed`. Inbound legacy `{ updated }` from an older engine still deserializes (serde alias), but new consumers should read `entries_changed`.
- **`mkdir`/`rm`/`mv` gained structured fields** (`path`/`already_existed`, `path`/`was_present`, `src`/`dst`/`overwrote`) alongside the original boolean. Additive: existing readers of `created`/`removed`/`moved` keep working.
- **Configuration moved to the central `configuration` worker** (database-style) with hot-reload. `--config` is now a first-boot seed only; the live value from the configuration worker wins once an entry exists. `--manifest` was removed (the interface is collected live).
- **New read-only `shell::config-status`** reports the last hot-reload outcome. Operator/automation only; it is denied to agents.

## Troubleshooting

- **`fs.host_root is unset ... refusing to start unjailed`**: set `fs.host_root` to a directory, or set `fs.allow_unjailed: true`.
- **`command '<x>' not in allowlist`**: the basename of `argv[0]` is not in a non-empty `allowlist`. Add it, or empty the list to allow anything.
- **`S215 path escapes host_root` on a path inside the jail**: a symlink in the path resolves outside the jail. Resolve it yourself, or move the target inside `host_root`.
- **`S300` on a sandbox target**: the host cannot boot microVMs. Sandbox execution requires Apple Silicon or `/dev/kvm`.
- **Worker never connects**: the engine is not running or not bound on the configured `--url`. Start the engine first; the default WebSocket port is 49134.

For the threat model, streaming wire shapes, and contributor build steps, see [ARCHITECTURE.md](ARCHITECTURE.md).

## License

Apache 2.0 — see [LICENSE](https://github.com/iii-hq/workers/blob/main/LICENSE).
