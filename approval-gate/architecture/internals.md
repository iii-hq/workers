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
| `permissions/` | Inline rule compiler + evaluator (first match wins; no match → hold). Built-in default: deny `approval::*` only. |
| `denial.rs` | `DenialEnvelope` assembly + text rendering; reason strings ported verbatim from the prior art. |
| `redact.rs` | Recursive argument redaction (pure port of the proven `redact.ts`). |
| `settings.rs` | Effective-settings computation, lazy seeding, immutable mutation helpers, tolerant vs strict reads. |
| `pending.rs` | Inbox record store: `get`/`put`/`list_all` and **`delete_with_gate`** — the single deletion helper. |
| `config.rs` | The single `WorkerConfig` (Path B): serde + schemars schema, `from_yaml`/`from_json`/`to_json`/`json_schema`/`boot_signature` (`hook` is the structural field — re-bound live on change) and `permissions()` (compiles the inline `rules`). |
| `configuration.rs` | `configuration` worker integration: `register_config` / `fetch_config`, the `ConfigCell` snapshot, `reloadable`, and the typed, re-fetching `approval::on-config-change` trigger handler. |
| `events.rs` | The two custom trigger types, `SubscriberSet`s, binding filters, the `EventSink` trait + `Emitter` (Void-action fan-out). |
| `functions/` | One file per `approval::*` function; `mod.rs` holds `Deps` and the typed registration helper. |
| `main.rs` | Boot order: register+fetch config (fatal) → trigger types → functions → best-effort hook/session bindings → `register_config_trigger` (last). |

Every handler takes `Deps { iii, sink, config }` — a `ConfigCell` it snapshots
once per call via `deps.config().await` — and reaches siblings
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
3. **One** config + settings snapshot: `deps.config().await` for the live
   `WorkerConfig`, then `state::get approval_settings/<sid>` (tolerant — an
   outage degrades to configuration defaults, which never widen beyond what the
   operator configured).
4. Pure short-circuits (`decision::pre_policy_allow`): `full` → continue;
   `approved_always` hit → continue (every mode); `auto` + `always_allow`
   hit → continue. Allow-list entries match by equality fast-path or `*`
   glob (seed entries are documented as "ids / globs").
5. Config rules fallback: `allow` → continue; `deny` → deny with the permissions
   envelope reason; no match → **hold**.
6. Hold path:
   - Idempotency first: an existing record (redelivered at-least-once step)
     returns `hold` with `pending_timeout_ms: 0` without rewriting or re-emitting.
   - `session::get` soft-fetch under its own `session_fetch_timeout_ms`
     budget; context fields are omitted on any failure.
   - Record written **synchronously before returning hold** — write failure
     → deny (`gate_unavailable`), never hold blind. A non-null `old_value`
     on the write means a concurrent duplicate won the race: skip emission.
   - `pending_created` emits via `tokio::spawn` **after** the hook returns —
     fan-out never blocks the trigger hot path.

## State lifecycle and the emit gate

Two scopes, both with explicit deletion paths:

| Scope/key | Created | Deleted by |
|---|---|---|
| `approval_pending/<sid>/<cid>` | in-hook, before `hold` returns | resolve · `harness::turn-completed` · `session::deleted` |
| `approval_settings/<sid>` | first user mutation (lazy; reads never write) | `session::deleted` · `approval::clear-settings` |

All three pending-deletion paths funnel through
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
turn/session cleanup collects it; a decision is never lost.

## Settings: lazy seeding

`effective(stored, cfg)`: a stored record wins; otherwise the settings
are computed in memory from the live `WorkerConfig` (`default_mode` +
`always_allow_seed`; seed entries carry
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

`configuration.rs` owns the `configuration` worker integration — Path B,
mirroring `context-manager` / `session-manager`. `register_config` registers
the `WorkerConfig` JSON Schema and seeds `WorkerConfig::default()` as
`initial_value` only when no value is stored yet (re-registration preserves the
stored value); `ensure_rules_seeded` then backfills a missing `rules` field
into an already-stored value (a pre-rules config) without clobbering operator
edits; `fetch_config` reads the authoritative, env-expanded value at boot. The
register/fetch pair is **required** — a failed register/fetch aborts boot, so
the gate always runs on a known, authoritative policy surface (never a guessed
one). When nothing is stored, the built-in defaults (`manual`, `[]`, the
shipped `!approval::*` rules, the `*` hook) are what gets seeded and used.

The live value is held in a `ConfigCell` (`Arc<RwLock<Arc<WorkerConfig>>>`)
that every handler snapshots per call. `register_config_trigger` registers the
**typed** `approval::on-config-change` handler (`OnConfigChangeEvent` →
`OnConfigChangeResponse` — never a `Value` handler, registered off the public
`catalog()`) and binds the `configuration` trigger. On `configuration:updated`
it **re-fetches** via `configuration::get` (ignoring the trigger payload, so a
direct call can't inject config) and swaps the cell. A change to the boot
signature (`hook`) **re-binds** the hook live:
`register_config_trigger` retains the `Trigger` handle in `TriggerHandles`, and
the handler registers the new binding then `unregister()`s the old (a fail-safe
overlap — the gate is idempotent, so a brief double-fire is harmless), so no
field requires a restart; every other field hot-applies. The config parse is **strict**
(`deny_unknown_fields`): an unparseable stored value is rejected and the
last-good snapshot kept, so a typo'd operator edit can't silently widen
access. (Per-session settings records keep their own tolerant read — see
*Settings: lazy seeding*.)

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
- `register_trigger` acks asynchronously: `Ok` means "request sent"; a
  missing trigger type surfaces later as an SDK-level
  `trigger_type_not_found` error log. Boot therefore never depends on
  binding success.

## Testing

- **Unit** (`cargo test`): every module has a suite beside it. Pure-logic
  modules (`decision`, `redact`, `denial`, `settings`, `permissions`, `types`, …) run with
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
