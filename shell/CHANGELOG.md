# Changelog

## 0.7.0

Environment-variable DX overhaul: one consolidated `env` config block, a
self-documenting config schema, and a discoverable operator surface for the
binary itself.

### Breaking
- **`inherit_env` and `allowed_env` are replaced by a nested `env` block**
  (`env.inherit`, `env.allow`) — no legacy aliases. The old top-level keys are
  **rejected at parse** with a migration hint naming the new keys. This is
  deliberate fail-closed behavior: serde ignores unknown fields, so accepting
  the old shape would silently boot with `env.inherit false` and stop
  forwarding the worker's environment to children.

### Added
- `--version` prints the worker version.
- `--url` is documented in `--help`, including the `III_URL` env var binding.
- Pre-connect reachability probe: when the engine is unreachable at boot, one
  ERROR names the URL and the fix (`is the iii engine running? Set --url or
  III_URL...`) before the SDK's silent 2s-backoff retry loop takes over. The
  worker still never exits.
- Every operator-visible config field (including the nested `env`, `fs`, and
  `sandbox` blocks) now carries a schema description, so the console
  configuration UI documents each knob inline. Pinned by a unit test.
- A `## Running` README section documents the binary's full operator surface
  (`--config`, `--url`/`III_URL`, `--version`, `RUST_LOG`).

### Migration
```yaml
# 0.6.x                                # 0.7.0
inherit_env: true                      env:
allowed_env: [PATH, HOME, LANG]          inherit: true
                                         allow: [PATH, HOME, LANG]
```
- A stored configuration value (id `shell`) still carrying the old keys makes
  the worker fail closed at boot with the hint above. Rewrite it via
  `configuration::set` with the nested shape.
- **Order matters**: deploy the 0.7.0 binary FIRST, then rewrite the stored
  value. Writing the new shape while 0.6.x is still running makes the old
  worker hot-reload it, ignore the unknown `env` block, and silently stop
  forwarding env until restart.

## 0.6.0

The standalone `coder` worker is folded into `shell`. There is now ONE worker,
ONE config, and ONE jail for both command execution and code-file editing.

### Added
- The `coder::*` code surface (`info`, `read-file`, `search`, `list-folder`,
  `tree`, `create-file`, `update-file`, `delete-file`, `move`) is served by the
  shell worker, over the same jail as `shell::fs::*`. Function ids and `C2xx`
  error codes are unchanged.
- **Multi-root jail:** `fs.host_roots` (a list). `fs.host_root` is kept as a
  one-entry legacy alias; setting both is a config error. Relative paths anchor
  at the primary (first) root; absolute paths are accepted inside any root.
- **Unified protected paths:** `code.non_accessible_globs` is honored by BOTH
  surfaces — the code functions show-but-lock (`C211`), `shell::fs::*`
  hard-rejects (`S215`) — so secrets are declared once. `fs.denylist_paths`
  (absolute-prefix) remains as a separate hard layer.
- The exec allowlist guard now rejects a command path inside ANY writable root.
- One-shot, never-widen, idempotent migration of an existing `coder`
  configuration entry into the `shell` value at boot (best-effort, non-fatal).

### Changed
- The unbounded code read/scan handlers (`tree`, `search`, `list-folder`,
  `read-file`) run off the executor via `spawn_blocking`.
- Agent prompts route code-file work through `coder::*` **on the shell worker**
  (no separate registry install).

### Migration
- `iii worker add shell` now brings the whole surface; the standalone `coder`
  worker is retired. An existing `coder` config entry is folded into `shell`
  automatically on first boot and left intact as a rollback artifact.
- After deploying, run `iii worker restart shell` (the source watcher does not
  always restart the VM process).
