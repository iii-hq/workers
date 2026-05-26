---
type: how-to
functions: [coder::read-file, coder::search, coder::list-folder, coder::tree, coder::create-file, coder::update-file, coder::delete-file]
title: Read, search, edit, and manage files inside the jailed base_path
---

# When to use

The `coder::*` surface is the agent's single tool for filesystem work
inside the operator-configured `base_path`. Every call resolves its
`path` argument relative to that root, refuses anything that would
escape it (absolute inputs, `..` segments, symlinks that point out),
and screens reads + writes against `non_accessible_globs` so secret
files (`.env`, `*.pem`, `secrets/**`) stay invisible to the
content-touching functions even when they appear in directory
listings. Each function applies the same security model — callers
never have to re-check.

| Question                                              | Use this              |
|-------------------------------------------------------|-----------------------|
| Get a bird's-eye view of an unfamiliar repo           | `coder::tree`         |
| List one folder, paginated                            | `coder::list-folder`  |
| Find a string or regex across many files              | `coder::search`       |
| Read one file's contents                              | `coder::read-file`    |
| Make a new file (optionally in a fresh subtree)       | `coder::create-file`  |
| Edit existing files line-by-line                      | `coder::update-file`  |
| Remove files or directories                           | `coder::delete-file`  |

Reach for `shell::exec` (in the [`shell`](../../shell/README.md)
worker) instead when you need to run a process — build, test, format,
git, package manager, anything that spawns an executable. `coder::*`
is filesystem-only; it never shells out.

# `coder::tree`

Recursive directory snapshot, bounded so the response stays small even
for monorepo roots. Use it as the first call against an unfamiliar
codebase: one round-trip gives you the shape of the project up to
`max_depth` levels, and folders that hit `per_folder_limit` are
flagged with a `truncated` block that points you at
`coder::list-folder` for paginated drilldown.

## Inputs

```json
{
  "path":             ".",   // optional; folder relative to base_path; defaults to "."
  "max_depth":        4,     // optional; falls back to config.tree_default_depth (4)
  "per_folder_limit": 50     // optional; falls back to config.tree_per_folder_limit (50)
}
```

No fields are required — `coder::tree {}` snapshots the whole
`base_path` at default depth and per-folder limit. `path` must point
at a directory; pointing it at a file returns `C210`. The root node is
depth 0, so `max_depth: 1` lists the root's immediate children and
stops.

## Outputs

```json
{
  "root": {
    "name":           "base",                  // basename of `path`
    "path":           "",                      // relative path under base_path; "" for the root
    "kind":           "dir",                   // "file" | "dir" | "symlink" | "other"
    "size":           4096,
    "mtime":          1716470000,              // Unix epoch seconds
    "non_accessible": false,                   // omitted when false
    "children": [
      {
        "name":  "src",
        "path":  "src",
        "kind":  "dir",
        "size":  4096,
        "mtime": 1716470000,
        "children": [
          { "name": "main.rs", "path": "src/main.rs", "kind": "file", "size": 312, "mtime": 1716470000 }
        ]
      }
    ],
    "truncated": {                             // omitted when the folder fit
      "reason": "per_folder_limit",            // "per_folder_limit" | "max_depth" | "io_error"
      "shown":  50,
      "total":  237,                           // populated only when reason == "per_folder_limit"
      "hint":   "use coder::list-folder for paginated access to all entries"
    }
  }
}
```

- Children are sorted lex by `name` per folder.
- `children` is omitted on file nodes and on truncated `max_depth`
  nodes (the walk stops before reading the folder).
- `non_accessible: true` propagates from a `non_accessible_globs`
  match on the entry's relative path — the entry is still listed in
  the tree, but the content-touching functions will refuse it.
- `truncated.reason` is `"per_folder_limit"` when the folder had more
  than `per_folder_limit` children (the response carries the first
  `shown` children and the real `total`), `"max_depth"` when the walk
  hit the depth cap (no `children`, no `total`), or `"io_error"` when
  the folder couldn't be read (the `hint` carries the underlying
  message).

# `coder::list-folder`

Paginated single-folder listing. Use it when `coder::tree` returns a
truncated folder you need to enumerate fully, or when you already know
which folder you want and don't need its subtree. Non-accessible
entries are still returned with a `non_accessible: true` flag so the
agent can tell that a `.env` exists without being able to read it.

## Inputs

