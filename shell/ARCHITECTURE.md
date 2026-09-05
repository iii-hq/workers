# shell — architecture and operator notes

The published `README.md` and `skills/SKILL.md` for this worker are hand-maintained. This file holds the operator/contributor material that does not belong in the published surfaces: full configuration table, threat model, wire shapes for the streaming functions, troubleshooting, tests, and deferred work.

## Build and wire-up

```bash
# 1. Install the iii engine (drops the binary at $HOME/.local/bin/iii by default).
curl -fsSL https://install.iii.dev/iii/main/install.sh | sh

# 2. Build this worker.
cargo build --release --bin iii-shell

# 3. Wire the binary where the engine looks (registered worker name = `shell`,
#    per worker-compose.yaml — fall back to ~/.iii/workers/iii-shell if your
#    engine resolves by binary name).
mkdir -p ~/.iii/workers
ln -sfn $(pwd)/target/release/iii-shell ~/.iii/workers/shell

# 4. Start the engine (it spawns the worker). Pin fs.host_roots or set
#    fs.allow_unjailed: true in config.yaml first — the worker refuses to
#    start unjailed by default.
iii -c ./config.yaml
```

`iii trigger compose::add worker=shell` does not currently pull `iii-sandbox` along — run `iii trigger compose::add worker=iii-sandbox` separately before using `shell::exec { target: sandbox }` or any `shell::fs::*` sandbox-target path. Plain host-targeted `shell::exec` works without it.

## Injected console UI (`ui/`, `src/ui.rs`)

