# coder

A path-jailed code worker for iii agents. `coder::*` lets agents read,
search, edit, create, and delete files inside one or more configured
allowed roots — without ever escaping them via `..` or symlinks. A
glob-based `non_accessible` list keeps sensitive files (`.env`, `*.pem`,
anything under `secrets/`) visible to directory listings but unreadable
and unwritable.

## Install

```bash
iii worker add coder
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next
`iii start`.

## Skills

Install the `coder` agent skill for Claude Code, Cursor, and 30+ other agents:

```bash
npx skills add iii-hq/workers --skill coder
```

Browse or install every worker skill at once:

```bash
npx skills add iii-hq/workers --list
npx skills add iii-hq/workers --all
```

## Quickstart

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    // Create a file.
    iii.trigger(TriggerRequest {
        function_id: "coder::create-file".into(),
        payload: json!({
            "files": [{
                "path": "notes.md",
                "content": "# notes\n- one\n- two\n",
                "overwrite": false
            }]
        }),
        action: None,
        timeout_ms: Some(5_000),
    }).await?;

    // Apply two ops bottom-up in a single batch.
    iii.trigger(TriggerRequest {
        function_id: "coder::update-file".into(),
        payload: json!({
            "files": [{
                "path": "notes.md",
                "ops": [
                    { "op": "insert", "at_line": 2, "content": "draft" },
                    { "op": "update_lines", "from_line": 3, "to_line": 3, "content": "- ONE" }
                ]
            }]
        }),
        action: None,
        timeout_ms: Some(5_000),
    }).await?;

    // Read it back.
    let read = iii.trigger(TriggerRequest {
        function_id: "coder::read-file".into(),
        payload: json!({ "path": "notes.md" }),
        action: None,
        timeout_ms: Some(5_000),
    }).await?;
    println!("{read:#?}");

    Ok(())
}
```

## Functions

| Function id | What it does |
|---|---|
| `coder::info` | Discover the jail: canonical allowed roots, per-file size caps, response budgets, listing/search limits, `default_exclude_globs`, and non-accessible glob patterns. Call first when unsure where coder may read or write. |
| `coder::read-file` | Read a single file: `stat: true` probe (size + `total_lines`, no content), a streamed `line_from`/`line_to` window (`numbered: true` prefixes absolute line numbers), or a full read budgeted by `max_output_bytes`. Batch mode: pass `paths[]` to read multiple files in one call against a shared `batch_read_budget_bytes` cap; batch `stat` probes are budget-free. |
| `coder::search` | Search file contents (literal/regex) and/or paths inside the allowed roots. `context_lines_before`/`context_lines_after` (≤10) attach surrounding lines per match; responses are bounded by `max_matches` and `search_response_budget_bytes` (`truncated: true`, never an error). Noise dirs are skipped by default (`default_exclude_globs`). |
| `coder::update-file` | Apply batched `insert` / `remove` / `update_lines` / regex `replace` (`dot_matches_newline`, `expect_matches`) ops across one or more files. Line ops bottom-up; per-file atomic. Each applied op returns a bounded post-apply echo. |
| `coder::create-file` | Create one or more files with `overwrite` and `parents` flags. |
| `coder::delete-file` | Remove one or more paths; `recursive: true` required for non-empty dirs. |
| `coder::move` | Move or rename one or more paths; same-root moves are per-file atomic; cross-root file moves use copy+delete with rollback. |
| `coder::list-folder` | Paginated single-folder listing; entries carry `name` only — join onto the response's top-level `path`; non-accessible entries flagged. |
| `coder::tree` | Recursive snapshot bounded by `max_depth` and `per_folder_limit`. Nodes carry `name` only (child path = parent path + "/" + name; the root node's path is the response's top-level `path`). Noise dirs surface as childless `truncated` stubs; pass `use_default_excludes: false` to look inside. |

### `coder::update-file` semantics

Line ops (`insert`, `remove`, `update_lines`) use **1-based inclusive**
line numbers and are applied **bottom-up** (highest affected line
first), so each op still references the original line numbers from the
caller's perspective. Overlapping line ops are rejected (`C210`).
Regex `replace` ops run after line ops on the full file body;
`dot_matches_newline: true` lets `.` span newlines (multi-line regions
via two short anchors joined by `.*?`), and `expect_matches: N` is a
pre-write guard that fails the entry with `C210` when the actual match
count differs (`0` asserts absence). `replacement` expands capture
references (`$1`, `$name`, `${name}`); a literal `$` must be written
`$$` (JS/TS template literals in a replacement are the classic
collision: `` `Hello, $${name}!` `` outputs `` `Hello, ${name}!` ``),
and a reference to a group the pattern does not define fails pre-write
with `C210` — nothing is written. The whole batch is committed via a
sibling temp file + rename, so a failure mid-write leaves the original
file intact.

