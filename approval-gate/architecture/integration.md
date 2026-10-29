# approval-gate integration contract

For authors of workers and clients that call `approval::*` or observe its
event functions: the console's approval UI, notification workers, the
harness, dashboards. Maintainer internals live in
[internals.md](internals.md); spec authority is
[tech-specs/2026-06-agentic/approval-gate.md](../../tech-specs/2026-06-agentic/approval-gate.md)
(§ API Reference for full request/response types — this file is the
operational contract).

## Function surface

| Function | Caller | Purpose |
|---|---|---|
| `approval::gate` | harness only (the target of this worker's observer of `harness::hook::on-pre-call`) | The hook: `HookInput` → `{ decision: "continue" \| "deny" \| "hold" }`. Never call directly. |
| `approval::filesystem-access-watch` | harness only (the target of this worker's observer of `harness::hook::on-post-call`) | The post-call hook: `HookInput` with the function result → `continue` or a `filesystem_access` hold. Never call directly. |
| `approval::resolve` | console / inbox UI (human-only) | Apply a decision to a held call: `{ session_id, function_call_id, decision: "allow" \| "deny", reason? }` → `{ resolved, turn_resumed? }`. |
| `approval::list-pending` | console / notification workers | The inbox: filters `session_id?`, `metadata?` (subset-equality tenancy match), `limit?` (default 50), opaque `cursor?`; ordered by `pending_at` asc. |
| `approval::get-pending` | console | One record or `null`. |
| `approval::set-mode` | console (human-only) | `manual` / `auto` / `full`. |
| `approval::add-always-allow` / `remove-always-allow` | console (human-only) | Curate the auto-mode trust list (idempotent add / no-op remove). |
| `approval::approve-always` | console (human-only) | Per-session grant honoured in **every** mode; call it right before `resolve { decision: "allow" }` for an "Approve always" button. |
| `approval::get-settings` | console | Effective settings + `source: "stored" \| "defaults"`. Never writes. |
| `approval::clear-settings` | console | Drop the stored record; revert to deployment defaults. |
| `approval::on-config-change` / `on-session-deleted` / `on-turn-completed` | targets of this worker's observers of `configuration::on-updated`, `session::on-deleted`, `harness::on-turn-completed` | Internal — never call directly. |

Errors use `code: message` with codes `approval/invalid_payload`,
`approval/state_unavailable`, `approval/harness_unavailable`. An unknown
`{ session_id, function_call_id }` on `resolve` is **not** an error — it
returns `{ resolved: false }` (duplicate decisions race benignly).
`session_id` / `function_call_id` must not contain `/`.

## Event functions

`approval::on-pending-created` and `approval::on-pending-resolved` are
private functions: only this worker calls them, and anyone may observe them.
Each handler returns the event it is called with, so an observer's `input`
and `output` are both the event. Neither takes args; select events with
filters. Delivery is fire-and-forget, at-least-once, **unordered** —
reconcile with one `approval::list-pending` call after a restart. The worker
calls each function only while something observes it.

### `approval::on-pending-created`

A call was held and its inbox record written. Called asynchronously after the
hook returns `hold` — never on the hook's hot path.

Event: the `PendingApprovalRecord` plus `status: "pending"` — ids
(`session_id`, `turn_id`, `function_call_id`, `function_id`), redacted
`arguments_excerpt`, `pending_at`, denormalized
`session_title` / `session_description` / `session_metadata` (omitted when
session-manager was unreachable at hold time), sub-agent `depth`, `kind`
and, for a `filesystem_access` hold, `access_request`.
Self-sufficient for notification copy — no follow-up reads needed, and safe
to forward to push/Slack payloads (arguments are redacted and clipped).

### `approval::on-pending-resolved`

A pending call left the inbox. Called **exactly once per record** — your
badge-clearing logic can trust it. Event: ids plus
`outcome: "allow" | "deny" | "aborted"`, operator `reason` (deny
only), `session_metadata`, `resolved_at`.

### Observing them

| Select | Filter |
|---|---|
| one session | `{ "input.session_id": { "$eq": "s_1" } }` |
| one tenant (subset of `session_metadata`) | `{ "input.session_metadata.owner": { "$eq": "u_1" } }`, one entry per key |

```ts
iii.observe(
  {
    function_id: 'approval::on-pending-created',
    filters: { 'input.session_metadata.owner': { $eq: 'u_1' } },
    deliver: 'output',   // the target receives the event itself
  },
  'notify::approval-requested',
)
```

A multi-tenant notification worker observes with its tenancy keys and
receives only its own sessions' events. An observer that passes args is
refused with `INVALID_ARGS`.

## The decision flow, end to end

```mermaid
sequenceDiagram
  participant H as harness
  participant AG as approval-gate
  participant UI as console
  participant N as notify worker
  H->>AG: approval::gate (harness::hook::on-pre-call)
  AG-->>H: { decision: "hold", pending_timeout_ms: 0 }
  AG--)N: approval::on-pending-created
  UI->>AG: approval::resolve { decision: "allow" }
  AG->>H: harness::function::resolve { action: "execute" }
  Note over H: re-enqueue turn; run the released call through the remaining call pipeline
  AG--)N: approval::on-pending-resolved { outcome: "allow" }
```

On `deny`, the gate calls `harness::function::resolve` with
`action: "deliver"`, `is_error: true`, a text rendering of the reason in
`content`, and the full `DenialEnvelope` in `details` — the model sees it
and can adapt.

## Harness contract

What the harness provides — and what this worker assumes, faked in-process
by the test suites when the harness binary is not available:

- **`harness::hook::on-pre-call`.** The worker observes it with
  `approval::gate` at startup, with args
  `{ functions: ["*"], timeout_ms: 5000, on_error: "fail_closed" }` (fixed —
  not in the configuration entry). The harness calls the gate through that
  observer synchronously and treats the return value as `HookOutput`.
- **`harness::hook::on-post-call`.** The worker observes it with
  `approval::filesystem-access-watch`, with args
  `{ functions: ["shell::*", "coder::*"], timeout_ms: 5000, on_error: "fail_open" }`.
- **`harness::function::resolve`** accepting
  `{ session_id, turn_id, function_call_id, action: "execute" }` (release a
  held call) and `{ ..., action: "deliver", is_error, content, details }`
  (settle with a result), idempotent on the deterministic entry id,
  returning `{ resolved, turn_resumed }`.
- **`harness::on-turn-completed`** with at least `turn_id` in the event
  (terminal-turn purge).

The observers stay `pending` while the harness is not running and become
active when it registers its functions, so the start order of the two
workers does not matter. The `pre_call` ordering caveat from the spec
applies: hooks run *after* the harness's fail-closed allow/deny globs — a
deployment that wants everything gated sets a broad function policy and lets
the gate hold/deny.

## Deployment notes

- **session-manager** (soft): provides hold-time context and the
  `session::on-deleted` cascade. Without it, records carry no session context
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
  remaining call pipeline (post-call hooks, redaction, checkpoints,
  provenance).
- **Don't persist approval history from the inbox** — records vanish on
  resolution by design. Observe `approval::on-pending-resolved` and keep
  your own log if your deployment needs one.
- **Don't poll `list_pending` for liveness** — observe the event functions;
  use `list_pending` for reconciliation and initial render.
