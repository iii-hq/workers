# Changelog

## Unreleased

### Added

- **The explorer is shaped like VS Code.** An activity bar on the sidebar's
  outer edge switches between Explorer, Search, Source control and Timeline;
  one tab strip holds files and diffs alike (a diff tab names what it
  compares: Staged, Changes, a turn, a revision), and what a click opens is
  decided by the view it comes from — Explorer opens the file, Source
  control opens its staged or unstaged diff (both can be open at once),
  Timeline opens the diff of one turn; editor tabs carry file-type icons,
  Git colours, the preview italics and a dirty dot in place of the close
  button; breadcrumbs sit above the editor and the diff;
  files, folders, the empty space and the tabs have right-click menus (new
  file/folder, rename with F2, delete with confirmation, duplicate, copy
  path, open in terminal, find in folder, compare, discard changes, close
  others/right/saved/all). Back and forward across opened files
  (`Shift+Alt+←/→`), go to line (`L`), reveal the active file (`R`).
- **Source control view** — Staged changes and Changes with the letter
  VS Code shows per status, stage / unstage / discard per file or per
  section (discard asks first), and a commit box. Rows open the file's diff
  for their side of the index.
- **Timeline view** — every Harness turn of the chat, newest first, as a
  folder-like group named after the message that started it (the harness
  `turn-started` event now carries `message_preview` and `depth`), holding
  the files the turn changed. Work done by sub-agents the turn spawned is
  recorded under that turn by the worker (from the parent link the harness
  stamps into the child's hooks and events) and tagged with the agent's
  name. Opening a file shows that turn's exact patch: its pre-image against
  the body it left behind (`shell::turns::get` now returns `after`, the
  next turn's pre-image of the same path, and `agent`). A turn or a single
  file rolls back through the new `shell::turns::revert`, which restores the
  pre-image bodies the change history kept (created files are removed, moved
  files go back, bodies that were never stored are reported, not guessed).
- **Compare with…** — the working copy of a file against any branch, tag,
  recent commit or typed revision, from the editor header, the tabs or the
  explorer menu.
- **The chat composer's folder picker, in the header and the empty pane.**
  The console now shares `DirectoryPicker` (remembered projects first, a
  level-by-level browse to add one, every pick validated by this worker),
  so the explorer's root picker is the same control the chat uses instead of
  a native select over the base paths.
- **Tabs per folder, and a folder that sticks.** Switching the pane to
  another folder keeps what was open in the one being left and brings back
  what was open in the one being entered (up to sixteen folders, persisted).
  A folder picked in the pane is restored on reload even beside a chat on
  another folder; the chat's next folder change still re-roots the pane.
- **One folder for the pane and its chat, both ways.** The pane already
  followed the chat's folder; now a folder picked in the pane (header or
  empty pane) is handed to the chat beside it too, which records the change
  in the conversation as it does for its own picker. The "Browsing X. Chat
  still works in Y" banner and its "use for chat" button are gone with the
  state they described.
- **Tabs that outlive their files.** A file deleted or moved outside the
  editor — while the console was away, or live — no longer surfaces as a
  wire error in the editor: the tab is struck through in the strip, the
  editor shows a "no longer here" state with the path and a way out (try
  again, show the folder, close the tab), a buffer that was already loaded
  stays editable with a "gone from disk — save to put it back" note, and a
  file that comes back (created, or an atomic replace) reloads on its own.
  The tabs restored with a folder are probed once, so a file gone since the
  last visit reads as gone before it is opened; a persisted folder that no
  longer exists is reported in a notice once the pane has settled on
  another. A diff that cannot load gets the same shape of state.
- **Several Shell panes in one workspace tab.** The console passes a
  `paneId` to every page; the explorer keys its persisted state, its
  terminal leases and the engine functions its live triggers call on it, so
  two panes can browse two folders with separate terminal sessions and
  neither hears the other's file changes.
- **A new empty pane.** The wordmark, the folder sentence with the picker,
  one card per surface (open a file, search, browse, source control,
  timeline, terminal) with its key and what is behind it right now (how
  many changes and how many are staged, how many turns and the last one's
  name), and the files opened last.
- **`shell::workspace::read-bytes`** — one bounded byte range of a file per
  call, so the explorer streams a large image into a Blob chunk by chunk
  instead of asking for a single base64 frame the size of the picture.
- **`coder::search`** takes `respect_gitignore` (walk with `.gitignore`
  rules) and `fuzzy_paths` (quick-open ranking of path matches, best
  first); the command palette's `#` file search uses both.

### Changed

- **The review scope picker is gone.** Uncommitted / unstaged / staged /
  commit / branch / turn / session scopes, the review toolbar, the inline
  diff editing with its save barrier and the browser-side pre-turn snapshot
  all went with it; the views above cover the same ground with one model,
  and a turn's patch now comes from the worker's change history instead of
  a snapshot the page took at turn start.
- **Large workspaces and large files stay fast.** The explorer lists the
  root three levels deep and fetches a folder when it is expanded; watcher
  bursts patch the tree in place instead of re-listing it. The editor asks
  for up to 8 MiB per file (the worker's 128 KiB agent budget was cutting
  ordinary source files short) and, through the console `CodeEditor`'s new
  `fill` mode, owns its viewport so only visible lines render; files past
  the budget open as a read-only window of their first 5,000 lines.
- **Search results read like an editor's.** Matches group by file with the
  hit highlighted inside a short window of its line, the list is
  virtualized and keyboard-walkable, the query runs as you type, and the
  `Aa` / whole-word / regex toggles live inside the box; include/exclude
  globs and a "skip files ignored by Git" switch sit behind the disclosure.
- **`coder::search` content scanning** runs one compiled regex over each
  file's bytes and fans files out across a small thread pool while results
  are consumed in walk order — the same input truncates at the same place
  every time — and the walk is name-sorted.

### Fixed

- **The explorer forgot everything on a worker restart.** The worker
  registered its `shell-ui` configuration entry with an `initial_value` of
  `{}` on every boot, and `configuration::register` replaces the stored
  value whenever a seed is present — so each restart of the worker (or of
  the engine, which restarts the worker) erased every pane's browsed folder,
  open tabs and expanded folders. The seed is now installed only when
  nothing is stored, the way the `shell` entry's own seed already was. On
  the page side, the read that seeds a pane is retried a few times when it
  fails for a reason other than "nothing there", so an engine still coming
  up cannot make the pane boot fresh and then save its defaults over the
  stored state; and a save that finds the entry not registered yet is
  retried on the next change instead of disabling persistence for the page.

## 0.12.0

### Added

- **A page can take back its own orphaned session** — `shell::pty::adopt`
  re-owns an UNATTACHED session without a reconnect token, for the browser
  that lost its storage while the agent kept working. It refuses a session
  someone is attached to, and refuses a console page that is not the
  session's own; credentials rotate, so the previous owner's are dead.
  `shell::pty::sessions` now reports each session's `ui` — which console page
  it belongs to, never which browser — so a page can recognise its orphan.

- **The terminal's type size is the reader's choice** — every pane carries a
  stepper (8–40 px, 14 by default; Ctrl or ⌘ + scroll does the same), and the
  size is stored per browser rather than in this worker's configuration: the
  same engine read from a laptop and from a wall display wants two different
  answers. Every terminal the console renders shares the one value, agent-CLI
  pages included, and a change refits the pane so the PTY hears the new
  geometry through the ordinary resize path.

- **A PTY session can run one named program** — `shell::pty::open` takes
  `program`, `args`, and `env`. Without them a session is still the user's
  login shell. A caller that can open a login shell can already run any
  program by typing it, so this is reach rather than privilege: it lets a
  session BE one program, with no shell around it, which is what an agent CLI
  in a console page needs. Per-session `env` follows the same deny-only rule
  as `shell::exec`'s per-call env — an exec-hijacking key (`PATH`, `LD_*`,
  `DYLD_*`, `BASH_ENV`, ...) refuses the whole call.
