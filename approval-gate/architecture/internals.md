# approval-gate internals

For maintainers changing this worker. The integration contract lives in
[integration.md](integration.md); spec authority is
[tech-specs/2026-06-agentic/approval-gate.md](../../tech-specs/2026-06-agentic/approval-gate.md).

## Module map

| Module | Responsibility |
|---|---|
| `types.rs` | Every wire type (serde + schemars), id validation (`/` is the reserved key separator), `metadata_matches` subset-equality. |
| `state.rs` / `harness.rs` / `session.rs` | Thin per-target `iii.trigger` wrappers (state kv, `harness::function::resolve`, `session::get`). No transport abstraction; tests run against a real engine via `testkit/engine.rs`. |
| `decision.rs` | **Pure** evaluation order (no I/O): human-only prefix check, mode/allow-list short-circuits, `*`-glob matching. |
| `policy.rs` | `policy::check_permissions` client: 5s budget, reply parsing, failure mapping. |
| `denial.rs` | `DenialEnvelope` assembly + text rendering; reason strings ported verbatim from the prior art. |
| `redact.rs` | Recursive argument redaction (pure port of the proven `redact.ts`). |
| `settings.rs` | Effective-settings computation, lazy seeding, immutable mutation helpers, tolerant vs strict reads. |
| `pending.rs` | Inbox record store: `get`/`put`/`list_all` and **`delete_with_gate`** — the single deletion helper. |
| `gate_config.rs` | The `approval-gate` configuration entry: schema, field-wise tolerant parse, `Arc<RwLock<GateDefaults>>`. |
| `events.rs` | The two custom trigger types, `SubscriberSet`s, binding filters, the `EventSink` trait + `Emitter` (Void-action fan-out). |
| `functions/` | One file per `approval::*` function; `mod.rs` holds `Deps` and the typed registration helper. |
| `main.rs` | Boot order: trigger types → functions → best-effort bindings → configuration entry + initial read. |

Every handler takes `Deps { iii, sink, defaults, cfg }` and reaches siblings
through the thin wrapper modules. Pure-logic modules are unit-tested with no
engine; the `approval::*` handlers are driven against a real spawned engine
via `testkit::engine` (see Testing).

## The gate's control flow (`functions/gate.rs`)

The hook **never returns an error**: every failure mode resolves to a
fail-closed `deny` (the harness's `on_error: fail_closed` would read an
exception the same way, but an explicit deny carries a reason the model can
adapt to).

