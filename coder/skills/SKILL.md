---
name: coder
description: >-
  Read, search, and edit files inside path-jailed allowed roots — structured
  filesystem ops for agents, with glob-based secret protection and per-file
  atomic writes.
---

# coder

The coder worker is a path-jailed surface for filesystem work. Every `coder::*`
call is scoped to one or more operator-configured allowed roots. Call
`coder::info` first to discover the canonical allowed roots, size caps,
response budgets (`max_output_bytes`, `batch_read_budget_bytes`,
`search_response_budget_bytes`), the `default_exclude_globs` noise filter, and
non-accessible globs. Relative paths resolve against the primary root (index
0); absolute paths are accepted when they canonicalize inside any allowed root
— outside every root returns `C215`. `..` segments and escaping symlinks are
also rejected. A glob-based `non_accessible_globs` list keeps sensitive files
(`.env`, `*.pem`, anything under `secrets/`) visible to directory listings but
unreadable, unwritable, and unsearchable.

The surface covers the whole read-explore-edit cycle: navigate with
`coder::tree` and `coder::list-folder`, discover with `coder::search`, inspect
with `coder::read-file`, then mutate through the batched `coder::create-file`,
`coder::update-file`, and `coder::delete-file`. Add it with `iii worker add
coder`; operator caps and budgets live in `config.yaml`. It is filesystem-only
and never spawns a process.

## When to Use

- Get the shape of an unfamiliar repo in one round-trip (`coder::tree`), then
  drill into folders flagged as truncated (`coder::list-folder`).
- Find a string, symbol, or TODO across many files by content or path
  (`coder::search`).
- Read a file window-first: `stat: true` probe, then just the lines you need
  (`coder::read-file` — see Window-first reading).
- Make a targeted edit in TWO calls — search with context, edit directly — no
  file read in between (the 2-call edit workflow below).
- Scaffold a fresh file or subtree, or rewrite existing source line-by-line
  (`coder::create-file`, `coder::update-file`).
- Remove stale files or directories (`coder::delete-file`).

## The 2-call edit workflow

1. `coder::search {query, context_lines_before: 3, context_lines_after: 8}` —
   the context lines plus 1-based line numbers are usually all you need.
2. Edit with `coder::update-file`:
   - Regex `replace` when the region is uniquely anchorable: two short anchors
     joined by `.*?`, with `dot_matches_newline: true` and `expect_matches: 1`.
   - `update_lines` when it is not (repeated code, nothing distinctive): take
     exact coordinates from a `numbered: true` read window first.

Wildcard economy: replace large regions WITHOUT quoting them —
`"anchor_start.*?anchor_end"` with `dot_matches_newline: true`. ALWAYS prefer
wildcards over pasting the block into the pattern. `expect_matches: 1` turns a
silent multi-site clobber into a safe pre-write error; `expect_matches: 0`
asserts absence. `$` in `replacement` is a capture reference (`$1`, `${name}`)
— write a literal `$` as `$$`; JS/TS template literals are the classic trap
(write `Hello, $${name}!` to output `Hello, ${name}!`), and a reference to a
group the pattern does not define fails pre-write with `C210`. Every applied
op returns bounded post-apply echoes (line ops: region ±2 lines; replace: up
to 5 sites, each showing the first AND last line of its replaced region with
inner lines counted in `elided`) — verify from the echoes instead of
re-reading the file.

Bulk rename: `coder::search {query: "\bold_name\b", regex: true}` → ONE
`coder::update-file` call with a `\b`-anchored `replace` op per file,
`expect_matches` pinned to that file's count from the search results (in the
JSON wire payload write `\\b` — a lone `\b` is valid JSON but means the
BACKSPACE character, so the pattern silently matches nothing). Omitting
`expect_matches` replaces all matches in the file unconditionally — fine when
the search already showed you every site.

## Window-first reading

Never full-read a file you haven't probed. `stat: true` costs ~90 tokens and
returns size plus `total_lines`; then fetch only the window you need with
`line_from`/`line_to`. If `total_lines` > ~400, window it. Full reads larger
than `max_output_bytes` (default 128 KiB) refuse with a C213 that carries the
file's size, line count, and the recovery calls.

