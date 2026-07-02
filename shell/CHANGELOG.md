# Changelog

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
