# Changelog

## 0.5.0

Config source-of-truth moves to the `configuration` worker (the pattern
`database` and `storage` already use). `--config` becomes a first-register
seed, and the numeric tuning knobs hot-reload.

### Changed — runtime config now lives in the `configuration` worker

- **coder's config is sourced from the `configuration` worker under id
  `coder`, not from a file read on every boot.** At startup coder registers its
  JSON Schema, reads the live value via `configuration::get` (which env-expands
  `${VAR}`), and binds a `configuration` trigger that re-fetches the
  authoritative value on change (it never trusts the trigger payload). Persisted
  values default to `./data/configuration/coder.yaml`; edit that file or call
  `configuration::set id=coder` and the change propagates without re-reading the
  seed file.
- **`--config <path>` is now a first-register SEED, not the runtime source of
  truth.** When present and no value is yet stored for id `coder`, the file's
  contents are passed as `initial_value` on `configuration::register`. After the
  first register the stored value is authoritative — re-running with `--config`
  does NOT overwrite it. `${VAR}` is expanded only when the seed file is read;
  `configuration::get` values are already expanded and are never expanded twice.
- **`--config` no longer has a default and no longer auto-loads `./config.yaml`
  on every boot.** With no seed and no stored value coder runs on the built-in
  default jail (`base_paths` `["./", "/tmp"]`, **no** `non_accessible_globs`).
  Migration: deployments that relied on an on-disk `./config.yaml` being
  auto-loaded must pass it explicitly via `--config ./config.yaml` for the
  first register (to preserve the shipped secret-glob protection), or set the
  value once via `configuration::set id=coder`.

### Changed — split reload policy (jail restart-only, knobs hot-reload)

- **The security jail is restart-required; the numeric budgets/limits
  hot-reload.** On a `configuration:updated` event coder re-fetches and, if the
  change alters any JAIL field — `base_paths` (and legacy `base_path`),
  `non_accessible_globs`, `default_exclude_globs` — the reload is **refused**:
  the running jail is kept and coder logs `restart coder to apply` (the
  `PathResolver` is built once at boot and is never swapped at runtime, the same
  way `storage` refuses a topology change). When the jail signature is
  unchanged, the config snapshot is swapped live and handlers read the current
  snapshot on their next call. The hot-reloadable knobs are `max_read_bytes`,
  `max_write_bytes`, `tree_default_depth`, `tree_per_folder_limit`,
  `list_default_page_size`, `list_max_page_size`, `search_default_max_matches`,
  `search_default_max_line_bytes`, `search_response_budget_bytes`,
  `batch_read_budget_bytes`, and `max_output_bytes`. `coder::info` keeps
  reporting the boot-time roots until restart.

### Docs

- New [`config.yaml.example`](config.yaml.example) documents the
  seed-on-first-register contract, the built-in default, and the
  configuration-worker source of truth.
- New [`config.collect.yaml`](config.collect.yaml) seeds a single `/tmp` root
  so the registry-publish CI job can boot a throwaway coder and read back its
  interface without depending on the runner's working directory.
- `config.yaml` header now states it is a SEED for `initial_value`, not the
  runtime source of truth, and spells out the restart-vs-hot-reload split.
- README `## Configuration` documents the `configuration`-worker sourcing,
  `--config` seed semantics, and the reload policy.

## 0.4.1

Fix wave driven by a live harness session failure (session q8x6g248).

### Fixed — silent file corruption via undefined capture references

- **Replace ops now validate every `$` capture reference in `replacement`
  against the compiled pattern's actual groups, pre-write.** The regex crate
  expands undefined references to the EMPTY STRING: a replacement carrying a
  JS/TS template literal (`` `Hello, ${name}!` ``) against a pattern with no
  group `name` silently wrote `` `Hello, !` `` to disk with `success: true` —
  twice in the live session. An undefined reference now fails the entry with
  a per-entry `C210` (file byte-identical on disk, other files in the batch
  still apply) that names the offending reference, states what the pattern
  defines, and teaches the corrective rewrites: escape literal `$` as `$$`
  (`$${name}` outputs a literal `${name}`) or add the capture group. The
  validator tokenizes exactly like `Captures::expand` (`$$` escape, longest
  `[0-9A-Za-z_]` run for unbraced refs — `$1a` names group "1a", not group 1
  — braced `${ref}`, literal `$` fallbacks), pinned by a validator-vs-expand
  parity test. Valid rewrites (`$0`, `$1`, `$name`, `${1}a`) work unchanged.

### Changed — multi-line replace echoes show the region's tail

- **Replace-site echoes now show the FIRST and LAST line of the post-replace
  region** (previously the first matched line only), with `elided` set to the
  inner line count when the region spans more than 2 lines. Single-line
  replacements are unchanged; sites stay capped at 5 and the ~4 KiB echo
  budget still applies. In the production session the corrupted line sat in
  the tail of a multi-line replacement and stayed invisible until a full
  read — it is now visible directly in the mutation response.

### Docs — request envelopes + replacement teaching

- `coder::update-file`, `coder::create-file`, `coder::delete-file`, and
  `coder::move` descriptions now open with their top-level request shape
  (e.g. `{"files": [{"path": "...", "ops": [...]}]}`) — the live agent
  guessed `{path, edits[]}` because no surface showed the envelope.
- The `replacement` schema field documents capture-reference expansion, the
  `$$` escape, the JS/TS template-literal collision, and the `$1a`
  longest-run gotcha. SKILL.md and README updated to match.

## 0.4.0

### BREAKING