- **`shell::pty::sessions`** — live sessions with their program, cwd, status,
  last sequence number, replayable frames/bytes, and current output target.
  No credentials. It separates a terminal that shows nothing because nothing
  was produced from one whose frames the browser dropped.

### Fixed

- **A per-call env key now has to BE a key.** `shell::exec`, `shell::exec_bg`
  and `shell::pty::open` checked the deny-list against the string handed in,
  and an environment entry is one `key=value` string — so `PATH=/tmp/evil`
  read as an unknown (therefore allowed) name, passed the `PATH` rule, and
  still handed the child a `PATH`. A key must now be an environment variable
  name (`[A-Za-z_][A-Za-z0-9_]*`): an empty key, a key holding a NUL, and any
  key carrying `=` are refused (`S210` on the exec surface).

### Changed

- The output handler a session may deliver to is validated by SHAPE rather
  than by this worker's own page:
  `iii::<worker>-ui::pty-output::console-<browser-id>`. A worker that runs its
  own program in a session serves its own console page and therefore its own
  handler; a session still cannot be pointed at an arbitrary function.
- The `shell::pty::*` functions are tagged `trace_hidden`, so the console's
  traces page no longer opens with one span group per keystroke and redraw.
  They are one funnel click away (see `docs/sops/trace-hidden-functions.md`).