The worker ships UI into any running console (SOP:
`workers/docs/sops/injectable-console-ui.md`): the shell explorer page
(`#/ext/shell` — files/git/search sidebar beside the console's shared Monaco
editor and `FileDiff`) and the `shell::*` function-trigger renderers (moved
out of the console SPA; the console's `first-party/shell` family is gone).
The explorer acts through the worker's own functions: `coder::tree/read-file/
create-file/search` and `shell::exec` (git, argv form, `cwd`-scoped).

The explorer is shaped like VS Code so it needs no learning: an activity
bar (Explorer, Search, Source control, Timeline) on the sidebar's outer
edge, one tab strip for everything the main pane shows, breadcrumbs,
right-click menus on files, folders and tabs, and inline create/rename
rows. A tab is a file (real content, editable) or a diff of one file
against one source; what a click opens is decided by the view it comes
from: Explorer opens files, Source control opens index diffs (a file can
be open twice, staged and unstaged), Timeline opens the diff of one turn.
Its structure, one module per concern under `ui/src/page/`:

| Module | Owns |
|---|---|
| `tabs.ts` + `diff-source.ts` | Tab identity and semantics: `file:<path>` and `diff:<source>:<path>` ids, preview/pin rules, the five diff sources (staged, unstaged, turn, compare, change) and which of them persist. |
| `diff-load.ts` + `DiffTab.tsx` | The two sides of a diff tab, per source (`git show` for HEAD/index/revisions, `coder::read-file` for the working copy, `shell::turns::get` for a turn's pre-image and the body it left behind, `coder::change-diff` for a recorded change), and the read-only diff pane with the verbs that fit the source. Only the active diff loads; contents are cached per tab and re-read when the disk moved. |
| `use-workspace-tree.ts` + `tree-model.ts` | The file tree as a value: a shallow first listing (`coder::tree`, depth 3), a per-folder fetch when a folder is expanded, and watcher bursts (`shell::changed`) patched in place. |
| `FilesTab.tsx` | The Explorer view over `@pierre/trees` (virtualized rows): context menus, F2 rename, inline new file/folder, delete with confirmation, Git letters per row. |
| `SearchTab.tsx` + `search-model.ts` | Search as you type over `coder::search` (`respect_gitignore`), results grouped by file with the hit highlighted inside a short window of its line, keyboard-walkable, virtualized (`VirtualList.tsx`). |
| `SourceControlTab.tsx` + `use-source-control.ts` + `git-actions.ts` | Staged / Changes sections with stage, unstage, discard (confirmed) and commit, over `shell::exec` git in argv form. |
| `TimelineTab.tsx` + `turn-revert.ts` + `use-turn-summary.ts` | Every Harness turn of the chat, newest first, as a folder-like group named after the message that started it (the harness `turn-started` event's `message_preview`), holding the files it changed — sub-agent work included, tagged with the agent. A turn or a file rolls back through `shell::turns::revert`; the newest turn feeds the chat footer pill. |
| `EditorPane.tsx` + `large-file.ts` + `file-bytes.ts` | The shared Monaco `CodeEditor` in `fill` mode, an 8 MiB read budget with a read-only line window past it, raster images streamed in 1 MiB chunks over `shell::workspace::read-bytes`. A read that finds the file gone becomes the "no longer here" state; a loaded buffer whose file goes away stays editable so a save puts it back, and reloads by itself when the file returns. |
| `EditorTabs.tsx`, `Breadcrumbs.tsx`, `ContextMenu.tsx`, `ActivityBar.tsx`, `ViewHeader.tsx` | Chrome. |
| `nav-history.ts` | Back/forward across opened tabs (`Shift+Alt+←/→`); the recently opened list the empty pane offers. |
| `pane-scope.ts` + `persist.ts` + `root-memory.ts` | One page instance per pane: its state key (`paneId`, `tabId` on older consoles), the state it persists (folder, pinned or not, view, options, terminal layout) through `shell::ui-state::get`/`set` — one JSON file per pane under the worker's data directory (`src/ui_state.rs`, below) — and what was open per folder so switching folders and back finds the tabs again. The load that seeds a pane retries on any failure (a pane that boots believing nothing was stored would save its defaults over the stored state); only a clean "nothing stored" is final. The terminal leases and the pane's live trigger functions (`shell::changed`, the harness turn events) are keyed the same way, so two panes of one tab never share a terminal or hear each other's folder. |
| `missing-files.ts` + `PaneNotice.tsx` + `load-error.ts` | Tabs that outlive their files: the set of open file paths the page knows are gone (fed by a stat probe after a restore, the editor's own read, and the live feed; cleared when the file comes back or the tab closes), the one shape every "cannot show content" state takes (icon, title, path, one line, the verbs out), and the worker's `C211` read failure told apart from the rest. |
| `ShellLauncher.tsx` | The empty pane: the wordmark, the folder sentence with the console's `DirectoryPicker` (the chat composer's, also the header's picker), one card per surface with its key and what is behind it (change and turn counts, the last turn's name), the files opened last. |

Building the worker therefore needs Node + pnpm on PATH: `build.rs` runs
`pnpm install && pnpm build` in `ui/` when `ui/dist/` is missing or stale and
`include_str!`s the outputs (`SKIP_UI_BUILD=1` uses existing `ui/dist/`
as-is). Dev loop: `cd ui && pnpm watch` plus `III_SHELL_UI_WATCH=1` on the
worker — open console tabs hot-swap the assets. UI parser/format tests:
`cd ui && pnpm test`.

## CLI flags

| flag | default | purpose |
|------|---------|---------|
| `--config <path>` | `./config.yaml` | Optional seed config: the YAML is passed as `initial_value` when registering the schema with the `configuration` worker on first boot. It is **not** the live source of truth — the live value is fetched over RPC after registration. When the file is absent and nothing is stored yet, the worker seeds a built-in zero-config default (`ShellConfig::seed_default()`, unjailed by explicit opt-in) instead. |
| `--url <ws-url>` | `ws://127.0.0.1:49134` | iii engine WebSocket. Also read from the `III_URL` env var (the flag wins). A pre-connect probe logs one ERROR with a fix hint when the engine is unreachable; the SDK then retries forever with a 2s backoff. |
| `--version` | — | print the worker version |

Logging is controlled by the `RUST_LOG` env var (tracing `EnvFilter` syntax; default `info`).

## Configuration

The shell worker integrates with the central `configuration` worker rather than reading a static file at runtime:

1. On boot it registers a schema with id `shell`; the YAML at `--config <path>` (default `./config.yaml`) is sent as the `initial_value` (populates the first-boot default). If the file is missing or unreadable the worker warns and, when no value is stored yet, seeds the built-in zero-config default (`ShellConfig::seed_default()`) so it still boots.
2. It immediately fetches the live value over RPC and activates the security policy and fs backend from that response.
3. It then registers the `configuration:updated` trigger and runs a **fail-closed** boot reconcile before exposing any public function. The reconcile re-fetches the authoritative value (closing the race where an update lands between the initial fetch and trigger registration, leaving no listener). If that re-fetch fails the worker aborts startup — it exits rather than serve a possibly stale security policy, and no `shell::*` / `shell::fs::*` function is ever exposed.
4. It subscribes to `configuration:updated` events. When the config for schema id `shell` changes, the worker hot-reloads the security policy and fs backend atomically.
5. If the incoming config is invalid or unsafe (e.g. schema validation passes but the worker cannot build it — bad denylist regex, unreachable jail root), the worker keeps the last-good runtime and logs an error — it does **not** crash, and it does **not** retry (re-fetching returns the same bad value, so a retry would storm). The rejection is recorded and surfaced by `shell::config-status` (a `rejected` outcome with a non-zero `rejected_reloads` count) so the divergence between the central store and the enforced policy is detectable instead of silent.
6. A reload that widens the jail (clearing `host_roots`) succeeds, but is logged as a privilege change.

## Full YAML defaults

These are the CODE defaults (`ShellConfig::default()` — fail-closed: `env.inherit
false`, unjailed refused unless explicitly opted in). The shipped seed
`config.yaml` / `seed_default()` is deliberately more permissive for dev use:
`env.inherit true`, unjailed but with the opt-in given explicitly
(`fs.allow_unjailed: true`), `max_timeout_ms 120000`, catastrophic-only
denylist.

| key | default | enforced where |
|-----|---------|----------------|
| `max_timeout_ms` | `30000` | foreground `exec` hard cap; per-call `timeout_ms` clamped to this |
| `max_bg_timeout_ms` | `0` | host bg job hard cap in ms; `0` = unbounded (separate from `max_timeout_ms`, which bounds foreground exec) |
| `default_timeout_ms` | `10000` | applied when caller omits `timeout_ms` |
| `max_output_bytes` | `1048576` (1 MiB) | stdout/stderr truncated; `*_truncated` flagged |
| `working_dir` | `null` | pins cwd for spawned commands when set |
| `env.inherit` | `false` | forward the worker's FULL env to children; when `false`, only `env.allow` keys are forwarded |
| `env.allow` | `[PATH, HOME, LANG, LC_ALL, TERM]` | forwarding allowlist when `env.inherit` is false. No effect on the per-call `env` override, which is deny-only (gated solely by the hardcoded dangerous-key list) |
| `denylist_patterns` | `[]` | advisory regex tripwire on `argv.join(" ")` |
| `max_concurrent_jobs` | `16` | rejects new `exec_bg` past the cap |
| `job_retention_secs` | `3600` | finished jobs evicted by a background reaper (interval `min(30s, retention/2)`) — the primary prune path; prune-on-`shell::list` remains as a harmless secondary trigger |
| `fs.host_roots` | `[]` | jail roots; first = primary; required non-empty unless `fs.allow_unjailed: true` |
| `fs.allow_unjailed` | `false` | explicit opt-in to running with an empty `host_roots` |
| `fs.max_read_bytes` | `0` (unlimited) | pre-flight cap via `fs::metadata` (`S218`) |
| `fs.max_write_bytes` | `0` (unlimited) | mid-stream cap during write (`S218`) |
| `fs.denylist_paths` | `[]` | absolute-prefix denylist; rejected with `S215` |
| `fs.allow_special_bits` | `false` | setuid/setgid/sticky bits in `mkdir`/`chmod`/`write` modes are rejected with `S210` unless `true` |
| `sandbox.enabled` | `true` | `false` → every sandbox-target call returns `S210` |

## Threat model

The host backend's path-validation gate is check-then-use: there is a TOCTOU window between validation and the `std::fs::*` call. Validation walks to the longest existing ancestor, canonicalizes that (resolving symlinks in the existing portion), and lexically collapses the non-existent tail before the jail-root containment check — so a symlink whose target escapes the jail cannot slip through the lexical fallback. The worker is intended for trusted caller pipelines; for untrusted input, use the sandbox backend.

Host-targeted calls run with the shell worker's OS permissions. The denylist is regex over `argv.join(" ")` and only catches honest typos — a caller invoking a shell or interpreter (`sh`, `node`, `python`, …) can bypass it by construction. The actual security boundary is `target: { kind: "sandbox", sandbox_id }`.

Command policy is deny-only: the shell never decides which commands are *allowed* — that (ask/allow) layer is the approval-gate's; the shell only refuses catastrophic patterns.

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

## Console-only functions

Registered with `internal: true` (or documented as control plane) — the
explorer page calls them; agents should not need them.

| Function | Purpose |
|---|---|
| `shell::workspace::read-bytes` | One bounded byte range of a file, base64, at most 4 MiB raw per call: `{ path, offset?, length? }` → `{ path, size, offset, length, content, mtime, eof }`. The page streams a large image chunk by chunk instead of asking `coder::read-file` for one 14 MiB frame. Jailed through the `coder::*` resolver. |
| `shell::turns::get` | Now also carries, per file, `agent` (the sub-agent session and display name when a child made the change) and `after` (the body the turn left behind: the first later turn's pre-image of the same path, inflated from the blob store; absent when the working copy is the after side). Turns carry `title`, the `message_preview` from the harness `turn-started` event. |
| `shell::turns::revert` | Undo one turn's recorded changes from the pre-image blob store: `{ session_id, turn_id, paths? }` → per-file `{ path, kind, action, success, error? }` plus `reverted` / `failed` counts. Created files are removed, modified and deleted files get their stored body back, moved files return to their source path. Bodies the hooks never stored (over 64 KiB, binary, watcher-observed) are reported as `unavailable`, never guessed. Paths pass the operator denylist and non-accessible globs; containment is not re-checked because the paths are the worker's own records. |
| `shell::ui-state::get` / `shell::ui-state::set` | The explorer page's per-pane state (browsed folder, open tabs, expanded folders, view, options, terminal layout): `get { key, legacy_key? }` → `{ key, state \| null }`, `set { key, state }` → `{ key, bytes }`; `key` is the console pane id, `legacy_key` the workspace tab id older saves were keyed by. Both `trace_hidden`. |

**Where the pane state lives** (`src/ui_state.rs`): one JSON file per
pane key under `data/shell/ui-state/panes/` (resolved against
`III_COMPOSE_DIR` like `turns.data_dir`; the key is percent-encoded into
the file name). It is developer-local runtime state, so it sits in the
gitignored `data/` tree — NOT in the engine's `configuration` store, which
persists into the project's committable `config/` folder. A `set` writes
only that pane's file (temp + rename, under one store-wide mutex): panes
never clobber each other, two writers of one pane serialize, and a reader
never sees a torn document. A missing or unparsable file reads as
"nothing stored". Until 0.12.x the page read-modify-wrote one `shell-ui`
configuration entry holding every pane; at boot, before these functions
are exposed, the worker imports whatever that entry still holds (files
already present win) and blanks the entry, so `config/shell-ui.yaml` can
be deleted. Files for panes that no longer exist are not pruned (the
entry never pruned them either).

**Sub-agent changes are recorded under the spawning turn.** The harness
stamps `parent_session_id` / `parent_turn_id` into a child session's turn
metadata, which reaches the shell's pre/post-trigger hooks as `metadata`,
and its `turn-started` event carries `parent`. `TurnLog` keeps a
child → parent map from both, resolves every hooked or watcher-observed
change to the top-level ancestor's turn (`resolve_root`), and records
the child as the file's `agent`. Child sessions keep no record of their
own; a child's `turn-completed` closes nothing.

`coder::search` grew two flags the page relies on: `respect_gitignore`
(walk with `.gitignore`/`.ignore` rules inside a repository) and
`fuzzy_paths` (quick-open ranking of path matches, best first). Content
scanning runs the regex over each file's bytes and fans files out across
a small thread pool while results are consumed in walk order, so caps and
budgets cut at the same place every time.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `fs.host_roots is empty and fs.allow_unjailed is false — refusing to start unjailed` | Default config no longer permits running unjailed. | Set `fs.host_roots` to at least one directory, OR set `fs.allow_unjailed: true`. |
| Worker never connects to engine | Engine isn't running or isn't bound on the URL the worker is configured for. | Start the engine first; check `--url` matches. The default WS port is 49134. |
| Engine started but doesn't see the worker | Binary isn't symlinked at `~/.iii/workers/shell`. | `ln -sfn $(pwd)/target/release/iii-shell ~/.iii/workers/shell` |
| `S215 path escapes the fs jail roots` on a path inside the jail | A symlink in the path resolves outside the jail. | Resolve the symlink yourself, or move the target inside a jail root. |

## Tests

- `tests/e2e/` — TypeScript harness. The default suite (`run-tests.sh`) covers happy paths, safety guardrails, jobs lifecycle, fs across host and sandbox targets, adversarial protocol-break suites for streaming/exec/jobs/encoding/concurrency, plus vulnerability-regression cases under `cases-vuln-repro.ts`. The jailed mode (`run-tests.sh --suite=jailed`, against `config-jailed.yaml`) covers the symlink-parent jail-escape regression. Case count drifts as cases are added/removed — treat `run-tests.sh`'s own summary line (or `reports/report.json`'s `total`) as ground truth rather than a number in this doc.
- `tests/*.rs` — Rust integration tests (`jobs_lifecycle`, `host_fs_branches`, `sandbox_dispatch`, `function_handlers`) covering the host backend branches, sandbox forwarder, and every typed-registration handler. Run with `cargo test`.
- Line coverage measured with `cargo tarpaulin` sits around 65%; `jobs.rs` is at 100% and the sandbox dispatch path is fully exercised.

## What this is NOT

- **Not a PTY.** Interactive shells, TUIs, password prompts all break.
- **Not an isolation boundary itself.** Host-targeted calls run with the shell worker's OS permissions. For process isolation, set `target: { kind: "sandbox", sandbox_id }` — that path forwards through `iii-sandbox`'s microVM. The denylist still applies on top of either backend.
- **Not a streaming surface for `exec`.** Foreground `shell::exec` returns once the process exits and stdout/stderr are captured whole. Live streaming is `shell::exec_stream` (deferred).
- **Not per-caller-isolated.** The JOBS registry is a process-wide singleton. `shell::list` redacts argv/stdout/stderr to limit blast radius; full records are cap-gated by `job_id`.

## Deferred

- `shell::exec_stream` — live stdout/stderr via iii Streams (for long-running commands). Next iteration.
- Per-caller JOBS scoping (would replace the redact-summary-only mitigation in `shell::list` with proper isolation).
