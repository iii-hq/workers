# shell — architecture and operator notes

The published `README.md` and `skill.md` for this worker are rendered from `docs/`. This file holds the operator/contributor material that does not belong in the published surfaces — full configuration table, threat model, wire shapes for the streaming functions, troubleshooting, tests, and deferred work.

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

# 4. Start the engine (it spawns the worker). Pin a host_root or set
#    fs.allow_unjailed: true in config.yaml first — the worker refuses to
#    start unjailed by default.
iii -c ./config.yaml
```

`iii worker add shell` does not currently pull `iii-sandbox` along — run `iii worker add iii-sandbox` separately before using `shell::exec { target: sandbox }` or any `shell::fs::*` sandbox-target path. Plain host-targeted `shell::exec` works without it.

## CLI flags

| flag | default | purpose |
|------|---------|---------|
| `--config <path>` | `./config.yaml` | YAML config (shape below) |
| `--url <ws-url>` | `ws://127.0.0.1:49134` | iii engine WebSocket |
| `--manifest` | off | print the JSON function manifest and exit (use for tooling/introspection) |

## Full YAML defaults

| key | default | enforced where |
|-----|---------|----------------|
| `max_timeout_ms` | `30000` | hard cap; per-call `timeout_ms` clamped to this |
| `default_timeout_ms` | `10000` | applied when caller omits `timeout_ms` |
| `max_output_bytes` | `1048576` (1 MiB) | stdout/stderr truncated; `*_truncated` flagged |
| `working_dir` | `null` | pins cwd for spawned commands when set |
| `inherit_env` | `false` | when `false`, only `allowed_env` keys are forwarded |
| `allowed_env` | `[PATH, HOME, LANG, LC_ALL, TERM]` | env passthrough allowlist |
| `max_concurrent_jobs` | `16` | rejects new `exec_bg` past the cap |
| `job_retention_secs` | `3600` | finished jobs pruned on every `shell::list` |
| `fs.host_root` | `null` | jail root; required unless `fs.allow_unjailed: true` |
| `fs.allow_unjailed` | `false` | explicit opt-in to running with `host_root: null` |
| `fs.max_read_bytes` | `0` (unlimited) | pre-flight cap via `fs::metadata` (`S218`) |
| `fs.max_write_bytes` | `0` (unlimited) | mid-stream cap during write (`S218`) |
| `fs.denylist_paths` | `[]` | absolute-prefix denylist; rejected with `S215` |
| `sandbox.enabled` | `true` | `false` → every sandbox-target call returns `S210` |

## Threat model

The host backend's path-validation gate is check-then-use: there is a TOCTOU window between validation and the `std::fs::*` call. Validation walks to the longest existing ancestor, canonicalizes that (resolving symlinks in the existing portion), and lexically collapses the non-existent tail before the `starts_with(host_root)` check — so a symlink whose target escapes the jail cannot slip through the lexical fallback. The worker is intended for trusted caller pipelines; for untrusted input, use the sandbox backend.

Host-targeted calls run with the shell worker's OS permissions. The shell does not enforce command-level policy — that lives in approval-gate's rules layer (see `approval-gate/src/intercept.rs`). The actual isolation boundary is `target: { kind: "sandbox", sandbox_id }`.

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
| `fs.host_root is unset and fs.allow_unjailed is false — refusing to start unjailed` | Default config no longer permits running unjailed. | Set `fs.host_root` to a directory, OR set `fs.allow_unjailed: true`. |
| Worker never connects to engine | Engine isn't running or isn't bound on the URL the worker is configured for. | Start the engine first; check `--url` matches. The default WS port is 49134. |
| Engine started but doesn't see the worker | Binary isn't symlinked at `~/.iii/workers/shell`. | `ln -sfn $(pwd)/target/release/iii-shell ~/.iii/workers/shell` |
| `S215 path escapes host_root` on a path inside the jail | A symlink in the path resolves outside the jail. | Resolve the symlink yourself, or move the target inside `host_root`. |

## Tests

- `tests/e2e/` — TypeScript harness. The default suite (`run-tests.sh`) covers happy paths, remaining safety guardrails (timeout, output truncation, env scrubbing, fs path denylist), jobs lifecycle, fs across host and sandbox targets, adversarial protocol-break suites for streaming/exec/jobs/encoding/concurrency, plus vulnerability-regression cases under `cases-vuln-repro.ts`. The jailed suite (`run-tests-jailed.sh`, against `config-jailed.yaml`) covers the symlink-parent jail-escape regression.
- `tests/*.rs` — Rust integration tests (`jobs_lifecycle`, `host_fs_branches`, `sandbox_dispatch`, `function_handlers`) covering the host backend branches, sandbox forwarder, and every typed-registration handler. Run with `cargo test`.
- Line coverage measured with `cargo tarpaulin` sits around 65%; `jobs.rs` is at 100% and the sandbox dispatch path is fully exercised.

## What this is NOT

- **Not a PTY.** Interactive shells, TUIs, password prompts all break.
- **Not an isolation boundary itself.** Host-targeted calls run with the shell worker's OS permissions. For process isolation, set `target: { kind: "sandbox", sandbox_id }` — that path forwards through `iii-sandbox`'s microVM. Command-level policy is enforced upstream by approval-gate regardless of target.
- **Not a streaming surface for `exec`.** Foreground `shell::exec` returns once the process exits and stdout/stderr are captured whole. Live streaming is `shell::exec_stream` (deferred).
- **Not per-caller-isolated.** The JOBS registry is a process-wide singleton. `shell::list` redacts argv/stdout/stderr to limit blast radius; full records are cap-gated by `job_id`.

## Deferred

- `shell::exec_stream` — live stdout/stderr via iii Streams (for long-running commands). Next iteration.
- Per-caller JOBS scoping (would replace the redact-summary-only mitigation in `shell::list` with proper isolation).