## 0.10.3

### Fixed

- **Fresh managed installs no longer crash-loop with zero functions
  registered (MOT-4252)** — managed workers run with cwd = the engine's
  project directory, so the default `--config ./config.yaml` resolved to the
  engine's own worker roster (`workers: [...]`). Serde tolerates unknown
  top-level keys, so the roster parsed as an all-defaults shell config,
  failed the fail-closed jail validation, left the stored value null, and
  the engine's read-time schema validation then turned every
  `configuration::get` into a fatal SCHEMA_INVALID — boot died before any
  function registered, forever. Boot now classifies a seed file with no
  shell keys (and no removed/renamed shell key) as *foreign* and treats it
  like a missing file, so the zero-config path seeds the bootable
  permissive default. Shell-shaped files that fail to parse still refuse to
  boot, and un-migrated configs still get the loud migration hint.
- The boot probe of the stored value uses `configuration::get { raw: true }`
  so a stored null (the state a broken install is stuck in) is readable
  instead of erroring, and the bootable default is re-seeded over it —
  installs already caught in the crash loop self-heal on upgrade.

### Changed

- A `--config` seed (and the built-in default) is registered as
  `initial_value` only when no value is stored yet. Previously every boot
  sent `initial_value`, and `configuration::register` replaces the stored
  value when it is present — so a reboot with a seed file silently clobbered
  runtime `configuration::set` changes. The stored value now takes
  precedence once it exists, matching the documented contract. If a
  deployment relied on file-wins-on-restart, apply file edits with
  `configuration::set` (or clear the stored value) instead.

## 0.10.0

### Breaking

- **`coder::*` error codes renumbered to align with the `S2xx` scheme** —
  equal digits now mean the same failure class on both surfaces:
  already-exists `C217` → `C213`; too-large `C213` → `C218`;
  outside-session `C218` → `C220`. The approval-gate's jail-scope
  allowlist (`C218` → `C220`) ships in the same release wave; a stale
  approval-gate will not prompt on coder session-escapes until upgraded.
- **`shell::fs::*` existence redaction unified with coder's** —
  permission-denied, protected-glob, and `fs.denylist_paths` rejections
  fold into `S211` with the single "not found or not accessible" wording
  (previously `S215`). `S215` is now exclusively a jail-confinement
  escape. This closes an existence-probing side channel.

### Changed