- `numbered: true` recipe: a numbered window prefixes each line with its
  ABSOLUTE 1-based file line number — the exact coordinates
  `coder::update-file` line ops take, so go straight from window to
  `update_lines`. Bottom-up application lets multiple ops in one call all use
  original line numbers.
- Poor-man's outline: `coder::search {path: "src", regex: true, query:
  "^\s*(pub |fn |class |def |func |impl |interface )", include_globs:
  ["src/config.rs"]}` returns one file's structure with line numbers — the
  `\s*` catches indented declarations (impl/class methods). `path` must be a
  folder (scope it tight); the root-relative `include_globs` pins the file.
- Batch economy: batch related reads in ONE `paths[]` call (shared
  `batch_read_budget_bytes`, 1 MiB); batch `stat` probes are budget-free;
  batched mutators are per-entry isolated, so one bad path never aborts the
  rest.

## Tree and path notes

- Nodes carry only `name`. The root node's path IS the response's top-level
  `path` — start joining at the root's children: child path = parent path +
  "/" + name. Same rule for `coder::list-folder` entries.
- Noise dirs (node_modules, .git, target, …) appear as childless `truncated`
  stubs in `coder::tree` and are omitted from `coder::search`; pass
  `use_default_excludes: false` to look inside. `coder::info` lists the
  active globs.

## Boundaries

- Not for running processes — reach for `shell::exec` / `shell::exec_bg` in the
  `shell` worker to build, test, format, or run git. `coder::*` never shells out.
- Relative paths resolve against the primary allowed root; absolute paths inside
  any allowed root are accepted (as of 0.3.0). `..` and escaping symlinks are
  rejected. Use `coder::info` to discover roots when a path is rejected.
- `non_accessible_globs` blocks reads, writes, searches, and deletes — a denied
  path is folded with "not found" so callers can't probe for its existence.
- Writes fire no engine triggers and emit no events; the only effect is the
  filesystem mutation itself.
- For host-level structured fs that can forward into a sandbox microVM, use the
  `shell` worker's `shell::fs::*` surface instead.

## Functions

- `coder::info` — the jail contract: allowed roots, size caps, budgets, limits, `default_exclude_globs`, and non-accessible globs. Call FIRST when unsure where coder may read or write.
- `coder::tree` — recursive snapshot bounded by `max_depth` and a per-folder limit; slim nodes (`name` only — join rule above); capped folders are flagged for `coder::list-folder` drilldown; noise dirs surface as childless `truncated` stubs.
- `coder::list-folder` — paginated single-folder listing sorted by name; entries carry `name` only; non-accessible entries flagged `non_accessible: true`.
- `coder::search` — literal or regex over content and/or paths with include/exclude globs; `context_lines_before`/`context_lines_after` (≤10) attach surrounding lines; capped by `max_matches` AND `search_response_budget_bytes` — on `truncated: true`, refine the query rather than paginate.
- `coder::read-file` — `stat: true` probe, `line_from`/`line_to` windows, `numbered: true` absolute line numbers; full reads budgeted by `max_output_bytes` (C213 carries size/lines/recovery); batch `paths[]` shares `batch_read_budget_bytes`.
- `coder::create-file` — batched file creation with per-entry `overwrite` and `parents` flags.
- `coder::update-file` — batched `insert` / `remove` / `update_lines` / regex `replace` (`dot_matches_newline`, `expect_matches`) ops; every applied op echoes a bounded post-apply window for read-free verification.
- `coder::delete-file` — batched removal; `recursive: true` is required for non-empty directories and missing paths are idempotent successes.
- `coder::move` — batched move/rename with per-entry `overwrite` and `parents` flags; same-root renames are per-file atomic; cross-root file moves use copy+delete with rollback; cross-root directory moves are unsupported.

The batched mutators return one result per input entry so a single bad path
never aborts the rest of the call; line ops are 1-based, inclusive, applied
bottom-up against original line numbers; every file commits per-file atomically
via temp file plus rename.
