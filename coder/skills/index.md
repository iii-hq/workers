---
type: index
title: coder
---

# coder

A path-jailed code worker for iii agents. Every `coder::*` call resolves
its `path` argument relative to a single configured `base_path` and
refuses anything that would escape it — absolute inputs, `..` segments,
or crafted symlinks all return an error rather than being silently
re-jailed. A glob-based `non_accessible_globs` list keeps sensitive
files (`.env`, `*.pem`, anything under `secrets/`) visible to directory
listings but unreadable, unwritable, and unsearchable. One worker, one
namespace, one security boundary.

The surface covers the full read-explore-edit cycle: `coder::tree` and
`coder::list-folder` for navigation, `coder::search` for content/path
discovery, `coder::read-file` for inspection, and the batched
`coder::create-file` / `coder::update-file` / `coder::delete-file`
mutators for writes. Write functions commit atomically per file (temp
file + rename) so a mid-write failure leaves the original intact.

- **Files** (`coder::*`) — read, search, explore, create, edit, and
  delete files and folders, all scoped to `base_path` and screened by
  `non_accessible_globs`. Caps on per-file read/write bytes, per-folder
  listing pages, and search match counts are operator-configured.

## How-tos

### `coder::*`

- [`coder::tree`](iii://coder/coder) — recursive snapshot bounded by `max_depth` and `per_folder_limit`; reach for it first on an unfamiliar repo.
- [`coder::list-folder`](iii://coder/coder) — paginated single-folder listing sorted by name; non-accessible entries appear with `non_accessible: true`.
- [`coder::search`](iii://coder/coder) — literal or regex search over file content and/or paths with include/exclude globs.
- [`coder::read-file`](iii://coder/coder) — read a single file as UTF-8 plus `size` / `mode` / `mtime`, capped by `max_read_bytes`.
- [`coder::create-file`](iii://coder/coder) — batched file creation with per-entry `overwrite` and `parents` flags.
- [`coder::update-file`](iii://coder/coder) — batched `insert` / `remove` / `update_lines` / regex `replace` ops; line ops are 1-based, inclusive, applied bottom-up; atomic per file.
- [`coder::delete-file`](iii://coder/coder) — batched removal; `recursive: true` required for non-empty directories, missing paths are idempotent successes.
