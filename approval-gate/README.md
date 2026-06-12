# approval-gate

The policy and decision surface for human-held function calls
([spec](../tech-specs/2026-06-agentic/approval-gate.md)). Three surfaces, one
worker:

1. **The gate** — `approval::gate`, a `pre_dispatch` hook the worker binds
   itself at startup on the harness's `harness::hook::pre_dispatch` trigger
   type. It evaluates per-session mode, allow-lists, and the yaml policy, and
   answers `continue`, `deny`, or `hold`.
2. **The decision plane** — `approval::resolve` plus the per-session settings
   RPCs (`set_mode`, `add_always_allow`, `approve_always`, …). Human/console
   only.
3. **The pending inbox** — an **ephemeral** index of held calls
   (`approval::list_pending` / `approval::get_pending`) plus two trigger types
   (`approval::pending_created` / `approval::pending_resolved`) that
   notification workers and UIs bind to.

The worker keeps **no resolved-approval history**: a record exists only while
a call is held; every record has an explicit deletion path and a cron sweep as
GC backstop. The transcript's `function_result` and the `pending_resolved`
event are the audit trail.

## Standalone caveat

This worker codes against the greenfield harness contracts
(`harness::hook::pre_dispatch`, `harness::function::resolve`,
`harness::turn_completed` — see harness.md § Hooks / § API Reference), which
are **not implemented by the current harness yet**. All trigger bindings are
best-effort: on an engine without those trigger types the worker still boots,
serves its RPCs, registers its configuration entry, and logs
`trigger_type_not_found` for the absent bindings (restart it after the
sibling appears to re-bind). The integration suite exercises the harness
surface against in-process fakes until harness 1.0 lands.

## Install

```bash
iii worker add approval-gate
```

The sweep needs the engine's cron worker: `iii worker add iii-cron`. Without
it the expiry backstop never fires (the harness pending sweep — once it
exists — remains the second backstop).

## Quickstart

```bash
cargo build
./target/debug/approval-gate --url ws://127.0.0.1:49134 --config ./config.yaml
```

Hold → decide → release, from any client:

```bash
# A held call shows up in the inbox…
iii call approval::list_pending '{}'
# …a human allows it (the harness re-runs it through dispatch)…
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
5. fall through to `policy::check_permissions` (5s budget):
   `allow` → allow · `deny` → deny · `needs_approval` → **hold** ·
   unparseable reply → hold · transport failure/timeout → **deny**
   (`gate_unavailable` — fail closed, never an unattended hold)

No `policy::check_permissions` worker deployed? Every non-short-circuited
call is denied as `gate_unavailable`. Run a trivial policy worker (e.g.
"everything `needs_approval`") or lean on `always_allow_seed` / per-session
modes.

## Custom trigger types

| Type | Fires | Payload |
|---|---|---|
| `approval::pending_created` | a call was held and its inbox record written (async, off the hot path) | `PendingApprovalRecord & { status: "pending" }` — redacted args, session context, expiry: self-sufficient for notification copy |
| `approval::pending_resolved` | a pending call left the inbox (exactly once per record) | ids + `outcome: "allow" \| "deny" \| "timeout" \| "aborted"`, operator `reason` on deny |

Binding config (both types): `{ session_id?, metadata? }` — `metadata` is a
subset-equality match against the record's denormalized `session_metadata`,
so a multi-tenant notification worker binds to only its own sessions. After a
restart, reconcile with one `approval::list_pending` call.

## Configuration

Deployment defaults live in the engine configuration entry **`approval-gate`**
(operator-edited via the console's Configuration screen; reactive reload, no
polling):

```jsonc
{
  "default_mode": "manual",        // manual | auto | full — sessions with no stored settings
  "always_allow_seed": [],         // auto-mode trust profile (function ids / globs)
  "pending_timeout_ms": 1800000    // hold deadline; drives expires_at (default 30 min)
}
```

Without the configuration worker the gate runs on those built-in defaults —
fail-safe, never fail-open.

`config.yaml` carries runtime wiring only (hook binding globs/budget, sweep
cron expression, per-call timeouts) — see the file's comments.

## Agent exposure

Deny **all** `approval::*` and `configuration::*` functions to in-run agents:
`resolve` would let an agent approve its own held calls, and the settings
RPCs are self-escalation (the gate's `human_only_function` rule is the
in-depth backstop). `list_pending` / `get_pending` are read-only and
redacted, but they enumerate held calls **across sessions** — keep them off
agent allow-lists too.

## Local development & testing

```bash
cargo test                                   # unit suites (engine-free, FakeBus)
cargo test --test integration               # engine-backed; self-skips without `iii`
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
./target/debug/approval-gate --manifest      # registry-publish manifest
```

The integration suite spawns a real engine (`III_ENGINE_BIN` or `iii` on
PATH) with `configuration` + `iii-state`, registers the production surface
in-process, and fakes the not-yet-built siblings
(`policy::check_permissions`, `harness::function::resolve`, `session::get`).

## Architecture documentation

Deep documentation lives in [architecture/](architecture/):
[internals.md](architecture/internals.md) for maintainers (evaluation order,
the emit-gate deletion mechanics, lazy seeding, engine facts the code
depends on) and [integration.md](architecture/integration.md) for consumers
(the full function/trigger contract, the harness handoff, deployment notes,
what not to do).