- **`coder::*` follows the deny-only policy when unjailed (MOT-4099)** —
  with `fs.allow_unjailed: true` and empty `fs.host_roots`, coder accepts
  absolute paths anywhere on the host; the cwd + `/tmp` fallback roots
  only anchor relative paths, and the harness-stamped working directory
  is trusted as the anchor under both filesystem boundaries (matching
  `shell::exec`'s cwd contract). `fs.denylist_paths` now applies to
  `coder::*` in every mode (redacted `C211`). Jailed deployments are
  unchanged; the `workspace` boundary keeps session scoping so the
  folder-approval flow still triggers.
- **Unjailed `shell::fs::*` now enforces `code.non_accessible_globs`** —
  previously the glob check silently skipped paths outside every
  configured root, leaving secrets unprotected in unjailed mode.
- `coder::info` reports the effective access `mode` (`jailed` |
  `unjailed`).

### Migration

- Update any consumer branching on `C213`/`C217`/`C218` to the new
  numbers (`C218`/`C213`/`C220` respectively).
- Update any consumer that distinguished `S215` permission/denylist
  rejections from `S211` not-found — both are now `S211` by design.

## 0.8.0

Deny-only, permissive-first policy across the board: the shell no longer
carries a command allowlist, the fs jail is opt-in rather than defaulted on,
and the per-call `env` override on `shell::exec`/`exec_bg` is gated only by
the hardcoded dangerous-key denylist. Allow/ask policy (which commands need
a human) lives in the approval-gate; the sandbox backend is the real
security boundary for untrusted exec.

### Breaking
- **`allowlist` is removed.** Any config still carrying the key — including
  the inert `allowlist: []` written by older seeds — is rejected at parse
  with a migration hint. Rewrite the stored value via `configuration::set`
  (id: `shell`) without the key. There is no replacement: to gate commands,
  add approval-gate rules.
- **The planted-binary guard is removed with it.** Command paths (including
  files inside the writable fs jail, e.g. your own build output) now
  execute. The guard existed solely to prevent allowlist bypass; with no
  allowlist, running a planted file grants nothing `sh -c` doesn't already
  grant.
- **The shipped default is unjailed.** `config.yaml`/`seed_default()` now set
  `fs.allow_unjailed: true` with empty `fs.host_roots`: `shell::fs::*` and
  `shell::exec`'s per-call `cwd` operate against the real filesystem,
  confined only by `fs.denylist_paths` — matching `shell::exec` itself,
  which has never been confinement-based. `coder::*` is unaffected: it falls
  back to its own default roots (engine workspace cwd + `/tmp`) whenever
  `fs.host_roots` is empty, regardless of `fs.allow_unjailed`. This only
  affects a fresh, zero-config install or a stored config that gets nulled —
  an existing deployment with an explicit `fs.host_roots` keeps it; the
  stored value always wins over the seed.
- **The per-call `env` override on `shell::exec`/`exec_bg` is deny-only.**
  `env.allow` no longer gates which keys a caller may set per call — a key
  is now permitted UNLESS it's an exec-hijacking key (`PATH`, `IFS`, `HOME`,
  `LD_*`/`DYLD_*`, interpreter startup keys, ...), rejected unconditionally
  regardless of any config. `env.allow` keeps its other job unchanged:
  which vars get forwarded from the worker's own environment when
  `env.inherit` is false. This is a pure widening (nothing that worked
  before now fails) — no stored config needs a rewrite. There is no
  replacement for restricting per-call env keys further; use approval-gate
  rules if you need that.

