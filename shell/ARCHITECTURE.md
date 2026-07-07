# shell — architecture and operator notes

The published `README.md` and `skills/SKILL.md` for this worker are hand-maintained. This file holds the operator/contributor material that does not belong in the published surfaces: full configuration table, threat model, wire shapes for the streaming functions, troubleshooting, tests, and deferred work.

## Build and wire-up

```bash
# 1. Install the iii engine (drops the binary at $HOME/.local/bin/iii by default).
curl -fsSL https://install.iii.dev/iii/main/install.sh | sh

# 2. Build this worker.
cargo build --release --bin iii-shell

# 3. Wire the binary where the engine looks (registered worker name = `shell`,
#    per iii.worker.yaml — fall back to ~/.iii/workers/iii-shell if your
#    engine resolves by binary name).
mkdir -p ~/.iii/workers
ln -sfn $(pwd)/target/release/iii-shell ~/.iii/workers/shell

# 4. Start the engine (it spawns the worker). Pin fs.host_roots or set
#    fs.allow_unjailed: true in config.yaml first — the worker refuses to
#    start unjailed by default.
iii -c ./config.yaml
```

`iii worker add shell` does not currently pull `iii-sandbox` along — run `iii worker add iii-sandbox` separately before using `shell::exec { target: sandbox }` or any `shell::fs::*` sandbox-target path. Plain host-targeted `shell::exec` works without it.

## CLI flags

| flag | default | purpose |
|------|---------|---------|
| `--config <path>` | `./config.yaml` | Optional seed config: the YAML is passed as `initial_value` when registering the schema with the `configuration` worker on first boot. It is **not** the live source of truth — the live value is fetched over RPC after registration. When the file is absent and nothing is stored yet, the worker seeds a built-in zero-config default (`ShellConfig::seed_default()`, unjailed by explicit opt-in) instead. |
| `--url <ws-url>` | `ws://127.0.0.1:49134` | iii engine WebSocket. Also read from the `III_URL` env var (the flag wins). A pre-connect probe logs one ERROR with a fix hint when the engine is unreachable; the SDK then retries forever with a 2s backoff. |
| `--version` | — | print the worker version |

Logging is controlled by the `RUST_LOG` env var (tracing `EnvFilter` syntax; default `info`).

## Configuration

The shell worker integrates with the central `configuration` worker rather than reading a static file at runtime:

1. On boot it registers a schema with id `shell`; the YAML at `--config <path>` (default `./config.yaml`) is sent as the `initial_value` (populates the first-boot default). If the file is missing or unreadable the worker warns and, when no value is stored yet, seeds the built-in zero-config default (`ShellConfig::seed_default()`) so it still boots.
2. It immediately fetches the live value over RPC and activates the security policy and fs backend from that response.
3. It then registers the `configuration:updated` trigger and runs a **fail-closed** boot reconcile before exposing any public function. The reconcile re-fetches the authoritative value (closing the race where an update lands between the initial fetch and trigger registration, leaving no listener). If that re-fetch fails the worker aborts startup — it exits rather than serve a possibly stale security policy, and no `shell::*` / `shell::fs::*` function is ever exposed.
4. It subscribes to `configuration:updated` events. When the config for schema id `shell` changes, the worker hot-reloads the security policy and fs backend atomically.
5. If the incoming config is invalid or unsafe (e.g. schema validation passes but the worker cannot build it — bad denylist regex, unreachable jail root), the worker keeps the last-good runtime and logs an error — it does **not** crash, and it does **not** retry (re-fetching returns the same bad value, so a retry would storm). The rejection is recorded and surfaced by `shell::config-status` (a `rejected` outcome with a non-zero `rejected_reloads` count) so the divergence between the central store and the enforced policy is detectable instead of silent.
6. A reload that widens the jail (clearing `host_roots`) succeeds, but is logged as a privilege change.

## Full YAML defaults

These are the CODE defaults (`ShellConfig::default()` — fail-closed: `env.inherit
false`, unjailed refused unless explicitly opted in). The shipped seed
`config.yaml` / `seed_default()` is deliberately more permissive for dev use:
`env.inherit true`, unjailed but with the opt-in given explicitly
(`fs.allow_unjailed: true`), `max_timeout_ms 120000`, catastrophic-only
denylist.

| key | default | enforced where |
|-----|---------|----------------|
| `max_timeout_ms` | `30000` | foreground `exec` hard cap; per-call `timeout_ms` clamped to this |
| `max_bg_timeout_ms` | `0` | host bg job hard cap in ms; `0` = unbounded (separate from `max_timeout_ms`, which bounds foreground exec) |
| `default_timeout_ms` | `10000` | applied when caller omits `timeout_ms` |
| `max_output_bytes` | `1048576` (1 MiB) | stdout/stderr truncated; `*_truncated` flagged |
| `working_dir` | `null` | pins cwd for spawned commands when set |
| `env.inherit` | `false` | forward the worker's FULL env to children; when `false`, only `env.allow` keys are forwarded |
| `env.allow` | `[PATH, HOME, LANG, LC_ALL, TERM]` | forwarding allowlist when `env.inherit` is false. No effect on the per-call `env` override, which is deny-only (gated solely by the hardcoded dangerous-key list) |
| `denylist_patterns` | `[]` | advisory regex tripwire on `argv.join(" ")` |
| `max_concurrent_jobs` | `16` | rejects new `exec_bg` past the cap |
| `job_retention_secs` | `3600` | finished jobs evicted by a background reaper (interval `min(30s, retention/2)`) — the primary prune path; prune-on-`shell::list` remains as a harmless secondary trigger |
| `fs.host_roots` | `[]` | jail roots; first = primary; required non-empty unless `fs.allow_unjailed: true` |
| `fs.allow_unjailed` | `false` | explicit opt-in to running with an empty `host_roots` |
| `fs.max_read_bytes` | `0` (unlimited) | pre-flight cap via `fs::metadata` (`S218`) |
| `fs.max_write_bytes` | `0` (unlimited) | mid-stream cap during write (`S218`) |
| `fs.denylist_paths` | `[]` | absolute-prefix denylist; rejected with `S215` |
| `fs.allow_special_bits` | `false` | setuid/setgid/sticky bits in `mkdir`/`chmod`/`write` modes are rejected with `S210` unless `true` |
| `sandbox.enabled` | `true` | `false` → every sandbox-target call returns `S210` |