On success, each applied line op echoes a bounded post-apply window
(±2 context lines). Regex replace ops return up to 5 per-match-site
echoes, each showing the first and last line of its post-replace
region (inner line count reported via `elided`). An
`echoes_truncated` flag is set when the budget is exhausted before all
echoes could be returned.

```jsonc
{
  "files": [{
    "path": "schema.sql",
    "ops": [
      { "op": "insert",       "at_line": 1, "content": "-- header\n-- v2" },
      { "op": "remove",       "from_line": 5, "to_line": 12 },
      { "op": "update_lines", "from_line": 30, "to_line": 30, "content": "PRIMARY KEY (id)" },
      { "op": "replace",      "pattern": "OLD_", "replacement": "NEW_" }
    ]
  }]
}
```

### Error codes

All errors return as JSON objects of the form `{"code":"C2xx","message":"..."}`.

| Code | Meaning |
|---|---|
| `C210` | Bad input (malformed payload, illegal line numbers, overlapping ops, `expect_matches` mismatch, undefined `$` capture reference in a replacement, conflicting config keys, …) |
| `C211` | Path not found OR matches a `non_accessible_globs` entry |
| `C213` | File exceeds `max_read_bytes` or `max_write_bytes`; full read exceeds `max_output_bytes` (carries size/lines/recovery); batch budget exhausted |
| `C215` | Path is outside every allowed root (lexically or through a symlink); names all roots + recovery |
| `C216` | Underlying I/O error |
| `C217` | `coder::create-file` or `coder::move` saw an existing file with `overwrite=false` |

## Configuration

As of 0.5.0 coder's runtime config lives in the **`configuration` worker**
under id **`coder`** (the same pattern `database` and `storage` use). At boot
coder registers its JSON Schema, reads the live value via `configuration::get`
(the configuration worker env-expands `${VAR}`), and binds a `configuration`
trigger so it re-fetches on change. Persisted values default to
`./data/configuration/coder.yaml` — edit that file directly or call
`configuration::set id=coder`; both propagate without re-reading the seed file.

```bash
iii trigger configuration::get id=coder
iii trigger configuration::set id=coder value='{"base_paths":["/srv/project"],"max_read_bytes":20971520}'
```

### `--config` is a first-register seed

Pass `--config <path>` to supply a YAML seed file. When present AND no value is
yet stored for id `coder`, its contents are passed as `initial_value` on
`configuration::register`. After that first register the stored value is
authoritative — re-running with `--config` does NOT overwrite it. With no seed
and no stored value coder runs on the built-in default jail (`base_paths`
`["./", "/tmp"]`, **no** `non_accessible_globs` — seed the shipped
[`config.yaml`](config.yaml) to keep secret-file protection). See
[`config.yaml.example`](config.yaml.example). `${VAR}` placeholders are expanded
only when the seed file is read; `configuration::get` values are already
expanded, so they are never expanded twice.

### Reload policy

coder splits its keys the way `storage` splits its topology — the security
**jail** is restart-only, the numeric tuning knobs hot-reload:

- **JAIL fields — RESTART-REQUIRED**: `base_paths` (and legacy `base_path`),
  `non_accessible_globs`, `default_exclude_globs`. These four are everything
  the `PathResolver` compiles. On a `configuration:updated` event coder
  re-fetches the authoritative value (it never trusts the trigger payload); if
  the change alters any jail field it is **refused** — the running jail is kept
  and coder logs `restart coder to apply`. The `PathResolver` is built once at
  boot and is never rebuilt at runtime. `coder::info` keeps reporting the
  boot-time roots until restart.
- **HOT-RELOADABLE — numeric budgets/limits**: `max_read_bytes`,
  `max_write_bytes`, `tree_default_depth`, `tree_per_folder_limit`,
  `list_default_page_size`, `list_max_page_size`, `search_default_max_matches`,
  `search_default_max_line_bytes`, `search_response_budget_bytes`,
  `batch_read_budget_bytes`, `max_output_bytes`. When the jail signature is
  unchanged, coder swaps the config snapshot live and handlers read the current
  snapshot on their next call. Invalid configs are rejected and the previous
  snapshot is kept.

### Value shape

