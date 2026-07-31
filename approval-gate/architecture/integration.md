# approval-gate integration contract

For authors of workers and clients that call `approval::*` or bind its
trigger types: the console's approval UI, notification workers, the harness
(once its hook surface lands), dashboards. Maintainer internals live in
[internals.md](internals.md); spec authority is
[tech-specs/2026-06-agentic/approval-gate.md](../../tech-specs/2026-06-agentic/approval-gate.md)
(§ API Reference for full request/response types — this file is the
operational contract).

## Function surface

| Function | Caller | Purpose |
|---|---|---|
| `approval::gate` | harness only (via the `harness::hook::pre-trigger` binding) | The hook: `HookInput` → `{ decision: "continue" \| "deny" \| "hold" }`. Never call directly. |
| `approval::resolve` | console / inbox UI (human-only) | Apply a decision to a held call: `{ session_id, function_call_id, decision: "allow" \| "deny", reason? }` → `{ resolved, turn_resumed? }`. |
| `approval::list-pending` | console / notification workers | The inbox: filters `session_id?`, `metadata?` (subset-equality tenancy match), `limit?` (default 50), opaque `cursor?`; ordered by `pending_at` asc. |
| `approval::get-pending` | console | One record or `null`. |
| `approval::set-mode` | console (human-only) | `manual` / `auto` / `full`. |
| `approval::add-always-allow` / `remove-always-allow` | console (human-only) | Curate the auto-mode trust list (idempotent add / no-op remove). |
| `approval::approve-always` | console (human-only) | Per-session grant honoured in **every** mode; call it right before `resolve { decision: "allow" }` for an "Approve always" button. |
| `approval::get-settings` | console | Effective settings + `source: "stored" \| "defaults"`. Never writes. |
| `approval::clear-settings` | console | Drop the stored record; revert to deployment defaults. |
| `approval::on-config-change` / `on-session-deleted` / `on-turn-completed` | trigger handlers | Internal — never call directly. |

Errors use `code: message` with codes `approval/invalid_payload`,
`approval/state_unavailable`, `approval/harness_unavailable`. An unknown
`{ session_id, function_call_id }` on `resolve` is **not** an error — it
returns `{ resolved: false }` (duplicate decisions race benignly).
`session_id` / `function_call_id` must not contain `/`.

## Trigger types

Bind with the standard two-step pattern (register your handler function,
then `registerTrigger` with the type). Delivery is fire-and-forget,
at-least-once, **unordered** — reconcile with one `approval::list-pending`
call after a restart.

### `approval::pending-created`

A call was held and its inbox record written. Fires asynchronously after the
hook returns `hold` — never on the trigger hot path.

Payload: the `PendingApprovalRecord` plus `status: "pending"` — ids
(`session_id`, `turn_id`, `function_call_id`, `function_id`), redacted
`arguments_excerpt`, `pending_at`, denormalized
`session_title` / `session_description` / `session_metadata` (omitted when
session-manager was unreachable at hold time), sub-agent `depth`.
Self-sufficient for notification copy — no follow-up reads needed, and safe
to forward to push/Slack payloads (arguments are redacted and clipped).

### `approval::pending-resolved`

A pending call left the inbox. Emitted **exactly once per record** — your
badge-clearing logic can trust it. Payload: ids plus
`outcome: "allow" | "deny" | "aborted"`, operator `reason` (deny
only), `session_metadata`, `resolved_at`.

### Binding config (both types)

```jsonc
{ "session_id": "s_1",                 // optional: one session only
  "metadata": { "owner": "u_1" } }     // optional: subset-equality vs session_metadata
```

Unknown config fields are rejected at registration (a typo'd filter fails
loudly). A multi-tenant notification worker binds with its tenancy metadata
and receives only its own sessions' events.

## The decision flow, end to end

```mermaid
sequenceDiagram
  participant H as harness
  participant AG as approval-gate
  participant UI as console
  participant N as notify worker
  H->>AG: approval::gate (pre_trigger hook)
  AG-->>H: { decision: "hold", pending_timeout_ms: 0 }
  AG--)N: approval::pending-created
  UI->>AG: approval::resolve { decision: "allow" }
  AG->>H: harness::function::resolve { action: "execute" }
  Note over H: re-enqueue turn; run the released call through the remaining trigger pipeline
  AG--)N: approval::pending-resolved { outcome: "allow" }
```

On `deny`, the gate calls `harness::function::resolve` with
`action: "deliver"`, `is_error: true`, a text rendering of the reason in
`content`, and the full `DenialEnvelope` in `details` — the model sees it
and can adapt.

## Harness contract

What the (future) harness must provide — and what this worker already
assumes, faked today by `tests/integration.rs`:

- **`harness::hook::pre-trigger` trigger type.** The worker binds
  `approval::gate` at startup with
  `{ functions: ["*"], timeout_ms: 5000, on_error: "fail_closed" }` at worker
  startup (fixed — not in the configuration entry). The hook is an ordinary registered
  function: the harness invokes it synchronously and treats the return value as
  `HookOutput`.
- **`harness::function::resolve`** accepting
  `{ session_id, turn_id, function_call_id, action: "execute" }` (release a
  held call) and `{ ..., action: "deliver", is_error, content, details }`
  (settle with a result), idempotent on the deterministic entry id,
  returning `{ resolved, turn_resumed }`.
- **`harness::turn-completed` trigger type** with at least `turn_id` in the
  payload (terminal-turn purge).

Until those exist the bindings log `trigger_type_not_found` at boot
(harmless); **restart the worker after the harness lands to re-bind**. The
`pre_trigger` ordering caveat from the spec applies: hooks run *after* the
harness's structural function policy. The shipped deny-only default permits
every non-denied call to reach the gate, which then holds or denies it.

## Deployment notes

- **session-manager** (soft): provides hold-time context and the
  `session::deleted` cascade. Without it, records carry no session context
  and settings cleanup relies on `approval::clear-settings`.
- **Configuration (required)**: the worker's config — the approval defaults
  (`default_mode`, `rules`) — lives in the `approval-gate` configuration entry; there is **no
  `config.yaml`**. It is a required boot dependency: a failed register/fetch
  aborts startup. `configuration::set` replaces the **whole** value —
  read-merge-write to edit one field. When `rules` is omitted, the built-in
  shipped defaults apply. Every field hot-reloads via snapshot swap.

## What not to do

- **Never expose `approval::*` or `configuration::*` to in-run agents.** An
  agent with `resolve` approves its own calls; with `set_mode` it
  self-escalates to `full`. The gate's `human_only_function` rule is the
  backstop, not the primary defense — keep these off agent allow-lists.
- **Don't give agents `list_pending` / `get_pending` either**: read-only and
  redacted, but they enumerate held calls **across sessions** (same
  multi-tenant leak caveat as `session::list`).
- **Don't execute a held function yourself** after an allow — always go
  through `approval::resolve` so the harness runs the call through its
  remaining trigger pipeline (post_trigger redaction, checkpoints,
  provenance).
- **Don't persist approval history from the inbox** — records vanish on
  resolution by design. Bind `pending_resolved` and keep your own log if
  your deployment needs one.
- **Don't poll `list_pending` for liveness** — bind the trigger types; use
  `list_pending` for reconciliation and initial render.