```json
{
  "path":      "src",   // optional; folder relative to base_path; defaults to "."
  "page":      1,       // optional; 1-based; defaults to 1
  "page_size": 100      // optional; falls back to config.list_default_page_size (100); capped at list_max_page_size (1000)
}
```

`path` must resolve to a directory; pointing it at a file returns
`C210`. `page_size` above `list_max_page_size` is silently clamped
down. `page` values past the end return `entries: []` with
`has_more: false`, not an error.

## Outputs

```json
{
  "path":      "src",
  "entries": [
    {
      "name":           ".env",
      "kind":           "file",                // "file" | "dir" | "symlink" | "other"
      "size":           512,
      "mtime":          1716470000,            // Unix epoch seconds
      "non_accessible": true                   // matched a non_accessible_globs pattern
    },
    {
      "name":           "main.rs",
      "kind":           "file",
      "size":           312,
      "mtime":          1716470000,
      "non_accessible": false
    }
  ],
  "total":     2,                              // total entries in the folder, across all pages
  "page":      1,
  "page_size": 100,
  "has_more":  false
}
```

- Entries are sorted lex by `name` (dotfiles first per Unix
  convention).
- `total` reflects the full folder, not the current page — divide by
  `page_size` to know how many pages exist.
- `has_more` is `true` when there's at least one more page after this
  one.

# `coder::search`

Combined content + path search across the whole jailed tree. Walks
`base_path`, applies the include/exclude globs, skips non-accessible
files entirely (their bytes never reach the matcher), and reports
content hits and path hits in separate arrays of one response.

## Inputs

```json
{
  "query":          "TODO",                    // required; non-empty
  "path":           ".",                       // optional; folder relative to base_path scoping the walk; default "."; must be a directory
  "regex":          false,                     // optional; default false (literal substring); when true, query is a regex::Regex pattern
  "ignore_case":    false,                     // optional; default false; applies to both literal and regex modes
  "include_globs":  ["**/*.rs"],               // optional; globs relative to base_path; empty = include everything
  "exclude_globs":  ["build/**"],              // optional; globs relative to base_path; empty = exclude nothing
  "max_matches":    1000,                      // optional; falls back to config.search_default_max_matches (1000)
  "max_line_bytes": 4096,                      // optional; falls back to config.search_default_max_line_bytes (4096)
  "search_content": true,                      // optional; default true; scan file contents
  "search_paths":   true                       // optional; default true; match the relative path itself
}
```

`query` is required and must be non-empty. At least one of
`search_content` / `search_paths` must be `true` — both `false`
returns `C210`. Glob syntax is the `globset` crate's (`**/`, `*`,
`?`, character classes); a bad glob or bad regex returns `C210` with
the offending pattern in the message.

Binary files (any NUL byte in the first read) are skipped from
content scanning. Files larger than `max_read_bytes` are also skipped
to keep the search from loading multi-GB blobs.

## Outputs

```json
{
  "content_matches": [
    {
      "path":   "src/main.rs",                 // relative to base_path
      "line":   42,                            // 1-based
      "column": 5,                             // 1-based, byte offset of the first match on the line
      "text":   "    // TODO: handle EOF"     // the full matched line, truncated to max_line_bytes
    }
  ],
  "path_matches": [
    { "path": "docs/TODO.md" }
  ],
  "truncated": false                           // true when either match list was capped at max_matches
}
```

- A line that exceeds `max_line_bytes` is truncated for both matching
  and reporting; the matcher never sees the overflowed tail.
- Each line emits at most one match (the first hit) — `truncated`
  reports per-file/per-line saturation, not per-character.
- `path` defaults to `.` (the whole jail). Set it to a subfolder
  (e.g. `"src"`) to scope the walk; globs and result `path`s remain
  anchored at `base_path` so `include_globs: ["src/**/*.rs"]` keeps
  working regardless of `path`. Pointing `path` at a file (not a
  directory) returns `C210`.
- When `truncated: true`, tighten the pattern or narrow with
  `include_globs` / `exclude_globs` rather than bumping `max_matches`
  blindly; the cap exists to keep responses bounded.

# `coder::read-file`

Read one file's bytes and metadata. Use it after `coder::search`
surfaces an interesting hit, or whenever you have a path in hand and
want the full content (not just a snippet). The single-file shape
keeps the call cheap; for many small files prefer a `coder::tree` plus
targeted reads over scanning the whole tree.

## Inputs

