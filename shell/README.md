# shell

Run Unix commands, background jobs, and structured filesystem operations from the iii engine, on the host or forwarded into a sandbox microVM.

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

## Running

The binary needs no required environment variables — it boots against a local
engine with pure defaults. The full operator surface:

| Knob | Default | What it does |
|---|---|---|
| `--url <URL>` / `III_URL` env var | `ws://127.0.0.1:49134` | WebSocket URL of the iii engine. The CLI flag wins over the env var. |
| `--config <path>` | `./config.yaml` | Seed config sent as `initial_value` on FIRST registration only; the stored value wins afterwards (see [Configure](#configure)). |
| `--version` | — | Print the worker version (also registered with the engine as worker metadata). |
| `RUST_LOG` env var | `info` | Log filter (tracing `EnvFilter` syntax, e.g. `RUST_LOG=shell=debug,info`). |

If the engine is unreachable at boot, a pre-connect probe (run detached, so it
never delays startup) logs one ERROR ("engine unreachable at <host>:<port> —
is the iii engine running? Set --url or the III_URL env var...", logging the
resolved host/port rather than the raw URL) and the worker keeps retrying in
the background every 2s — it never exits, so supervised deployments recover
as soon as the engine is up.

A `--config` seed file that exists but fails to parse (for example, one still
carrying 0.6.x keys) aborts boot rather than silently falling back to the
permissive built-in default — see [Upgrading to 0.7.0](#upgrading-to-070). A
genuinely missing file still falls through gracefully to the stored value or
the built-in zero-config default.

## Configure

Settings are managed through the central `configuration` worker. On boot, the shell worker registers its schema (id `shell`) and fetches the live value over RPC — that live value is the authoritative config, not a local file. The optional `--config <path>` flag (default `./config.yaml`) provides the `initial_value` sent on first registration only; once registered, subsequent boots pull the stored value from the `configuration` worker. When the config changes, the worker hot-reloads the security policy and fs backend automatically (see [Hot-reload](#hot-reload)).

The worker refuses to start unless `fs.host_roots` is set, or `fs.allow_unjailed: true` is explicitly opted in, because an unset root exposes the whole host filesystem behind only the advisory denylist.

By default, `mkdir`/`chmod`/`write` reject modes carrying setuid/setgid/sticky bits (the top octal digit, e.g. `4755`) with `S210`, since they are a privilege-escalation primitive when the worker runs as root. Set `fs.allow_special_bits: true` only if your workload genuinely needs them.

```yaml
max_timeout_ms: 120000       # foreground exec hard cap; per-call timeout_ms is clamped to this
max_bg_timeout_ms: 0         # host bg job hard cap in ms; 0 = unbounded (foreground uses max_timeout_ms)
default_timeout_ms: 30000    # applied when the caller omits timeout_ms (code default 10000)
max_output_bytes: 1048576    # 1 MiB; stdout/stderr past this set *_truncated
env:
  inherit: true              # forward the worker's env to children; per-call dangerous keys still blocked
  allow: [PATH, HOME, LANG, LC_ALL, TERM]  # forwarded when inherit is false; has no effect on per-call `env` (deny-only — dangerous keys never settable)

# Command policy is deny-only: allow/ask policy lives in the approval-gate.
# denylist_patterns are advisory regex over argv.join(" "), a tripwire for
# catastrophic mistakes only, NOT a security boundary. Command-shaped
# patterns are anchored to argv[0] (tolerating a sudo/doas/nohup/env/timeout
# wrapper) so `grep -rn shutdown src/` is not rejected but `sudo shutdown -h
# now` still is; argument-shaped ones (rm -rf /) stay unanchored. See the
# shipped config.yaml for the full patterns.
denylist_patterns:
  - "rm\\s+-rf\\s+/"
  - "^(\\S*/)?mkfs"
  - "^(\\S*/)?dd\\s+if="

max_concurrent_jobs: 16      # exec_bg past this is rejected
job_retention_secs: 3600     # finished jobs pruned after this

fs:
  host_roots: []             # jail roots for shell::fs::*/cwd; empty = unjailed (the shipped default); first entry = primary
  allow_unjailed: true       # explicit opt-in to running with no jail root (required whenever host_roots is empty)
  max_read_bytes: 16777216   # 0 = unlimited (reads stream; cap bounds caller cost)
  max_write_bytes: 16777216  # 0 = unlimited
  denylist_paths: [/etc/passwd, /etc/shadow]  # primary protection while unjailed; defense in depth once you set host_roots
  allow_special_bits: false  # permit setuid/setgid/sticky bits in mode (default false)

sandbox:
  enabled: true              # false -> every target: sandbox call returns S210
```

### Zero-config default

With no `--config` file and no value stored in the `configuration` worker, the worker seeds a built-in default on first registration — so it boots with nothing configured (database-style). That built-in default is the shipped [`config.yaml`](config.yaml): unjailed (`fs.allow_unjailed: true`, empty `host_roots`), env forwarded, deny-only exec with a catastrophic-only denylist (kept in sync by a unit test). Unjailed means `shell::fs::*` and `shell::exec`'s per-call `cwd` operate against the real filesystem, confined only by `fs.denylist_paths` — matching `shell::exec` itself, which has never been confinement-based. `coder::*` is the one exception: it falls back to its own default roots (engine workspace cwd + `/tmp`) whenever `fs.host_roots` is empty, so it stays reasonably scoped even under the zero-config default. Set `fs.host_roots` if you want `shell::fs::*`/`cwd` jailed too. If the stored value is later nulled, the worker does not silently fall back to this seed: boot fails closed and a hot-reload keeps the last-good config. A config that is *present* but leaves `fs.host_roots` unset (without `fs.allow_unjailed: true`) also fails closed — the failure mode only disappears once you explicitly opt in, one way or another.

Host `shell::exec` is not a security boundary: any interpreter (`sh`, `node`, `python3`) can construct a denylisted token at runtime and bypass the regex. Run untrusted input with `target: { kind: "sandbox", sandbox_id }`, which forwards through the `iii-sandbox` microVM. The denylist still applies on top of either backend.

### Per-call `cwd`, `env`, and `stdin` (host target)

`shell::exec` and `shell::exec_bg` each accept optional fields so an agent can scope a single command to a directory, set specific env values, and feed it standard input without wrapping everything in `sh -lc` (which would blur what the argv actually was):

- **`cwd`** (string): the working directory for this one call. It is confined to the fs jail **exactly** like `shell::fs::*` paths — jail-relative when `fs.host_roots` is set (else absolute), canonicalized, and required to resolve inside a jail root and miss `denylist_paths`. A `cwd` that escapes the jail returns `S215`; one that doesn't exist or isn't a directory returns `S211`/`S210`. Omit it to use the configured `working_dir` (unchanged default).
- **`env`** (object of string→string): per-call environment values. Deny-only, like the exec/fs command policy: a key may be set to **any** value **except** an exec-hijacking key — `PATH`, `IFS`, `HOME`, every `LD_*`/`DYLD_*` variant, and other loader/lookup-path and interpreter startup-file keys (`GCONV_PATH`, `BASH_ENV`, `ENV`, `PYTHONSTARTUP`, `PERL5OPT`, `RUBYOPT`, `NODE_OPTIONS`, …), which are rejected unconditionally. `env.allow` plays **no role** in this gate at all — that list's only job is which vars get *forwarded* from the worker's own environment when `env.inherit` is false (see [Configure](#configure)); it has nothing to do with what a per-call override may set. Supplying a dangerous key rejects the **whole call** with `S210` (the offending key is named); the env is never silently dropped. A per-call value overrides the value that would otherwise be forwarded for that key. So an agent can do `NODE_ENV=test` freely, but can never inject `PATH`, `HOME`, or `LD_PRELOAD`.
- **`stdin`** (string): written to the program's standard input, which is then closed (EOF). Use it to feed `tee`, `patch`, `cat`, or any stdin filter instead of a shell heredoc. Omit it and stdin is `/dev/null`.

All three fields are **host-only**. The `sandbox::exec` protocol does not forward `cwd`/`env`/`stdin`, so a sandbox-targeted call that supplies any of them is rejected with `S210` rather than silently ignoring it. Omit them and behaviour is identical to prior versions.

## Quick start

```sh
npm install iii-sdk
```

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
| `shell::exec` | Run a command in the foreground; returns stdout, stderr, exit code, and timing. Blocks until exit or timeout. Accepts optional host-only `cwd` (confined to `fs.host_roots` when set, else absolute), `env` (deny-only, gated by a dangerous-key denylist), and `stdin` (string piped to the program's stdin, then EOF) — see [Per-call `cwd`, `env`, and `stdin`](#per-call-cwd-env-and-stdin-host-target). |
| `shell::exec_bg` | Spawn a command as a background job; returns `{ job_id, argv }` immediately. Host-targeted jobs run until they exit or `shell::kill` terminates them — unbounded by default, and capped only when the operator sets a positive `max_bg_timeout_ms` (default `0` = unbounded), after which a runaway job is killed and its status becomes `killed`. Sandbox jobs honor `timeout_ms`. Same optional host-only `cwd`/`env`/`stdin` as `shell::exec`. |
| `shell::status` | Fetch one job's full record: state, exit code, and captured stdout/stderr. A missing id — one that never existed or aged out past `job_retention_secs` — returns an `S211` ("no such job") error. |
| `shell::list` | Enumerate current jobs as lightweight summaries; argv, stdout, and stderr are redacted. |
| `shell::kill` | Terminate a running background job by `job_id`. Sandbox jobs cannot be hard-killed: the record flips to `killed` but the in-VM process runs until its `timeout_ms` (or `sandbox::stop`). |
| `shell::config-status` | *(operator/automation only — not agent-callable)* Report the last hot-reload outcome: `last_outcome` (`applied`/`rejected`), `last_error`, and `rejected_reloads` (count since boot). A rejected outcome or non-zero count means a stored config was refused and shell is enforcing an older policy than the central store. Takes no arguments. |
| `shell::fs::ls` | List a directory's entries with structured metadata. |
| `shell::fs::stat` | Read one path's metadata (size, mode, symlink flag). |
| `shell::fs::mkdir` | Create a directory, optionally with missing parents. Returns `{ created, path, already_existed }`. |
| `shell::fs::rm` | Remove a file or directory, optionally recursive. Returns `{ removed, path, was_present }`. |
| `shell::fs::chmod` | Change a path's mode, and optionally its uid/gid. Returns `{ entries_changed, path, recursive }`. (**Breaking**: field renamed from `updated` to `entries_changed`.) |
| `shell::fs::mv` | Rename or move one path within the jail. Returns `{ moved, src, dst, overwrote }`. |
| `shell::fs::grep` | Recursive regex search across a tree; returns structured matches. Keys are singular `include_glob`/`exclude_glob`; the case flag is `ignore_case`. |
| `shell::fs::sed` | Regex find-and-replace across one file or many. |
| `shell::fs::write` | Write a file. Simplest form passes inline string `content` (host target only): `{ path, content: "file text" }`, with `mode` (octal, default `"0644"`) and `parents: true` to create parents. A `ContentRef` object in `content` instead streams large/staged payloads through an SDK channel (temp file + atomic rename) and is **required** for sandbox targets — an inline string on a sandbox target returns `S210`. Batch form: pass `files: [{ path, content, mode?, parents? }, ...]` (host, inline per file) to write several files in one call; the response then carries per-file `files: [{ path, bytes_written }]`. A single-file write leaves `files` empty and returns `{ bytes_written, path }`. Supplying both single `path`/`content` and `files` returns `S210`. |
| `shell::fs::read` | Stream a file's bytes out through an SDK channel. For an inline read on the web surface, use the `harness::fs::read_inline` wrapper instead. |

Every `shell::fs::*` call accepts the same optional `target` as `exec`, so host and sandbox share one wire shape.

## Code surface (`coder::*`)

The shell worker also serves the **`coder::*`** code-file functions (the former
standalone `coder` worker, folded in). They are agent-ergonomic, structured
file operations that share `shell::fs::*`'s jail **when `fs.host_roots` is
set**. When it's empty (the shipped default — see
[Zero-config default](#zero-config-default)), the two surfaces diverge:
`shell::fs::*` becomes fully unjailed, but `coder::*` falls back to its own
default roots (engine workspace cwd + `/tmp`) instead — it never runs
fully unjailed, regardless of `fs.allow_unjailed`.

| Function | Purpose |
|---|---|
| `coder::info` | Discover the jail: roots, caps, response budgets, exclude/non-accessible globs. Call first. |
| `coder::read-file` | Windowed reads (`line_from`/`line_to`), `stat` probe, byte-budgeted full reads, and multi-file batch reads. |
| `coder::search` | Literal/regex content + path search with context lines, bounded by match/byte budgets. |
| `coder::list-folder` | Paginated single-folder listing. |
| `coder::tree` | Recursive depth- and per-folder-bounded directory snapshot. |
| `coder::create-file` / `coder::update-file` / `coder::delete-file` / `coder::move` | Batched create, line/regex edits, delete, and atomic rename/move. |

Roots come from `fs.host_roots` (with the cwd+`/tmp` fallback noted above);
protection globs come from `code.non_accessible_globs` in the shipped
`config.yaml`'s `code:` block — the **same** list `shell::fs::*` enforces.
`coder::*` returns its own `C2xx` codes, distinct from `shell::*`'s `S2xx`:

| Code | Meaning |
|---|---|
| `C210` | Malformed input: bad payload, illegal line numbers, overlapping ops. |
| `C211` | Path not found, or matched `non_accessible_globs` — deliberately the same code for both, so a caller can't probe for a denied file's existence by toggling the glob. |
| `C213` | File exceeds `max_read_bytes`/`max_write_bytes`. |
| `C215` | Path escapes every allowed root, lexically or through a symlink. |
| `C216` | Underlying I/O error. |
| `C217` | `create-file` saw an existing file and `overwrite=false`. |
| `C218` | Path resolves inside a configured root but outside the per-call `scope_root` the session is scoped to. |

No separate install: `iii worker add shell` brings the whole surface.

## Hot-reload

When the `configuration` worker pushes an updated config, the shell worker swaps in the new security policy and fs backend atomically. A few things to know:

- Each call executes against one consistent runtime snapshot; there is no mid-call config change.
- Already-running background jobs are **not** retroactively re-checked when the policy tightens — they continue under the policy that was active when they were spawned.
- A reload that widens the jail (for example, clearing `host_roots`) succeeds but is logged as a privilege change.
- If the incoming config is invalid or unsafe, the worker keeps the last-good runtime and logs an error. The rejection is also surfaced through `shell::config-status` (a `rejected` outcome with a non-zero `rejected_reloads` count), so the divergence between the central store and the policy shell is actually enforcing is detectable instead of silent. Rejections are kept last-good and not retried (re-fetching returns the same bad value), so they will not retry-storm.
- At boot the reconcile against the configuration worker is **fail-closed**: the worker refuses to start (and exposes no functions) if it cannot confirm the authoritative config, so it never serves a possibly stale security policy.

## Errors

Returned error bodies carry a stable `code` field. Denylist rejections come back as a plain message (`command matches denylist: <pattern>`) rather than an S-code.

| Code | Meaning |
|---|---|
| `S200` | In-VM execution failure on a sandbox target. |
| `S210` | Invalid request: non-absolute path, empty command or pattern, bad octal mode, malformed payload, `sandbox.enabled: false` on a sandbox-targeted call, a `cwd` that is not a directory, an `env` key in the dangerous-key denylist, `cwd`/`env`/`stdin` supplied on a sandbox target (host-only), an inline string `content` on a sandbox-targeted `shell::fs::write`, or both single `path`/`content` and `files` on `shell::fs::write`. |
| `S211` | Path not found (including a `cwd` that does not exist). |
| `S212` | Wrong file type for the operation (for example, a file where a directory was expected). |
| `S213` | Path already exists. |
| `S214` | Directory not empty (non-recursive `rm`). |
| `S215` | Path (or a per-call `cwd`) escapes the `fs.host_roots` jail, hits `fs.denylist_paths`, or permission denied. |
| `S216` | Generic shell-internal failure: host spawn error, channel error, or a bad engine response. |
| `S217` | Invalid regex passed to `grep`/`sed`. |
| `S218` | `fs.max_read_bytes` / `fs.max_write_bytes` cap exceeded. |
| `S300` | Sandbox VM boot failed (needs a virtualization host: Apple Silicon or `/dev/kvm`). |

Sandbox-forwarded `fs::*`/`exec` errors can also surface engine codes verbatim instead of collapsing to `S216`: `S001`–`S004` (sandbox lifecycle), `S100`–`S102` (image/VM/resource), `S300`, and `S400`. Branch on the specific code where relevant; only an unrecognized engine code falls back to `S216`.

## Upgrading to 0.8.0

- **BREAKING: `allowlist` removed.** The command allowlist is gone — the
  shell is deny-only now (see [Configure](#configure)); allow/ask policy
  lives entirely in the approval-gate. Any stored config still carrying the
  key, including the inert `allowlist: []` written by older seeds, fails
  closed at boot with a migration hint. Rewrite the stored value to drop it:

  ```ts
  import { registerWorker } from 'iii-sdk'
  const iii = registerWorker(process.env.III_URL ?? 'ws://127.0.0.1:49134')
  const { value } = await iii.trigger({ function_id: 'configuration::get', payload: { id: 'shell' } })
  delete value.allowlist
  await iii.trigger({ function_id: 'configuration::set', payload: { id: 'shell', value } })
  ```

  If you seed from a `--config` file instead, just delete the `allowlist: []` line.

- **BREAKING (fresh installs only): the fs jail is unjailed by default.** The
  shipped `config.yaml` now sets `fs.allow_unjailed: true` with empty
  `fs.host_roots` — `shell::fs::*` and `shell::exec`'s per-call `cwd` operate
  against the real filesystem by default (see
  [Zero-config default](#zero-config-default)). This only affects a
  genuinely fresh install or a config that gets nulled — an existing
  deployment with an explicit `fs.host_roots` already in its stored config is
  unaffected; the stored value always wins over the seed. To keep the old
  jailed-to-`/tmp` behavior, set it explicitly:

  ```yaml
  fs:
    host_roots: [/tmp]
  ```

- **The per-call `env` override is deny-only.** `env.allow` no longer gates
  which keys `shell::exec`/`exec_bg`'s per-call `env` may set — only the
  hardcoded dangerous-key list does (see [Per-call `cwd`, `env`, and
  `stdin`](#per-call-cwd-env-and-stdin-host-target)). This is a pure
  widening: nothing that worked before now fails, and no stored config needs
  a rewrite. `env.allow` keeps its other job (forwarding when `env.inherit`
  is false) unchanged.

## Upgrading to 0.7.0

- **BREAKING: env config keys renamed and nested.** The top-level `inherit_env`
  and `allowed_env` keys are replaced by one `env` block — no legacy aliases:

  ```yaml
  # 0.6.x                                # 0.7.0
  inherit_env: true                      env:
  allowed_env: [PATH, HOME, LANG]          inherit: true
                                           allow: [PATH, HOME, LANG]
  ```

  The old keys are **rejected at parse** with a migration hint ("config keys
  removed in 0.7.0: `inherit_env` -> `env.inherit` ..."), because serde would
  otherwise ignore them silently and boot with env forwarding OFF. A stored
  configuration value still carrying the old keys makes the worker fail closed
  at boot; rewrite it via `configuration::set` (id `shell`) with the nested
  shape. **Sequencing matters**: update the binary FIRST, then the stored
  value — writing the new shape while 0.6.x is still running makes the old
  worker hot-reload it, ignore the unknown `env` block, and silently stop
  forwarding env until restart. A **half-migration** (nesting the OLD key
  names under the new block, e.g. `env: { inherit_env: true }`) is also
  rejected — `env` denies unknown fields — rather than silently falling back
  to the wider default `allow` list.
  A `--config` seed file carrying any of these removed keys now **aborts
  boot** rather than warning and falling back to the permissive built-in
  seed; only a genuinely missing seed file falls through gracefully.
- **BREAKING: `fs.host_root` (single-root alias) removed.** The 0.6.x
  one-entry alias for the jail root is **rejected at parse** with a migration
  hint ("config key removed in 0.7.0: `fs.host_root` -> `fs.host_roots`
  (one-entry list)"). Replace it with the list form:

  ```yaml
  # 0.6.x                                # 0.7.0
  fs:                                    fs:
    host_root: /srv/app                    host_roots: [/srv/app]
  ```

  Same fail-closed rationale as the env keys: serde would otherwise ignore
  the stale key and the worker would see no jail configured at all.
- **BREAKING: `code.base_path`/`code.base_paths` removed from the schema and
  REJECTED at parse.** They were inert even before removal — the code
  resolver has taken its roots from `fs.host_roots` since the coder merge —
  but this is a hard migration: "never had an effect" is not an exception. A
  stored value still carrying either fails closed with a hint naming both
  keys, same as every other removed key. Set the jail once via
  `fs.host_roots`.
- **The one-shot coder→shell config migration is removed.** 0.7.0 no longer
  folds a legacy standalone-`coder` configuration entry into the `shell`
  value at boot, and the hidden `migrated_from_coder` marker field is
  REJECTED at parse rather than silently tolerated. Boot also no longer
  probes `configuration::get` for the `coder` entry, so the
  "configuration 'coder' not found" WARN retries at startup are gone.
  **Two distinct upgrade scenarios**:
  - An install that ALREADY has a `shell` entry (it went through the fold
    under 0.6.x, so the entry carries `migrated_from_coder: true`) now fails
    closed at 0.7.0 boot with a migration hint, instead of silently parsing
    past the marker.
  - An install with ONLY a standalone `coder` entry and NO `shell` entry at
    all has nothing to reject — it still boots 0.7.0 with the generic
    permissive `/tmp` dev seed for `shell`, silently, because there is no
    stored `shell` value to fail closed on. The old `coder` roots and
    protected globs are NOT carried over. Boot 0.6.x once first (it performs
    the migration and writes the `shell` entry) before upgrading to 0.7.0 to
    avoid this.
- **`--version` added**, and `--url`/`III_URL` and `RUST_LOG` are now
  documented (see [Running](#running)).
- **Unreachable-engine boot is loud**: one ERROR naming the host/port and the
  fix hint, instead of only the SDK's silent retry WARNs. The probe runs
  detached so it never delays boot.
- **Every config field now carries a schema description**, so the console
  configuration UI documents each knob inline.
- **Command-shaped denylist patterns tolerate a wrapper prefix**
  (`sudo`/`doas`/`nohup`/`env`/`timeout [duration]`, optionally
  path-qualified): `sudo shutdown -h now` trips the tripwire again.

## Upgrading to 0.4.0

0.4.0 is a breaking release. Migrating from 0.3.x:

- **`shell::fs::chmod` response field renamed** `updated` → `entries_changed`; read `entries_changed`. Inbound legacy `{ updated }` from an older engine still deserializes (serde alias), but new consumers should read `entries_changed`.
- **`mkdir`/`rm`/`mv` gained structured fields** (`path`/`already_existed`, `path`/`was_present`, `src`/`dst`/`overwrote`) alongside the original boolean. Additive: existing readers of `created`/`removed`/`moved` keep working.
- **Configuration moved to the central `configuration` worker** (database-style) with hot-reload. `--config` is now a first-boot seed only; the live value from the configuration worker wins once an entry exists. `--manifest` was removed (the interface is collected live).
- **New read-only `shell::config-status`** reports the last hot-reload outcome. Operator/automation only; it is denied to agents.
- **Error envelope: the S-code is now the top-level wire `code`.** Every S-coded failure returns `{ "code": "S211", "message": "no such job: ..." }` with the S-code as the top-level `code`. Previously the code was buried — handlers returned the `{code,message}` JSON stringified into `message` under the generic `invocation_failed` code. Consumers that branched on `code == "invocation_failed"` or parsed the embedded JSON out of the message must update to read `error.code` directly.
- **Host background jobs are unbounded by default.** A host-targeted `shell::exec_bg` job runs until it exits or `shell::kill` terminates it — long installs, builds, dev-servers, and tails are not killed. To force-kill a runaway host bg job, set a positive `max_bg_timeout_ms` (new operator config field, default `0` = unbounded; separate from `max_timeout_ms`, which bounds foreground `shell::exec`); a job that exceeds it is killed and its status becomes `killed`. Sandbox jobs still honor `timeout_ms`.
- **Per-call `env` denylist broadened.** The always-rejected key set now includes `HOME` and loader/lookup-path keys (`LD_*`, `DYLD_*`, `GCONV_PATH`, …) **and** interpreter/shell startup-file keys (`BASH_ENV`, `ENV`, `PYTHONSTARTUP`, `PERL5OPT`, `RUBYOPT`, `NODE_OPTIONS`). These are rejected even if listed in `allowed_env`. `LD_*`/`DYLD_*` are matched by family prefix, so a loader variable not in the explicit list is still rejected. Note that `HOME` is in the default `allowed_env` for the worker's own forwarded env but is **not** settable per-call.
- **Sandbox-forwarded fs/exec errors can now surface engine S-codes** that 0.3.7 collapsed to `S216` — e.g. `S001`–`S004` (lifecycle), `S100`–`S102` (image/VM), `S300`, and `S400`. Branch on the specific code where relevant.

## Troubleshooting

- **`fs.host_roots is empty ... refusing to start unjailed`**: set `fs.host_roots` to at least one directory, or set `fs.allow_unjailed: true`.
- **`S215 path escapes the fs jail roots` on a path inside the jail**: a symlink in the path resolves outside the jail. Resolve it yourself, or move the target inside a jail root.
- **`S300` on a sandbox target**: the host cannot boot microVMs. Sandbox execution requires Apple Silicon or `/dev/kvm`.
- **Worker never connects**: the engine is not running or not bound on the configured `--url`. Start the engine first; the default WebSocket port is 49134.
- **`config keys removed in 0.7.0: ...` at boot or on reload**: the seed file or the stored configuration value still uses the 0.6.x `inherit_env`/`allowed_env` keys (nest them under `env:` as `inherit`/`allow`) or the single-root `fs.host_root` alias (use `fs.host_roots: [<path>]`) — see [Upgrading to 0.7.0](#upgrading-to-070).
- **`` `allowlist` (removed in 0.8.0, no replacement) `` at boot or on reload**: the seed file or the stored configuration value still carries the removed `allowlist` key (even an empty `allowlist: []` trips this) — see [Upgrading to 0.8.0](#upgrading-to-080) for the fix.
- **A relative path (including `~`) passed to `shell::fs::*`/`cwd` behaves unexpectedly**: paths are never shell-expanded. `~` is not resolved to your home directory — when jailed, a relative path (including one starting with `~`) resolves against the primary `fs.host_roots` entry, so `~/foo` is interpreted as a path literally named `~` under that root, not your home directory; when unjailed, only absolute paths are accepted at all, so a relative path (including `~/foo`) is rejected outright. Pass an absolute, already-expanded path instead.

For the threat model, streaming wire shapes, and contributor build steps, see [ARCHITECTURE.md](ARCHITECTURE.md).

## License

Apache 2.0 — see [LICENSE](https://github.com/iii-hq/workers/blob/main/LICENSE).
