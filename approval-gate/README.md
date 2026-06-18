# approval-gate

The policy and decision surface for human-held function calls
([spec](../tech-specs/2026-06-agentic/approval-gate.md)). Three surfaces, one
worker:

1. **The gate** — `approval::gate`, a `pre_trigger` hook the worker binds
   itself at startup on the harness's `harness::hook::pre-trigger` trigger
   type. It evaluates per-session mode, allow-lists, and inline config
   `rules`, and answers `continue`, `deny`, or `hold`.
2. **The decision plane** — `approval::resolve` plus the per-session settings
   RPCs (`set_mode`, `add_always_allow`, `approve_always`, …). Human/console
   only.
3. **The pending inbox** — an **ephemeral** index of held calls
   (`approval::list-pending` / `approval::get-pending`) plus two trigger types
   (`approval::pending-created` / `approval::pending-resolved`) that
   notification workers and UIs bind to.

The worker keeps **no resolved-approval history**: a record exists only while
a call is held; every record has an explicit deletion path (resolve, turn
abort, session delete). The transcript's `function_result` and the
`pending_resolved` event are the audit trail. Holds do not expire — they wait
until a human resolves or the turn/session is purged.

## Standalone caveat

This worker codes against the greenfield harness contracts
(`harness::hook::pre-trigger`, `harness::function::resolve`,
`harness::turn-completed` — see harness.md § Hooks / § API Reference), which
are **not implemented by the current harness yet**. The harness/session
trigger bindings are best-effort: on an engine without those trigger types the
worker still boots, serves its RPCs, and logs `trigger_type_not_found` for the
absent bindings (restart it after the sibling appears to re-bind). The
`configuration` worker is the one hard dependency — it is enabled by default in
the engine, and a failed register/fetch aborts boot. The integration suite
exercises the harness surface against in-process fakes until harness 1.0 lands.

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
  ]
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
PATH) with `configuration` + `iii-state`, registers the production surface
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