```json
{ "path": "src/main.rs" }                      // required; file relative to base_path
```

`path` is required. Pointing it at a directory returns `C210`;
pointing it at a non-accessible match returns `C211` (folded with
"not found" so callers can't probe). Files above `max_read_bytes`
(default 10 MiB) return `C213` before any bytes are loaded.

## Outputs

```json
{
  "path":    "src/main.rs",                    // echoed for caller correlation
  "content": "fn main() {\n    println!(\"hi\");\n}\n",
  "is_utf8": true,                             // false when invalid bytes were lossily replaced with U+FFFD
  "size":    34,                               // raw on-disk size in bytes
  "mode":    420,                              // Unix permission bits as decimal (0o644 == 420); 0o644 on non-Unix
  "mtime":   1716470000                        // Unix epoch seconds; 0 when the FS can't expose it
}
```

- `content` is always a string — binary or invalid-UTF-8 inputs are
  decoded with `String::from_utf8_lossy` and the `is_utf8: false`
  flag tells callers the byte count won't match exactly.
- `size` reflects on-disk bytes; `content.len()` may differ when
  `is_utf8: false` because of U+FFFD substitution.

# `coder::create-file`

Batched file creation. Each entry in `files[]` is treated
independently — a single bad path doesn't fail the rest of the batch;
instead its `results[i]` slot carries `success: false` and the JSON
error envelope. Use it to scaffold a fresh subtree (one call with
several entries, `parents: true`) or to write a single new file
without the read-modify-write dance of `coder::update-file`.

## Inputs

```json
{
  "files": [
    {
      "path":      "tests/foo_test.rs",        // required; relative to base_path
      "content":   "#[test]\nfn it_works() {}",// required; UTF-8 string written verbatim
      "mode":      "0644",                     // optional; octal as string; default "0644"; Unix only
      "parents":   true,                       // optional; create missing parent dirs; default true
      "overwrite": false                       // optional; refuse on existing path when false; default false
    }
  ]
}
```

`files` must contain at least one entry; `[]` returns `C210`. Per
entry, `path` and `content` are required. `parents: true` runs
`create_dir_all` on the parent before writing. `overwrite: false` on
an existing path yields `C217` in that entry's result (the rest of
the batch still runs).

## Outputs

```json
{
  "results": [
    {
      "path":          "tests/foo_test.rs",
      "success":       true,
      "bytes_written": 28
                                               // `error` omitted on success
    },
    {
      "path":          ".env",
      "success":       false,
      "bytes_written": 0,
      "error":         "{\"code\":\"C211\",\"message\":\".env: matches non_accessible_globs\"}"
    }
  ]
}
```

- One result per input entry, in input order.
- `error` is omitted on success; on failure it's the per-function
  JSON envelope (`{"code":"C2xx","message":"..."}`) the same way
  top-level errors are reported.
- `bytes_written` is `content.as_bytes().len()` on success, `0` on
  failure.

# `coder::update-file`

Batched line-oriented and regex edits. Each file in `files[]` carries
an array of ops; all ops on one file commit **atomically** via a
sibling temp file + `rename`, so a mid-write failure leaves the
original byte-identical. Across files the batch is independent — one
file's failure doesn't roll back another's.

Line numbers are **1-based and inclusive**. Line ops (`insert`,
`remove`, `update_lines`) within one file are applied **bottom-up**
(highest affected line first), so each op still references the original
line numbers the caller wrote — there's no need to recompute offsets
after an earlier op. Overlapping line ops in original-line space are
rejected up-front with `C210`. Regex `replace` ops run **after** all
line ops on the serialized file body, in declaration order, and do not
participate in line-space overlap checks.

## Inputs

```json
{
  "files": [
    {
      "path": "schema.sql",                    // required; relative to base_path
      "ops": [
        { "op": "insert",       "at_line":   1,                "content": "-- header\n-- v2" },
        { "op": "remove",       "from_line": 5,  "to_line": 12                                },
        { "op": "update_lines", "from_line": 30, "to_line": 30, "content": "PRIMARY KEY (id)" },
        { "op": "replace",      "pattern": "OLD_NAME", "replacement": "NEW_NAME"             }
      ]
    }
  ]
}
```

`files` must contain at least one entry, and each entry's `ops` must
contain at least one op — both empties return `C210`.

Each op shape:

- `insert` — insert `content` **before** `at_line` (1-based). The
  special value `at_line = lines + 1` appends to the end.
