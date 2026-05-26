# coder

A path-jailed code worker for iii agents. `coder::*` lets agents read,
search, edit, create, and delete files inside a single configured
`base_path` — without ever escaping it via `..`, absolute paths, or
symlinks. A glob-based `non_accessible` list keeps sensitive files
(`.env`, `*.pem`, anything under `secrets/`) visible to directory
listings but unreadable and unwritable.

## Install

```bash
iii worker add coder
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next
`iii start`.

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
| `coder::read-file` | Read a single file (capped at `max_read_bytes`). |
| `coder::search` | Search file contents (literal/regex) and/or paths under `base_path`. |
| `coder::update-file` | Apply batched `insert` / `remove` / `update_lines` / regex `replace` ops across one or more files. Line ops bottom-up; atomic per file. |
| `coder::create-file` | Create one or more files with `overwrite` and `parents` flags. |
| `coder::delete-file` | Remove one or more paths; `recursive: true` required for non-empty dirs. |
| `coder::list-folder` | Paginated single-folder listing; non-accessible entries flagged. |
| `coder::tree` | Recursive snapshot bounded by `max_depth` and `per_folder_limit`. |

### `coder::update-file` semantics

Line ops (`insert`, `remove`, `update_lines`) use **1-based inclusive**
line numbers and are applied **bottom-up** (highest affected line
first), so each op still references the original line numbers from the
caller's perspective. Overlapping line ops are rejected (`C210`).
Regex `replace` ops run after line ops on the full file body. The
whole batch is committed via a sibling temp file + rename, so a failure
mid-write leaves the original file intact.

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

All errors return as JSON strings of the form `{"code":"C2xx","message":"..."}`.

| Code | Meaning |
|---|---|
| `C210` | Bad input (malformed payload, illegal line numbers, overlapping ops, absolute path, …) |
| `C211` | Path not found OR matches a `non_accessible_globs` entry |
| `C213` | File exceeds `max_read_bytes` or `max_write_bytes` |
| `C215` | Path escapes `base_path` lexically or through a symlink |
| `C216` | Underlying I/O error |
| `C217` | `coder::create-file` saw an existing file with `overwrite=false` |

## Configuration

```yaml
base_path: ./                                # root every coder::* call is scoped under
non_accessible_globs:                        # listable but unreadable/unwritable
  - "**/.env"
  - "**/.env.*"
  - "**/*.pem"
  - "**/*.key"
  - "**/secrets/**"
max_read_bytes: 10485760                     # per-file read cap (10 MiB)
max_write_bytes: 10485760                    # per-file create/update cap (10 MiB)
tree_default_depth: 4                        # coder::tree depth when unset
tree_per_folder_limit: 50                    # children before tree truncates a folder
list_default_page_size: 100                  # coder::list-folder default page size
list_max_page_size: 1000                     # hard cap on coder::list-folder page_size
search_default_max_matches: 1000             # coder::search match cap
search_default_max_line_bytes: 4096          # per-line cap when scanning content
```

`non_accessible_globs` uses the same syntax as the `globset` crate (so
`**/`, `*`, `?`, character classes, …). Matching is done against the
*relative path* from `base_path`, so `**/.env` blocks `.env`,
`a/.env`, and `a/b/.env`.

## Security boundary

- `base_path` is canonicalised at startup; the worker refuses to start
  if it can't be reached.
- Every wire path must be **relative** to `base_path`; absolute paths
  return `C210` rather than being silently re-jailed.
- `..` and symlinks are resolved against the longest existing ancestor
  and rejected if they leave `base_path` (`C215`). Dangling symlinks
  in the tail are also rejected because the kernel would otherwise
  follow them on the next syscall.
- Non-accessible globs apply to reads as well as writes — the same
  glob hides the file from `coder::read-file`, `coder::update-file`,
  `coder::create-file`, `coder::delete-file`, and from
  `coder::search`'s content/path matches.
- Recursive `coder::delete-file` refuses to descend through a subtree
  that contains a non-accessible entry rather than removing it.