## Threat model

The host backend's path-validation gate is check-then-use: there is a TOCTOU window between validation and the `std::fs::*` call. Validation walks to the longest existing ancestor, canonicalizes that (resolving symlinks in the existing portion), and lexically collapses the non-existent tail before the jail-root containment check — so a symlink whose target escapes the jail cannot slip through the lexical fallback. The worker is intended for trusted caller pipelines; for untrusted input, use the sandbox backend.

Host-targeted calls run with the shell worker's OS permissions. The denylist is regex over `argv.join(" ")` and only catches honest typos — a caller invoking a shell or interpreter (`sh`, `node`, `python`, …) can bypass it by construction. The actual security boundary is `target: { kind: "sandbox", sandbox_id }`.

Command policy is deny-only: the shell never decides which commands are *allowed* — that (ask/allow) layer is the approval-gate's; the shell only refuses catastrophic patterns.

## Streaming wire shapes

`shell::fs::write` and `shell::fs::read` use `iii_sdk::channels::StreamChannelRef` instead of inline base64. The other eight `fs::*` ops are unchanged.

### `shell::fs::write` request

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

### `shell::fs::read` response

Request: `{ "target": ..., "path": "/abs/path" }`.

```jsonc
{
  "content": { "channel_id": "...", "access_key": "...", "direction": "read" },
  "size": 1234,
  "mode": "0644",
  "mtime": 1714780800
}
```

```rust
let resp: ReadResponse = iii.trigger("shell::fs::read", json!({
    "target": { "kind": "host" }, "path": "/tmp/x"
})).await?;
let reader = ChannelReader::new(iii.address(), &resp.content);
let bytes = reader.read_all().await?;
```

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `fs.host_roots is empty and fs.allow_unjailed is false — refusing to start unjailed` | Default config no longer permits running unjailed. | Set `fs.host_roots` to at least one directory, OR set `fs.allow_unjailed: true`. |
| Worker never connects to engine | Engine isn't running or isn't bound on the URL the worker is configured for. | Start the engine first; check `--url` matches. The default WS port is 49134. |
| Engine started but doesn't see the worker | Binary isn't symlinked at `~/.iii/workers/shell`. | `ln -sfn $(pwd)/target/release/iii-shell ~/.iii/workers/shell` |
| `S215 path escapes the fs jail roots` on a path inside the jail | A symlink in the path resolves outside the jail. | Resolve the symlink yourself, or move the target inside a jail root. |

## Tests

- `tests/e2e/` — TypeScript harness. The default suite (`run-tests.sh`) covers happy paths, safety guardrails, jobs lifecycle, fs across host and sandbox targets, adversarial protocol-break suites for streaming/exec/jobs/encoding/concurrency, plus vulnerability-regression cases under `cases-vuln-repro.ts`. The jailed suite (`run-tests-jailed.sh`, against `config-jailed.yaml`) covers the symlink-parent jail-escape regression. Case count drifts as cases are added/removed — treat `run-tests.sh`'s own summary line (or `reports/report.json`'s `total`) as ground truth rather than a number in this doc.
- `tests/*.rs` — Rust integration tests (`jobs_lifecycle`, `host_fs_branches`, `sandbox_dispatch`, `function_handlers`) covering the host backend branches, sandbox forwarder, and every typed-registration handler. Run with `cargo test`.
- Line coverage measured with `cargo tarpaulin` sits around 65%; `jobs.rs` is at 100% and the sandbox dispatch path is fully exercised.

## What this is NOT

- **Not a PTY.** Interactive shells, TUIs, password prompts all break.
- **Not an isolation boundary itself.** Host-targeted calls run with the shell worker's OS permissions. For process isolation, set `target: { kind: "sandbox", sandbox_id }` — that path forwards through `iii-sandbox`'s microVM. The denylist still applies on top of either backend.
- **Not a streaming surface for `exec`.** Foreground `shell::exec` returns once the process exits and stdout/stderr are captured whole. Live streaming is `shell::exec_stream` (deferred).
- **Not per-caller-isolated.** The JOBS registry is a process-wide singleton. `shell::list` redacts argv/stdout/stderr to limit blast radius; full records are cap-gated by `job_id`.

## Deferred

- `shell::exec_stream` — live stdout/stderr via iii Streams (for long-running commands). Next iteration.
- Per-caller JOBS scoping (would replace the redact-summary-only mitigation in `shell::list` with proper isolation).