```yaml
base_paths: ["./", "/tmp"]                   # allowed roots; first entry is the primary root
non_accessible_globs:                        # listable but unreadable/unwritable
  - "**/.env"
  - "**/.env.*"
  - "**/*.pem"
  - "**/*.key"
  - "**/secrets/**"
default_exclude_globs:                       # noise filter for tree/search (hide-only)
  - "**/.git/**"
  - "**/node_modules/**"
  - "**/target/**"
  - "**/dist/**"
  - "**/.venv/**"
  - "**/__pycache__/**"
max_read_bytes: 10485760                     # per-file read cap (10 MiB)
max_write_bytes: 10485760                    # per-file create/update cap (10 MiB)
tree_default_depth: 4                        # coder::tree depth when unset
tree_per_folder_limit: 50                    # children before tree truncates a folder
list_default_page_size: 100                  # coder::list-folder default page size
list_max_page_size: 1000                     # hard cap on coder::list-folder page_size
search_default_max_matches: 1000             # coder::search match cap
search_default_max_line_bytes: 4096          # per-line cap when scanning content
search_response_budget_bytes: 262144         # byte budget per search response (256 KiB)
batch_read_budget_bytes: 1048576             # aggregate cap for paths[] batch reads (1 MiB)
max_output_bytes: 131072                     # budget for single-path FULL reads (128 KiB)
```

`base_paths` is a list of allowed roots. The first entry is the
**primary root**: relative wire paths resolve against it. Absolute wire
paths are accepted when they canonicalize inside any listed root; outside
all roots → `C215`. Default when neither `base_paths` nor the legacy
`base_path` is set: `["./", "/tmp"]`. Legacy `base_path: <dir>` is still
honored as a one-entry list; setting both keys is a startup error
(`C210`).

`non_accessible_globs` uses the same syntax as the `globset` crate (so
`**/`, `*`, `?`, character classes, …). Matching is done against the
path *relative to its containing root*, so `**/.env` blocks `.env`,
`a/.env`, and `a/b/.env` in every allowed root.

`default_exclude_globs` (same syntax and root-relative matching as
`non_accessible_globs`) is a **hide-only** noise filter: `coder::tree`
skips descent into matching directories (they appear as childless nodes
flagged `truncated` with reason `"default_exclude"`) and `coder::search`
omits matching paths. It grants no access protection — that remains
`non_accessible_globs`. Callers opt out per call with
`use_default_excludes: false`. Default: `.git`, `node_modules`,
`target`, `dist`, `.venv`, `__pycache__`.

`search_response_budget_bytes` bounds one `coder::search` response in
converted wire bytes (path + matched text + context lines). When the
next match would exceed the budget the search stops accumulating and
sets `truncated: true` — it degrades, it never errors. Default 256 KiB.

`max_output_bytes` budgets single-path **full** reads in
`coder::read-file` (bytes of returned content after UTF-8 sanitization,
numbered prefixes included). Oversize full reads fail with a `C213`
carrying the file's size, line count, and recovery calls; callers can
raise it per call via the `max_output_bytes` request field (clamped to
`max_read_bytes`) or switch to `line_from`/`line_to` windows. Windowed
reads and batch mode (`batch_read_budget_bytes`) are not governed by
this key. Default 128 KiB.

## Instrumentation

Count C2xx errors by code and function over time with `scripts/error-frequency.py` — accepts session export `.md` files or queries the live engine directly. See the script header for usage and the baseline (session vqrfg31f, 3× C210 → tool abandonment, pre-0.3.0).

## Security boundary

- Each allowed root is canonicalized at startup. Unreachable roots are
  skipped with a warning; if zero roots remain the worker refuses to
  start. The final canonical root list is logged at startup.
- Every wire path must be **relative** (resolves against the primary
  root and must stay inside it → `C215`) or **absolute inside an
  allowed root** (accepted as of 0.3.0 → `C215` if outside every root).
- `..` and symlinks are resolved against the longest existing ancestor
  and rejected if they leave the containing root (`C215`). Dangling
  symlinks in the tail are also rejected.
- Non-accessible globs apply to reads as well as writes — the same
  glob hides the file from `coder::read-file`, `coder::update-file`,
  `coder::create-file`, `coder::delete-file`, and from
  `coder::search`'s content/path matches.
- Recursive `coder::delete-file` refuses to descend through a subtree
  that contains a non-accessible entry.
- All responses carry canonical absolute paths so multi-root results
  are unambiguous.
- **`/tmp` posture**: `/tmp` is world-writable and shared. It is
  included in the default `base_paths` so agent tasks targeting `/tmp`
  work out of the box on a trusted bus. Operators on multi-tenant hosts
  should remove it and configure only the project root(s) they intend
  to expose.
