# Changelog

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