- `remove` — delete the inclusive range `from_line..=to_line`.
- `update_lines` — overwrite the inclusive range with `content` (split
  by `\n`).
- `replace` — substitute all regex matches in the file body. Fields:
  `pattern` (required regex), `replacement` (required; Rust capture
  syntax such as `$1` / `$name`), optional `ignore_case` (default
  `false`). Runs after line ops in the same batch. Empty `pattern` or
  invalid regex returns `C210`. No matches is a no-op; the op still
  counts toward `applied`.

`content` on line ops may be multi-line; lines are split on `\n` and
`\r` from CRLF inputs is trimmed. File line endings (`\n` vs `\r\n`)
and the presence/absence of a trailing newline are preserved across
the rewrite (regex `replace` operates on the joined body and may change
line count if the replacement introduces or removes newlines).

## Outputs

```json
{
  "results": [
    {
      "path":           "schema.sql",
      "success":        true,
      "applied":        3,                     // count of ops applied (only meaningful on success)
      "new_line_count": 27                     // final line count after the rewrite
                                               // `error` omitted on success
    },
    {
      "path":           ".env",
      "success":        false,
      "applied":        0,
      "new_line_count": 0,
      "error":          "{\"code\":\"C211\",\"message\":\".env: matches non_accessible_globs\"}"
    }
  ]
}
```

- One result per input file, in input order.
- `applied` and `new_line_count` are `0` on failure; on success
  `applied` equals `ops.len()` for that file.
- `error` is omitted on success; on failure it carries the per-file
  JSON error envelope.

Overlap semantics for line ops (each rejected with `C210`):

- Two `remove` / `update_lines` ranges sharing any line.
- An `insert` at a line covered by a `remove` / `update_lines` range.
- Two `insert`s at the same `at_line`.
- Any range with `from_line == 0`, `from_line > to_line`, or
  `to_line` past EOF.

# `coder::delete-file`

Batched removal. Like `create-file` and `update-file`, per-path errors
land in `results[i]` instead of failing the whole batch. Missing paths
are **idempotent successes** (`success: true, removed: false`) — safe
to retry. Directories require an explicit `recursive: true`; without
it, a non-empty directory returns an error in its result slot.

## Inputs

```json
{
  "paths":     [".cache/build", "stale.log"],  // required; non-empty; each relative to base_path
  "recursive": true                            // optional; default false; required for non-empty directories
}
```

`paths` must contain at least one entry; `[]` returns `C210`. Trying
to delete `base_path` itself (e.g. `"."` resolving to the root) is
rejected with `C210` regardless of `recursive`.

When `recursive: true`, the walk **refuses to descend through any
subtree containing a `non_accessible_globs` match**. The whole
directory remains untouched and the result reports `C211` with the
offending child — agents can't wipe out a `secrets/` folder by
deleting its parent.

## Outputs

```json
{
  "results": [
    {
      "path":    ".cache/build",
      "success": true,
      "removed": true                          // false when the path was already gone
                                               // `error` omitted on success
    },
    {
      "path":    "stale.log",
      "success": true,
      "removed": false                         // file didn't exist; treated as a no-op success
    },
    {
      "path":    "secrets",
      "success": false,
      "removed": false,
      "error":   "{\"code\":\"C211\",\"message\":\"recursive delete blocked: secrets contains non-accessible secrets/api.pem\"}"
    }
  ]
}
```

- One result per input path, in input order.
- `removed: false` with `success: true` means the path didn't exist
  at call time — the post-condition (the path is absent) is true, so
  the call is idempotent.

# Errors

All `coder::*` errors serialize as `{"code":"C2xx","message":"..."}`.
Top-level failures (e.g. an empty `files: []`) come back as the
function's own `Result::Err`; the batched functions
(`create-file`, `update-file`, `delete-file`) instead embed the same
envelope inside each `results[i].error` so a single bad path never
aborts the whole call.

| Code   | Meaning                                                                                       |
|--------|-----------------------------------------------------------------------------------------------|
| `C210` | Bad input — malformed payload, illegal line numbers, overlapping ops, absolute path, bad glob/regex, refusing to delete `base_path` itself. |
| `C211` | Path not found **or** matches `non_accessible_globs`. Folded into one code so callers can't probe for the existence of a denied file. |
| `C213` | File exceeds `max_read_bytes` (read side) or `max_write_bytes` (create/update). For `update-file` this fires on either the pre-edit size or the post-edit size. |
| `C215` | Path escapes `base_path` lexically (`..`) or via a symlink, or the symlink target dangles outside the jail. |
| `C216` | Underlying I/O error (permission denied, EIO, …). The `message` carries the OS error string. |
| `C217` | `coder::create-file` saw an existing path with `overwrite: false`. Set `overwrite: true` to replace, or pick a different `path`. |

