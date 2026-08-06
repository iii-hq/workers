# approval-gate

The policy and decision surface for human-held function calls
([spec](https://github.com/iii-hq/workers/blob/main/tech-specs/2026-06-agentic/approval-gate.md)). Four surfaces, one
worker:

1. **The gate** — `approval::gate`, a `pre_trigger` hook the worker binds
   itself at startup on the harness's `harness::hook::pre-trigger` trigger
   type. It evaluates per-session mode, allow-lists, and inline config
   `rules`, and answers `continue`, `deny`, or `hold`.
2. **The grant watch** — `approval::grant-watch`, a `post_trigger` hook bound
   on `shell::*` / `coder::*`. It watches for a jail-scope `grant_hint` on a
   dispatch failure and converts it into a `folder_access` hold instead of
   letting the raw rejection reach the model. See
   [Folder-access grant watch](#folder-access-grant-watch) below.
3. **The decision plane** — `approval::resolve` plus the per-session settings
   RPCs (`set_mode`, `add_always_allow`, `approve_always`, …). Human/console
   only.
4. **The pending inbox** — an **ephemeral** index of held calls
   (`approval::list-pending` / `approval::get-pending`) plus two trigger types
   (`approval::pending-created` / `approval::pending-resolved`) that
   notification workers and UIs bind to. `PendingApprovalRecord.kind`
   (`"function"` | `"folder_access"`) tells a console which prompt to render.

The worker keeps **no resolved-approval history**: a record exists only while
a call is held; every record has an explicit deletion path (resolve, turn
abort, session delete). The transcript's `function_result` and the
`pending_resolved` event are the audit trail. Holds do not expire — they wait
until a human resolves or the turn/session is purged.

## Standalone caveat

This worker codes against the greenfield harness contracts
(`harness::hook::pre-trigger`, `harness::hook::post-trigger`,
`harness::function::resolve`, `harness::turn-completed`,
`harness::workspace::grant`/`grants` — see harness.md § Hooks / § API
Reference), which are **not implemented by the current harness yet**. The
harness/session trigger bindings are best-effort: on an engine without those
trigger types the worker still boots, serves its RPCs, and logs
`trigger_type_not_found` for the absent bindings (restart it after the
sibling appears to re-bind). Lacking `harness::workspace::grant`/`grants`
specifically degrades `approval::grant-watch` alone (see
[Folder-access grant watch](#folder-access-grant-watch)) — it never holds,
the rest of the worker is unaffected. The `configuration` worker is the one
hard dependency — it is enabled by default in the engine, and a failed
register/fetch aborts boot. The integration suite exercises the harness
surface against in-process fakes until harness 1.0 lands.

## Install

```bash
iii worker add approval-gate
```

## Quickstart

```bash
cargo build
./target/debug/approval-gate --url ws://127.0.0.1:49134
```

The authoritative config lives in the engine's `configuration` worker (no
committed `config.yaml`); pass `--config <file>.yaml` only to seed the entry on
its very first registration.

Hold → decide → release, from any client:

```bash
# A held call shows up in the inbox…
iii call approval::list-pending '{}'
# …a human allows it (the harness re-runs it through trigger)…
iii call approval::resolve '{"session_id": "s_1", "function_call_id": "c_1", "decision": "allow"}'
# …or denies it with a reason the model can adapt to.
iii call approval::resolve '{"session_id": "s_1", "function_call_id": "c_1", "decision": "deny", "reason": "not on prod"}'
```

## Permission model

Per-session mode plus two allow-lists, evaluated in this order (ported
unchanged from the proven implementation):

1. `approval::*` / `configuration::*` target → **deny** (`human_only_function`,
   even under `full` — self-escalation defense)
2. mode `full` → allow
3. `approved_always` hit → allow (**every** mode — remembered human decisions)
4. mode `auto` **and** `always_allow` hit → allow (dormant under `manual`)
5. fall through to configuration **`rules`** (first match wins):
   `allow` → allow · `deny` → deny · no match → **hold**

When the configuration entry omits `rules`, the gate denies only this
worker's own `approval::*` surface; every other call **holds**. On startup the
worker seeds/backfills the stored entry so `rules` are editable in the console.

## Custom trigger types

| Type | Fires | Payload |
|---|---|---|
| `approval::pending-created` | a call was held and its inbox record written (async, off the hot path) | `PendingApprovalRecord & { status: "pending" }` — redacted args, session context: self-sufficient for notification copy |
| `approval::pending-resolved` | a pending call left the inbox (exactly once per record) | ids + `outcome: "allow" \| "deny" \| "aborted"`, operator `reason` on deny |

Binding config (both types): `{ session_id?, metadata? }` — `metadata` is a
subset-equality match against the record's denormalized `session_metadata`,
so a multi-tenant notification worker binds to only its own sessions. After a
restart, reconcile with one `approval::list-pending` call.

## Folder-access grant watch

When a `shell::*` / `coder::*` call fails with a jail-scope error (`S215`,
`S220`, `C215`, `C220`) whose message carries a `grant_hint=<json>` tail,
`approval::grant-watch` (a `post_trigger` hook, bound `on_error: "fail_open"`
so a crashed/absent watch never turns an already-decided result into a stuck
call) converts the failure into a `kind: "folder_access"` pending record
instead of letting the raw denial reach the model. The record's
`grant_request: { dir, offending_path, error_code }` is what the console
shows: "allow access to `dir`?".

Before holding, the watch stands down (delivers the original error, no
prompt) whenever:

- the offending dir **is** the session's own workspace (`metadata.working_dir`)
  or under it — that is a harness stamp hole, not something a grant fixes;
- the harness already reports a durable grant covering the dir
  (`harness::workspace::grants`) — shell rejecting anyway is a store/stamp
  skew, not something a re-ask resolves;
- the user already **denied** this dir earlier in the session
  (`approval_grant_denied/<session_id>`);
- the call has already been re-asked `grant_reask_limit` times (default `3`,
  configurable — see below);
- the harness doesn't implement the workspace-grant control plane at all
  (`harness::workspace::grants` fails) — the watch stands down entirely
  rather than looping.

Resolving a `folder_access` record's `allow` decision takes an additional
`grant_scope` (`once` | `session` | `always`, default `once`):

| Scope | Effect |
|---|---|
| `once` | The release carries `extra_roots: [dir]` — trusted for just this one call. |
| `session` | `harness::workspace::grant` first (durable for the rest of the session), THEN release with no `extra_roots` — the harness stamps the grant on every subsequent call itself. Falls back to `once`-style `extra_roots` if the grant call fails, so the user's click still works. |
| `always` | Same as `session`, PLUS a best-effort append of `dir` to the `shell` deployment configuration's `fs.host_roots` (`configuration::get`/`configuration::set {id: "shell"}`). A persist failure is logged and never blocks the release. |

`deny` records the dir in `approval_grant_denied/<session_id>` (so the same
session won't be re-asked about it again) before delivering a denial whose
default reason mentions folder access specifically (`"user denied folder
access to <dir>"` unless the operator supplies their own).

**Degraded modes** — the watch is entirely additive and fails toward "do
nothing, deliver the original error":

- **No shell `grant_hint` support** (older shell/coder): the ladder never
  finds a hint, so `grant-watch` never triggers — behaves as if unbound.
- **No harness workspace-grant control plane** (older harness): the
  "already granted?" check fails, so every candidate stands down — no holds,
  no prompts, just the original error.
- **Harness/console down when the hook fires**: `on_error: "fail_open"`
  delivers the original error unchanged; nothing is stuck.

## Configuration

The whole config — runtime wiring **and** deployment approval defaults — lives
in the single engine configuration entry **`approval-gate`** (operator-edited
via the console's Configuration screen; reactive reload, no polling). There is
**no committed `config.yaml`**. On first boot the worker seeds the entry with
the built-in defaults (including `rules`) so the editor is pre-filled; existing
stored values are never overwritten except to add a missing `rules` field.

```jsonc
{
  "default_mode": "manual",        // manual | auto | full — sessions with no stored settings
  "rules": [                       // first match wins; no match → hold
    "!approval::*",
    { "function": "state::get", "action": "allow", "modes": ["auto"] }
  ],
  "grant_reask_limit": 3           // approval::grant-watch re-asks per held call before standing down
}
```

`configuration` is a **required boot dependency**: the worker registers the
schema and fetches the authoritative value at startup, and a failed
register/fetch aborts boot (the gate must run on a known, authoritative policy
surface, never a guessed one). When no value is stored yet the built-in
defaults above are seeded and used. **Every field hot-reloads on
`configuration::set` — nothing requires a restart**.

## Agent exposure

Deny **all** `approval::*` and `configuration::*` functions to in-run agents:
`resolve` would let an agent approve its own held calls, and the settings
RPCs are self-escalation (the gate's `human_only_function` rule is the
in-depth backstop). `list_pending` / `get_pending` are read-only and
redacted, but they enumerate held calls **across sessions** — keep them off
agent allow-lists too.

## Local development & testing

```bash
cargo test                                   # lib suites: pure unit + engine-backed handlers
cargo test --test integration               # engine-backed; self-skips without `iii`
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
./target/debug/approval-gate --manifest      # registry-publish manifest
```

The integration suite spawns a real engine (`III_ENGINE_BIN` or `iii` on
PATH) with `configuration` + `state`, registers the production surface
in-process, and fakes sibling RPCs where noted (`session::get`). With the
harness binary available, `tests/harness_integration.rs` additionally boots the
real harness worker for cross-worker hold / sweep / resolve checks.

## Architecture documentation

Deep documentation lives in [architecture/](architecture/):
[internals.md](architecture/internals.md) for maintainers (evaluation order,
the emit-gate deletion mechanics, lazy seeding, engine facts the code
depends on) and [integration.md](architecture/integration.md) for consumers
(the full function/trigger contract, the harness handoff, deployment notes,
what not to do).