### Migration
```yaml
# Rewrite the stored `shell` config to drop `allowlist` (any value,
# including `[]`) — there is no replacement key.
```
- A stored configuration value (id `shell`) still carrying `allowlist` makes
  the worker fail closed at boot with the hint above. Rewrite it via
  `configuration::set` without the key (see
  [README's Upgrading to 0.8.0](README.md#upgrading-to-080) for a runnable
  example).
- If you want to keep the pre-0.8.0 jailed-to-`/tmp` behavior instead of the
  new unjailed default, set `fs.host_roots: [/tmp]` explicitly — a fresh
  install with no config file at all now boots unjailed.
- **Order matters** for the `allowlist` removal, same as 0.7.0: deploy the
  0.8.0 binary FIRST, then rewrite the stored value. Writing the new shape
  while an older worker is still running makes it hot-reload a config it
  doesn't understand.

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
- **`fs.host_root` (the 0.6.x single-root alias) is removed** — use
  `fs.host_roots` (one-entry list). Like the env keys it is **rejected at
  parse** with a migration hint (`fs.host_root` -> `fs.host_roots`); serde
  would otherwise ignore the stale key and the worker would see no jail
  configured at all.
- **`code.base_path` and `code.base_paths` are removed from the schema and
  REJECTED at parse.** They were inert even before removal — the code
  resolver has always taken its roots from `fs.host_roots` (one jail
  config) — but this is a hard migration: "never had an effect" is not an
  exception. A stored value still carrying either fails closed with a hint
  naming both keys, same as every other removed key.
- **The one-shot coder→shell config migration is removed**
  (`migrate_legacy_coder`), and the hidden `migrated_from_coder` marker
  field is REJECTED at parse, not silently tolerated. 0.7.0 no longer folds
  a legacy standalone-`coder` configuration entry into the `shell` value at
  boot, and boot no longer probes `configuration::get` for a `coder` entry
  — which also removes the boot-time "configuration 'coder' not found" WARN
  retries. Two distinct upgrade scenarios:
  - An install that ALREADY has a `shell` entry (it went through the fold
    under 0.6.x, so that entry carries `migrated_from_coder: true`) now
    fails closed at 0.7.0 boot with a migration hint, instead of silently
    parsing past the marker.
  - An install with ONLY a standalone `coder` entry and NO `shell` entry at
    all has nothing to reject — `register_config` still seeds the generic
    permissive `/tmp` dev default for `shell`, silently, because there is no
    stored `shell` value to fail closed on. Boot 0.6.x once first (it
    performs the fold and writes the `shell` entry) before upgrading to
    0.7.0 to avoid this.

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

### Changed (shipped seed / defaults review)
- **Command-shaped denylist patterns are anchored to argv[0]**
  (`^(\S*/)?mkfs|dd|shutdown|reboot`): they fire when the tool IS the command,
  not when the word appears in an argument — `grep -rn shutdown src/` and
  `rg "dd if=" docs/` are no longer rejected. Argument-shaped patterns
  (`rm -rf /`, the fork bomb, `/etc/shadow`) stay unanchored. Stacks that
  rewrite their stored value by hand should adopt the anchored forms too.
- The denylist rejection message now says it is an advisory tripwire and to
  rephrase the command, so agents stop retrying verbatim.
- The seed uses the multi-root jail form (`fs.host_roots: [/tmp]`) — the
  singular `fs.host_root` was then removed outright (see Breaking).
- Seed `default_timeout_ms` raised 10s → 30s: the seed raises
  `max_timeout_ms` to 120s so real builds survive; callers omitting
  `timeout_ms` shouldn't be reaped at 10s on the same workload. The CODE
  default is unchanged (10s).
- `fs.max_read_bytes`/`fs.max_write_bytes` descriptions now explain why the
  code default is unlimited (reads/writes stream in chunks; the cap bounds
  caller cost, not worker memory), and the seed comments say
  `fs.denylist_paths` is defense in depth (unreachable anyway while jailed).
- Every `code.*` (CoderConfig) budget field now carries a schema description;
  the schema test covers all nested definitions, not just the top level.

### Fixed (pre-landing review)
- **Seed-file parse failures now fail closed.** A `--config` file that EXISTS
  but fails to parse (e.g. still carries the removed 0.6.x keys) aborts boot
  instead of warning and silently seeding the permissive built-in default in
  its place — that fallback would have handed a fresh registration an open
  allowlist and full env forwarding instead of the operator's intended
  policy. A genuinely MISSING file still falls through gracefully.
- **Hot-reload no longer retry-storms on an unparseable stored value.**
  Previously, a stored config carrying removed keys failed inside the fetch
  step and was misclassified as a *transient* error (dispatcher retries
  forever against bytes that can never parse). It is now classified as
  `Rejected` — the same treatment as an unbuildable-but-parseable config:
  keep last-good, ack (no storm), and record the rejection for
  `shell::config-status`.
- **`env.allow` half-migration is rejected.** `EnvConfig` now denies unknown
  fields, so nesting the OLD key names under the new block (e.g.
  `env: { inherit_env: true }`) fails closed instead of silently falling back
  to the wider default `allow` list.
- **Command-shaped denylist patterns tolerate a wrapper prefix**
  (`sudo`, `doas`, `nohup`, `env`, `timeout [duration]`, optionally
  path-qualified): `sudo shutdown -h now` trips the tripwire again — the
  argv[0]-anchoring in the first pass of this release had dropped that case,
  arguably the most likely accidental invocation for commands that normally
  require root.
- **The removed-key check found and fixed its own bug during consolidation**:
  merging the top-level and nested-`fs` checks into one function surfaced
  that the original used `.any()`, which short-circuits — a config carrying
  BOTH `inherit_env` and `allowed_env` only ever named the first in its error.
  Every removed key present is now named in one pass.
- **The boot-time reachability probe runs detached** so its DNS resolution
  (unbounded — system resolver) and 2s-per-address TCP connect attempts can
  never delay startup, and it logs host:port rather than the raw URL (a
  `wss://user:pass@host` URL could otherwise leak credentials to the log).
- **A half-migrated `env` block now gets the same migration hint as every
  other removed key.** Nesting the OLD field names under the NEW `env:`
  block (e.g. `env: { inherit_env: true }`) previously hit `EnvConfig`'s
  generic `deny_unknown_fields` serde error with no guidance; it now names
  the key and points at `env.inherit`/`env.allow` like the other rejections.
- **The anchored denylist wrapper tolerance is now case-insensitive and
  handles `env`'s idiomatic `KEY=VALUE...` form.** `SUDO shutdown -h now`
  and `env FOO=bar shutdown -h now` (env's actual common usage — bare
  `env cmd` was covered, `env KEY=VAL cmd` was not) now trip the tripwire;
  previously both silently bypassed it, undermining the wrapper-tolerance
  feature's own stated purpose for its most-used wrapper.
- **Fixed a flaky test**: two tests that mutate process-wide environment
  state (`std::env::set_var`) could race on separate `cargo test` threads
  within the same binary, producing an intermittent, environment-dependent
  failure. Both now serialize on a shared test-only mutex.

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
- **Multi-root jail:** `fs.host_roots` (a list). Relative paths anchor at the
  primary (first) root; absolute paths are accepted inside any root.
- **Unified protected paths:** `code.non_accessible_globs` is honored by BOTH
  surfaces — the code functions show-but-lock (`C211`), `shell::fs::*`
  hard-rejects (`S215`) — so secrets are declared once. `fs.denylist_paths`
  (absolute-prefix) remains as a separate hard layer.
- The exec allowlist guard now rejects a command path inside ANY writable root.

### Changed
- The unbounded code read/scan handlers (`tree`, `search`, `list-folder`,
  `read-file`) run off the executor via `spawn_blocking`.
- Agent prompts route code-file work through `coder::*` **on the shell worker**
  (no separate registry install).

### Migration
- `iii worker add shell` now brings the whole surface; the standalone `coder`
  worker is retired. Configure the folded code surface directly under `shell`.
- After deploying, run `iii worker restart shell` (the source watcher does not
  always restart the VM process).
