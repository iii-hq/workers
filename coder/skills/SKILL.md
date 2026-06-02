---
name: coder
description: >-
  Read, search, and edit files inside a path-jailed base_path — structured
  filesystem ops for agents, with glob-based secret protection and atomic
  per-file writes.
---

# coder

The coder worker is a path-jailed surface for filesystem work. Every `coder::*`
call resolves its `path` argument relative to a single operator-configured
`base_path` and refuses anything that would escape it — absolute inputs, `..`
segments, and crafted symlinks all return an error rather than being silently
re-jailed. A glob-based `non_accessible_globs` list keeps sensitive files
(`.env`, `*.pem`, anything under `secrets/`) visible to directory listings but
unreadable, unwritable, and unsearchable.

The surface covers the whole read-explore-edit cycle: navigate with
`coder::tree` and `coder::list-folder`, discover with `coder::search`, inspect
with `coder::read-file`, then mutate through the batched `coder::create-file`,
`coder::update-file`, and `coder::delete-file`. Add it with `iii worker add
coder`; operator caps on per-file read/write bytes, listing pages, and search
matches live in `config.yaml`. It is filesystem-only and never spawns a process.

## When to Use

- Get the shape of an unfamiliar repo in one round-trip (`coder::tree`), then
  drill into folders flagged as truncated (`coder::list-folder`).
- Find a string, symbol, or TODO across many files by content or path
  (`coder::search`).
- Read one file's full contents after a search hit (`coder::read-file`).
- Scaffold a fresh file or subtree, or rewrite existing source line-by-line
  (`coder::create-file`, `coder::update-file`).
- Remove stale files or directories (`coder::delete-file`).

## Boundaries

- Not for running processes — reach for `shell::exec` / `shell::exec_bg` in the
  `shell` worker to build, test, format, or run git. `coder::*` never shells out.
- Paths must be relative to `base_path`; absolute inputs, `..`, and escaping
  symlinks are rejected rather than re-jailed.
- `non_accessible_globs` blocks reads, writes, searches, and deletes — a denied
  path is folded with "not found" so callers can't probe for its existence.
- Writes fire no engine triggers and emit no events; the only effect is the
  filesystem mutation itself.
- For host-level structured fs that can forward into a sandbox microVM, use the
  `shell` worker's `shell::fs::*` surface instead.

## Functions

- `coder::tree` — recursive directory snapshot bounded by `max_depth` and a per-folder limit; folders that hit the cap are flagged for paginated drilldown.
- `coder::list-folder` — paginated single-folder listing sorted by name; non-accessible entries are still listed with a `non_accessible: true` flag.
- `coder::search` — literal or regex search over file content and/or paths, with include/exclude globs; non-accessible files are skipped entirely.
- `coder::read-file` — read one file as UTF-8 plus `size` / `mode` / `mtime`, capped by `max_read_bytes`.
- `coder::create-file` — batched file creation with per-entry `overwrite` and `parents` flags.
- `coder::update-file` — batched `insert` / `remove` / `update_lines` / regex `replace` ops across one or more files.
- `coder::delete-file` — batched removal; `recursive: true` is required for non-empty directories and missing paths are idempotent successes.

The batched mutators return one result per input entry so a single bad path never aborts the rest of the call, `coder::update-file` line ops are 1-based and inclusive and applied bottom-up so each op still references the caller's original line numbers, and every file commits atomically via a temp file plus rename.