- **Slim tree/list-folder wire shape**: per-node `path` is gone from
  `coder::tree` nodes and `coder::list-folder` entries — nodes carry only
  `name`. `coder::tree` now carries a top-level `path` (canonical absolute path
  of the snapshotted folder; `coder::list-folder` already had one). Migration:
  derive child paths by joining —
  child path = parent path + `"/"` + `name`; the root node's path IS the
  response's top-level `path` (do not join the root's own `name` onto it).
  Derived paths re-validate through the jail on use.

### Changed Defaults

- **Noise excludes are ON by default** (`default_exclude_globs`, new config
  key): `coder::tree` and `coder::search` now skip `**/.git/**`,
  `**/node_modules/**`, `**/target/**`, `**/dist/**`, `**/.venv/**`, and
  `**/__pycache__/**`. Excluded directories still appear in `coder::tree` as
  childless stubs flagged `truncated` (reason `"default_exclude"`); excluded
  files are omitted from `coder::search`. Hide-only — this grants no access
  protection (that remains `non_accessible_globs`). Opt out per call with
  `use_default_excludes: false`, or change the list via the
  `default_exclude_globs` config key. `coder::info` reports the active globs.
- **Single-path full reads are budgeted** (`max_output_bytes`, new config key,
  default 128 KiB): a `coder::read-file` full read whose converted content
  would exceed the budget now fails with a `C213` that carries the file's size
  and line count plus the recovery calls (window with `line_from`/`line_to`,
  probe with `stat: true`, or raise via the per-call `max_output_bytes`
  override, clamped to `max_read_bytes`). Windowed reads and batch mode are
  not governed by this key.

### New Features

- **`coder::search` context lines** (`context_lines_before` /
  `context_lines_after`, ≤10 each): content matches carry `before[]`/`after[]`
  arrays of surrounding lines, enabling the 2-call edit workflow (search with
  context → edit) with no file read in between.
- **`coder::search` response budget** (`search_response_budget_bytes`, default
  256 KiB): when the next match would exceed the budget the search stops
  accumulating and sets `truncated: true` — it degrades, it never errors.
- **`coder::read-file` `stat: true` probe**: size/mode/mtime plus
  `total_lines`/`is_utf8` without reading content. Budget-free in batch mode —
  probe many files, then window only what you need.
- **`coder::read-file` `numbered: true`**: prefixes each content line with its
  absolute 1-based file line number — the exact coordinates
  `coder::update-file` line ops take. Prefix bytes count against byte budgets.
- **Replace op `dot_matches_newline`**: lets `.` span newlines so two short
  anchors joined by `.*?` can replace a multi-line region without quoting it.
- **Replace op `expect_matches`**: pre-write guard — the op fails with a
  per-entry `C210` (before anything is written) when the actual match count
  differs. `expect_matches: 1` turns a silent multi-site clobber into a safe
  error; `expect_matches: 0` asserts absence.
- **`coder::info`** now reports `default_exclude_globs`, `max_output_bytes`,
  and `search_response_budget_bytes` alongside the existing caps.

### Fixes

- **`clip_line` UTF-8 boundary panic**: clipping a search result line to
  `max_line_bytes` now floors the clip point to the nearest UTF-8 character
  boundary instead of panicking when the cap fell inside a multi-byte
  character.

## 0.3.0

### BREAKING

- **Multi-root jail** (`base_paths[]`): replaces the single `base_path` scalar.
  Legacy `base_path` is still honored as a one-entry list; setting both keys is
  a startup error (`C210`). Default roots: `["./", "/tmp"]`.
- **Absolute paths inside any root are now accepted** (previously rejected as
  `C210`). Paths outside every allowed root still return `C215`.
- **All responses carry canonical absolute paths** — callers that compared
  relative echo values must update.
- **Structured per-entry errors**: `error` fields are now JSON objects
  `{"code":"C2xx","message":"..."}` instead of JSON strings containing escaped
  JSON. LLM agents can branch on `error.code` directly without string parsing.
- **`coder::update-file` response shape**: the old `before`/`after` full-body
  fields are gone. Each applied op now returns a bounded `OpEcho`
  (`op_index`, `from_line`, `lines`, `elided`, `total_replacements`) with ±2
  context lines for line ops and up to 5 per-match-site echoes for regex
  replace. An `echoes_truncated` flag is set when the budget runs out.
- **`coder::read-file` schema**: `path` and `paths[]` are now XOR fields
  (`C210` when both or neither is set). Single-path response fields are
  nullable; batch responses carry a top-level `results[]` array.

### New Features

- **`coder::info`** — pure discovery call: returns canonical allowed roots,
  per-file byte caps, listing/search limits, and non-accessible glob patterns.
  Agents that hit a path error should call this first to understand the jail
  contract.
- **`coder::move`** — batched move/rename with per-entry `overwrite` and
  `parents` flags. Same-root moves use `rename` (per-file atomic); cross-root
  file moves use copy+delete with rollback; cross-root directory moves are
  rejected with a clear `C210`.
- **`coder::read-file` line windows** (`line_from`/`line_to`): stream any slice
  of a large file without reading past the byte cap. `more_lines` flags
  remaining content; `total_lines` is set when the file was fully traversed.
- **`coder::read-file` batch mode** (`paths[]`): read multiple files or windows
  in one call against a shared `batch_read_budget_bytes` cap (default 1 MiB).
  Per-entry `C211`/`C213` errors leave other entries unaffected.
- **Prescriptive error messages** (C210/C211/C213/C215): every error names the
  relevant config key or root list and includes a corrective-action sentence so
  an LLM agent can repair its call without a human in the loop. The champion
  case: agents repeatedly abandoned coder after a single opaque `C210` with no
  hint about which config key was wrong or what values were legal.
- **Canonical request examples** in every function schema — wire-contract,
  golden-tested.