# Worked example

A typical read-explore-edit pass: snapshot the project, find an open
TODO, read it in context, rewrite the offending block, then add a
companion test file.

1. Get the shape of the project:

   ```json
   { "path": "src" }
   ```

   Sent to `coder::tree`. If a folder comes back with
   `truncated.reason == "per_folder_limit"`, follow up with
   `coder::list-folder` against that folder.

2. Find every open TODO in the Rust sources:

   ```json
   {
     "query":         "TODO",
     "include_globs": ["**/*.rs"],
     "exclude_globs": ["build/**", "target/**"]
   }
   ```

   Sent to `coder::search`. The response carries
   `content_matches[i]` with `path`, `line`, `column`, and the
   matched line `text`.

3. Read the file around a hit:

   ```json
   { "path": "src/foo.rs" }
   ```

   Sent to `coder::read-file`. `content` is the full file as a
   string; index into it using the `line` / `column` from step 2.

4. Rewrite the block:

   ```json
   {
     "files": [{
       "path": "src/foo.rs",
       "ops": [
         { "op": "update_lines", "from_line": 42, "to_line": 45,
           "content": "    let payload = parse(input)?;\n    handle(payload)?;\n    Ok(())" }
       ]
     }]
   }
   ```

   Sent to `coder::update-file`. The response's `results[0]` carries
   `success: true`, `applied: 1`, and the new `new_line_count`.

   To rename a symbol everywhere in the file without touching line
   numbers, add a regex `replace` after any line ops:

   ```json
   { "op": "replace", "pattern": "fn old_name", "replacement": "fn new_name" }
   ```

5. Scaffold a companion test in one call (notice `parents: true`
   creates `tests/` if it isn't there yet):

   ```json
   {
     "files": [{
       "path":    "tests/foo_test.rs",
       "content": "use my_crate::foo;\n\n#[test]\nfn it_works() {\n    assert!(foo::handle(\"x\").is_ok());\n}\n",
       "parents": true,
       "overwrite": false
     }]
   }
   ```

   Sent to `coder::create-file`. If `tests/foo_test.rs` already
   exists, the result carries `C217` and the original file stays put
   — flip `overwrite: true` and resend if you meant to replace it.

# Side effects

Three functions write to disk. None of them fire engine triggers or
emit events — the only observable effect is the filesystem mutation
described below.

- `coder::create-file` writes each entry's `content.as_bytes()` to
  `base_path/path` with `std::fs::write`. When `parents: true`, runs
  `std::fs::create_dir_all` on the parent first. On Unix, applies
  `mode` (octal string parsed with `u32::from_str_radix(..., 8)`,
  masked with `0o777`); on non-Unix the `mode` field is accepted but
  ignored.
- `coder::update-file` writes via a sibling temp file named
  `<basename>.coder-tmp-<pid>-<nanos>` in the target's parent
  directory, then `std::fs::rename`s it over the original. A crash
  mid-write leaves the original byte-identical; in rare cases an
  orphan temp file may remain (it's safe to remove manually). Line
  endings (`\n` vs `\r\n`) and trailing-newline presence are
  preserved from the original file.
- `coder::delete-file` calls `std::fs::remove_file` for files and
  empty dirs, `std::fs::remove_dir_all` when `recursive: true`. The
  recursive path walks the subtree first and aborts with `C211`
  (without removing anything) if **any** descendant matches
  `non_accessible_globs` — protecting against accidentally wiping a
  `secrets/` subtree by deleting its parent.

# Related

- [`shell::exec`](../../shell/README.md) and `shell::fs::*` — when
  you need to run a process (build, test, format, git) or stream
  bytes through a channel. `coder::*` never shells out.
- [`directory::skills::get`](iii://directory/skills/get) — the
  iii-directory worker that surfaces this how-to (and others) to
  agents at bootstrap time.
- The "Security boundary" section of [coder/README.md](../README.md)
  — operator-facing detail on `base_path` canonicalisation,
  `non_accessible_globs` syntax, and the symlink rejection rules.