1. No `call` payload, or `/` / empty `session_id` / `call.id` → deny (a call
   whose ids can't key a pending record can never be held).
2. Human-only: `function_id` starts with `approval::` or `configuration::` →
   deny `human_only_function`. Runs **before** the settings snapshot, even
   under `full` — self-escalation defense. (Deliberately broader than the
   prior art's six-id list.)
3. **One** settings snapshot: `state::get approval_settings/<sid>` (tolerant —
   an outage degrades to configuration defaults, which never widen beyond
   what the operator configured) merged with the in-memory `GateDefaults`.
4. Pure short-circuits (`decision::pre_policy_allow`): `full` → continue;
   `approved_always` hit → continue (every mode); `auto` + `always_allow`
   hit → continue. Allow-list entries match by equality fast-path or `*`
   glob (seed entries are documented as "ids / globs").
5. Policy fallback: `allow` → continue; `deny` → deny with the permissions
   envelope reason; **unparseable reply → hold** (a human look is the safe
   reading of "don't know"); **transport failure / timeout → deny**
   (`gate_unavailable` — never an unattended hold).
6. Hold path:
   - Idempotency first: an existing record (redelivered at-least-once step)
     returns `hold` without rewriting or re-emitting.
   - `session::get` soft-fetch under its own `session_fetch_timeout_ms`
     budget; context fields are omitted on any failure.
   - Record written **synchronously before returning hold** — write failure
     → deny (`gate_unavailable`), never hold blind. A non-null `old_value`
     on the write means a concurrent duplicate won the race: skip emission.
   - `pending_created` emits via `tokio::spawn` **after** the hook returns —
     fan-out never blocks the dispatch hot path.

## State lifecycle and the emit gate

Two scopes, both with explicit deletion paths:

| Scope/key | Created | Deleted by |
|---|---|---|
| `approval_pending/<sid>/<cid>` | in-hook, before `hold` returns | resolve · `harness::turn_completed` · `session::deleted` · sweep on `expires_at` |
| `approval_settings/<sid>` | first user mutation (lazy; reads never write) | `session::deleted` · `approval::clear_settings` |

All four pending-deletion paths funnel through
`pending::delete_with_gate`, which is where exactly-once emission is decided:

1. `state::set { value: null }` — the engine swaps the value under its write
   lock and returns the prior one. **This is the atomic gate**: across any
   set of racing deleters, exactly one observes the live record.
2. `state::delete` — cleanup. The engine does **not** treat a null set as a
   delete; it stores a literal null tombstone which `state::list` would
   return forever (verified against the engine source, `builtins/kv.rs`).
   The follow-up delete removes it. A failed cleanup is benign: readers
   skip nulls (`parse_record`), and the next deletion attempt re-deletes.
3. Only a caller that got `Some(record)` back emits `pending_resolved`.

Why not `state::delete` alone? The engine's delete handler is get-then-delete
(non-atomic at the worker layer) — two racing deleters could both read the
live value and double-emit.

Crash ordering in resolve: `harness::function::resolve` **first**, then
delete, then emit. A crash between resolve and delete leaks one record until
the sweep collects it; a decision is never lost. The sweep tolerates
`{ resolved: false }` and transport errors from the harness and deletes the
expired record regardless — the inbox must stay O(live holds) even in a
deployment with no harness at all.

## Settings: lazy seeding

`effective(stored, defaults)`: a stored record wins; otherwise the settings
are computed in memory from `GateDefaults` (seed entries carry
`granted_by: "seed"`). Reads — including the gate's hot path and
`get_settings` — never write. The first mutation materializes the record
from the **current** defaults, applies the change, and writes the whole
record once (`materialize_and`); from then on the stored record wins and
later seed changes don't retroactively edit it. Mutations use a **strict**
read (a state outage errors rather than re-seeding over an unreadable
record); the gate uses a **tolerant** read (outage degrades to defaults —
manual mode allows nothing extra, so degradation can't widen access).

Mutation helpers are immutable: `with_grant` (idempotent on exact
`function_id`) and `without_grant` return new lists.

## Configuration reload

`gate_config.rs` registers entry `approval-gate` **without** `initial_value`
(llm-router precedent: operator-stored values survive every re-register);
built-in defaults (`manual`, `[]`, 30 min) apply in memory whenever the
entry value is null or the configuration worker is absent. Parsing is
field-wise tolerant — one malformed field degrades to its default, never
fails the gate open. `approval::on_config_change` guards on
`id == "approval-gate"` and swaps the shared `RwLock`. Boot order: bind the
configuration trigger, register the entry, then one initial
`configuration::get` — an update landing in the gap is caught by either the
read or the trigger.

## Redaction (`redact.rs`)

Behavior-for-behavior port of the proven `redact.ts`: secret-keyed values
(11 keys + case-insensitive `_<key>` suffix match) collapse to
`"<redacted>"` whatever their type; strings clip at 256 **code points**
(`char`, not bytes — multi-byte-heavy strings within the cap must not be
clipped) with `…`; recursion is capped at depth 64 → `"<max-depth>"`
sentinel; the walk never mutates its input. Applied once, at record-build
time — `resolve` passes the stored excerpt through without re-redacting.

## Engine facts this code depends on

Verified against the engine source (`~/workspaces/personal/motia/iii`):

- `state::set` returns `{ old_value, new_value }` atomically; a null value
  is **stored**, not a delete (hence the two-step delete above).
- `state::get` / `state::delete` return the (old) value or null;
  `state::list` returns the scope's **values only**, no keys, no pagination.
- The cron trigger's config key is **`expression`** (6-field cron), not
  `schedule` — `docs/sops/binary-worker.md` is stale on this.
- `register_trigger` acks asynchronously: `Ok` means "request sent"; a
  missing trigger type surfaces later as an SDK-level
  `trigger_type_not_found` error log. Boot therefore never depends on
  binding success.

## Testing

- **Unit** (`cargo test`): every module has a suite beside it. Pure-logic
  modules (`decision`, `redact`, `denial`, `settings`, `types`, …) run with
  no engine. The `approval::*` handlers run against a real spawned engine via
  `testkit::engine` (`III_ENGINE_BIN` or `iii` on PATH; self-skips
  otherwise), so `delete_with_gate`'s null-tombstone invariant is exercised
  against the genuine kv contract. The gate suite reproduces the seven
  prior-art permission-matrix cases plus the fail-closed rows.
- **Integration** (`cargo test --test integration`): spawns a real engine
  (`III_ENGINE_BIN` or `iii` on PATH; self-skips otherwise) with
  `configuration` + `iii-state` real and the unbuilt siblings faked
  in-process. Verifies the no-tombstone invariant, exactly-once event
  delivery through real engine fan-out, and reactive configuration reload.
