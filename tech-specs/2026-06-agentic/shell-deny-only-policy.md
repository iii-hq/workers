# Shell deny-only command policy

**Ticket:** MOT-3872
**Status:** Approved design, pending implementation
**Scope:** shell worker only — no approval-gate changes

## Problem

The shell worker and the approval-gate both carry command policy, and they
overlap in a confusing way:

- The shell worker ships an `allowlist` field (argv[0] basename). Empty means
  open; non-empty flips exec to deny-by-default. The shipped seed is empty, so
  in practice the field is dead weight that still costs code: a
  planted-binary guard, error paths, docs, and tests all exist to protect a
  list nobody populates.
- The approval-gate already holds every unmatched call for human approval.
  Allow/ask policy — "which commands may run without a human" — belongs there,
  where trust accrues per-session (always-allow grants) and per-deployment
  (gate rules).

Two places to express "allow" is one too many. The first attempt at this
ticket (gate-side ask rules with a synthesized command line, commit
`21dc1064`) was rejected; this spec replaces it.

## Decision

One policy per layer:

| Layer | Owns | Mechanism |
|---|---|---|
| shell worker | **deny** — refuse catastrophic commands (advisory tripwire) | `denylist_patterns`, catastrophic-only seed |
| approval-gate | **ask/allow** — what needs a human | hold-by-default + gate rules + always-allow grants |

The shell worker loses the concept of an allowlist entirely. Its only
command policy is the denylist: an advisory tripwire for catastrophic
mistakes (`rm -rf /`, fork bomb, `mkfs`, `dd if=`, `shutdown`, `reboot`,
`/etc/shadow`), wrapper-tolerant and case-insensitive, exactly as seeded
today. The fs jail, the per-call env floor (`DANGEROUS_ENV_KEYS`), and the
sandbox backend remain the actual security boundaries — the denylist's
advisory framing does not change.

The gate is untouched: it already asks about everything by default (no-match
= hold), and its rules are already the layer where allows accumulate. The
denylist is what makes a coarse function-level "always allow `shell::exec`"
grant acceptable: even a fully trusted shell still refuses the catastrophic
patterns.

## Shell changes

### Config schema

- Delete `ShellConfig.allowlist`.
- Add `deleted("allowlist")` to `REMOVED_TOP_LEVEL_KEYS` (`src/config.rs`),
  following the 0.7.0 hard-migration convention: any stored value or YAML
  seed still carrying the key — including the inert `allowlist: []` the old
  seed wrote — is rejected at parse with the
  `configuration::set (id: shell)` hint. No silent tolerance.
- Drop `allowlist: []` from `config.yaml`, `config.collect.yaml`, and
  `ShellConfig::seed_default()`. The seed-sync unit test keeps them aligned.

### `is_command_allowed`

Shrinks to two checks: non-empty argv, then the compiled denylist regexes
over `argv.join(" ")`. Removed along with the allowlist branch:

- **The planted-binary guard** (reject a `command` path that canonicalizes
  inside the writable fs jail; reject all command paths when unjailed). Its
  entire threat model was allowlist bypass — "`shell::fs::write` can plant an
  executable whose basename passes the allowlist." With no allowlist, running
  a planted file grants nothing that `sh -c` doesn't already grant, while the
  guard actively blocks a legitimate coding-agent case: executing your own
  build output (`./target/debug/foo`) inside the jail. The guard is removed,
  not weakened — this is a deliberate, reviewed deletion, not an oversight.

### Docs

- `README.md` / `ARCHITECTURE.md`: drop "allowlisted command" phrasing and
  the allowlist config/error-table rows; add a short "policy layering"
  paragraph stating deny-only shell + gate-as-allow-layer, and that
  per-command allow policy belongs in approval-gate rules.
- `CHANGELOG.md`: breaking-change entry with the migration note below.

### Tests

- Unit: delete allowlist accept/reject/lists-allowed tests and the
  planted-binary guard tests; keep and extend denylist tests (wrapper
  tolerance, case-insensitivity) and the seed-sync test.
- E2E: delete allowlist-rejection cases; flip the planted-binary case from
  expect-reject to expect-run; denylist cases unchanged. Purge `allowlist`
  from `tests/e2e/config*.yaml` (after removal the key would hard-fail
  boot).

## Migration

Breaking for every existing stack: stored shell configurations were seeded
with `allowlist: []`, so on upgrade the worker fails closed at config parse
until the operator rewrites the stored value via `configuration::set`
(id: `shell`) without the key. The parse error names the key and the remedy.
This matches the 0.7.0 precedent (`fs.host_root`, `migrated_from_coder`):
removed keys hard-reject even when inert.

Operational note: the live `iii-running` stack needs this one-time rewrite
when the new shell build is deployed.

## Out of scope

- `env.allow` stays. It is env-key forwarding/settability, not command
  policy, and `DANGEROUS_ENV_KEYS` remains its non-overridable deny floor.
- Gate default-rule changes, `shell::classify_argv`, and gate-side
  command-content matching (a synthesized command line for rules). Nothing
  here precludes them; the classify-argv spec remains the compatible future
  direction if per-command gate policy is ever wanted.

## Housekeeping

- Implementation branch: `feat/shell-deny-only`; this spec commits there.
- Delete the orphaned `feat/approval-permissive-shell-defaults` branch
  (commit `21dc1064`, the rejected gate-side direction).
- MOT-3872 stays the ticket; PR title carries the `(MOT-3872)` prefix.
