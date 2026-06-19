# telegram-bot internals

This is the deep tour of everything that isn't ingress or configuration (covered
in [telegram-api.md](telegram-api.md) and [configuration.md](configuration.md)):
the reactive bindings to sibling workers, the render/streaming state machine, the
durable KV schema, the per-chat FSM, preference resolution, trace correlation,
and the concurrency model that holds it all together.

## 1. Crate layout

```
src/
├── main.rs            boot: register, fetch config, wire functions/triggers, start ingress
├── lib.rs             module exports
├── config.rs          WorkerConfig schema + WebhookConfig::endpoint_url
├── configuration.rs   configuration-worker integration + hot-reload
├── ingress.rs         polling loop + updates-adapter / webhook-route lifecycle
├── deps.rs            Deps + RuntimeState (all in-memory concurrency state)
├── kv.rs              durable state (the state-worker key schema)
├── types.rs           Telegram + harness/approval wire types; ChatFsm
├── preferences.rs     per-chat override > global config resolution
├── telemetry.rs       OpenTelemetry baggage + correlation ids
├── surface.rs         RPC function catalog (schemas, golden-tested)
├── text.rs            text utilities (UTF-8-safe truncation, etc.)
├── clients/           outbound RPC clients
│   ├── telegram.rs    Telegram Bot API client
│   ├── harness.rs     harness::send / stop / status
│   ├── approval.rs    approval::resolve / approve-always / list-pending
│   ├── router.rs      router::models::list
│   └── state.rs       state::get / set / delete (typed wrappers)
├── functions/
│   ├── mod.rs         function registration; trigger binding; webhook-route helpers
│   ├── webhook.rs     ingress sink + command/callback router + harness drive
│   ├── set_webhook.rs manual setWebhook re-arm endpoint
│   └── bindings/      the six sibling-event handlers
└── render/            outbound streaming pipeline
    ├── stream.rs      the render state machine (the heart)
    ├── verbosity.rs   phase classification + verbosity gating
    ├── format.rs      markdown → Telegram HTML
    ├── chunk.rs       ≤4096-byte UTF-8-safe splitting
    ├── throttle.rs    edit/draft rate limiting + revision freshness
    └── typing.rs      typing-indicator lifecycle
```

## 2. Two lifecycles

**Inbound (user → agent).** An update reaches `webhook::process_update`. A
command runs locally; a callback resolves a model pick or approval; plain text is
forwarded to `harness::send` (with the chat's selected model, effective thinking
level, allowed-functions policy, and system prompt). The first send in a chat has
no session id — harness assigns one, which the worker persists as the chat's
session. Subsequent sends reuse it until `/start` resets the chat.

**Outbound (agent → user).** Driving a turn makes `session-manager`/`harness`
emit events, which the engine routes to this worker's bindings, which call into
the render pipeline, which calls the Telegram API. See [§3](#3-reactive-bindings)
and [§5](#5-the-render-state-machine).

## 3. Reactive bindings

`functions::bind_triggers` binds six handlers to sibling triggers (best-effort —
a missing sibling only warns, so a binding can silently never fire):

| Trigger | Handler | Role filter | On fire |
|---|---|---|---|
| `session::message-added` | `on-message-added` | `assistant`, `function_result` | Start rendering a new entry (assistant text/thinking, or a verbosity-gated function-result bubble). |
| `session::message-updated` | `on-message-updated` | `assistant` | Apply an incremental revision to an existing entry (or reconcile if already finalized). |
| `session::status-changed` | `on-status-changed` | — | React to session status (drives typing/working state). |
| `harness::turn-completed` | `on-turn-completed` | — | Finalize all entries, suppress typing, drain one FIFO-queued message. |
| `approval::pending-created` | `on-pending-created` | — | Post the Approve/Reject/Approve-always keyboard. |
| `approval::pending-resolved` | `on-pending-resolved` | — | Clear the keyboard once resolved (by any surface). |

Two invariants:

- **No chat → no-op.** Every binding resolves `chat_id` from
  `kv::chat_id_for_session(session_id)`; if there's no mapping it returns
  `ok: true` and does nothing. (Bindings only know `session_id`; the reverse
  KV mapping is how they find the chat to post into.)
- **Role filtering is the sibling's job** (passed in the trigger config), but
  `on-message-added` re-checks the role defensively.

A subtle ordering rule: `on-turn-completed` **suppresses typing before it
finalizes**, because `turn-completed` can arrive before a stale
`status-changed(working)` — otherwise typing would restart after the final
answer.

### Approval flow end to end

```
approval-gate emits pending-created ─► on-pending-created:
     post inline keyboard [Approve a:<tok>] [Reject d:<tok>] [Approve always w:<tok>]
     store cb:<tok> → ApprovalCallbackData{ session_id, function_call_id, function_id }
     remember approval:<sid>:<fcid>:msg (the keyboard message id)

user taps a button ─► webhook handle_callback:
     a: → approval::resolve(Allow)
     d: → approval::resolve(Deny, reason)
     w: → approval::approve-always(function_id) THEN approval::resolve(Allow)
     answerCallbackQuery(toast); delete cb token

approval-gate emits pending-resolved ─► on-pending-resolved:
     clear the inline keyboard (editMessageReplyMarkup)
```

The callback token is a non-cryptographic 32-bit hash of `function_call_id`;
`resolve_callback` is a silent no-op if the token was already consumed, so
double-taps and redelivery are safe. `catch_up_approvals` reconciles any pending
approvals that have no keyboard message yet (e.g. created while the bot was down).

## 4. Sibling RPC clients

| Client call | Sibling function | Purpose |
|---|---|---|
| `harness::send` | `harness::send` | Drive a turn; returns `{session_id, turn_id}`. Carries model, thinking level, functions policy, system prompt, session seed, idempotency key, trace metadata. |
| `harness::stop` | `harness::stop` | Cancel the active turn (`/stop`, `/start` reset). |
| `harness::status_active` | `harness::status` | Is a turn running? (FIFO steering gate.) |
| `router::list_models` | `router::models::list` | Populate the `/model` picker. |
| `approval::resolve` / `approve_always` / `list_pending` | `approval::*` | Resolve approvals; whitelist a tool; reconcile pending. |
| `state::get` / `set` / `delete` | `state::*` | All durable KV (see [§6](#6-durable-state-the-kv-schema)). |

`harness::send` is given an `idempotency_key` (the Telegram `update_id`, or
`tg-fifo-{chat}` for queue drains) so webhook redelivery and restarts don't
double-send a turn.

## 5. The render state machine

The render pipeline ([`render/stream.rs`](../src/render/stream.rs)) turns session
entries into Telegram messages while surviving out-of-order delivery,
redelivery, and restarts. Entry points: `on_message_added`, `on_message_updated`,
`finalize_session`.

**Per-entry serialization.** All events for an entry run under a per-`(session,
entry)` async mutex (`entry_lock`), so an `added`, an `updated`, and a finalize
can't interleave. The lock serializes but does **not** order — so:

- **Revision freshness** (`revision_is_fresh`): an incoming revision must be
  `>= last applied`, else it's dropped as stale. (`message-added` carries
  revision 0, fresh only for a brand-new entry; `on_message_added` prefers an
  existing in-memory session so a `message-updated` that raced ahead isn't
  clobbered by the rev-0 add.)
- **Finalize reconciliation**: once an entry is finalized, later updates are
  ignored unless they carry a **strictly higher** revision, in which case
  `reconcile_finalized_update` edits the posted bubble. A finalize learned from
  durable state after a restart is recorded as `u64::MAX` ("finalized, revision
  unknown") so no late event reconciles it.

**Per-chat message ordering.** New bubbles must post in transcript order even
when entries materialize concurrently. Each entry gets an `order_key` (earliest
append timestamp, min-merged and persisted in KV). `send_in_order` registers an
`(order_key, entry_id, chunk)` slot and waits in `await_create_slot` until it is
the earliest slot **and** no earlier entry is still unmaterialized, then takes the
per-chat create lock to actually `sendMessage`. Waiters hold no create lock while
waiting (avoids deadlock against per-entry locks) and are woken by a per-chat
`Notify`. Because DashMap iteration is unordered, finalize sorts entries
explicitly by `(order_key, entry_id)`.

**Render step** (`apply_render`): freshness guard → classify `MessagePhase`
(`Empty`/`ThinkingOnly`/`Answering`) → compute effective verbosity → render
answer/thinking text → dispatch to draft or edit transport → record a
`PendingEntryState` snapshot (so the entry can be finalized even if its live
session was evicted). Transports, throttling, splitting, and the typing indicator
are detailed in [telegram-api.md §5–6](telegram-api.md#5-streaming-transports).

**Verbosity gating** ([`render/verbosity.rs`](../src/render/verbosity.rs)):
answer text excludes thinking and only includes function-call blocks at `high`+;
thinking text is ungated (it streams as a native rich-thinking draft regardless);
function-result entries only render at `debug`.

## 6. Durable state: the KV schema

All durable state lives in the external `state` worker under scope
`telegram-bot` (`STATE_SCOPE`). Keys carry no TTL — the `timeout_ms` on each call
is the RPC timeout, not a key expiry; keys live until explicitly deleted.

| Key | Value | Purpose |
|---|---|---|
| `chat:{chat_id}:session` | session_id | Forward chat → session mapping. |
| `session:{session_id}:chat` | chat_id | Reverse mapping (bindings resolve chat from session). |
| `chat:{chat_id}:fsm` | `idle`\|`awaiting_model` | Per-chat FSM ([§7](#7-the-per-chat-fsm)). |
| `chat:{chat_id}:model` | `{provider, id}` JSON | Selected model. |
| `chat:{chat_id}:verbosity` | verbosity string | Per-chat verbosity override. |
| `chat:{chat_id}:thinking_level` | level string | Per-chat thinking override (absent = inherit). |
| `entry:{sid}:{eid}:msg` | message_id | Which Telegram message an entry maps to. |
| `entry:{sid}:{eid}:chunk:{idx}:msg` | message_id | Continuation-chunk message ids. |
| `entry:{sid}:{eid}:order` | order_key | Append-order key for posting. |
| `entry:{sid}:{eid}:finalized` | bool | Finalized marker (survives restart). |
| `entry:{sid}:{eid}:thinking_msg` | message_id | The separate thinking-bubble message. |
| `approval:{sid}:{fcid}:msg` | message_id | The approval keyboard message. |
| `cb:{token}` | ApprovalCallbackData | Opaque callback token → approval target. |

`set_chat_session` is best-effort transactional: it writes the forward key, then
the reverse, and rolls back the forward key if the reverse write fails. A crash
between the two leaves an inconsistent pair that `clear`/`reset_for_chat` paths
clean up.

## 7. The per-chat FSM

`ChatFsm` has two states: `Idle` and `AwaitingModel`. `/start` (without a
`default_model`) shows the model picker and sets `AwaitingModel`; a message while
`AwaitingModel` is bounced ("Pick a model first"); selecting a model binds it and
returns to `Idle`. `parse()` maps any unknown/missing string to `Idle`, so a
corrupt key fails safe.

## 8. Preference resolution

`effective_verbosity` / `effective_thinking_level` return the per-chat KV
override if present, else the global `WorkerConfig` value — strict precedence,
no merge. Setting a thinking level to "off" deletes the override key (reverting
to global), so "off at chat level" isn't distinguishable from "inherit" via the
override getter alone.

## 9. Trace correlation

Telegram ingress and harness bindings share OpenTelemetry baggage so the console
can group traces by session and turn:

- **`iii.session.id`** — the harness session id, or `pending-{chat_id}` before
  the first send in a chat.
- **`iii.message.id`** — `tg-{update_id}` at ingress; the harness `turn_id` after
  `harness::send` returns and for binding handlers (resolved by
  `message_id_for_binding`: `active_turns[session_id]` turn_id → entry_id →
  session_id).

`harness::send` receives `options.metadata = { session_id, message_id, surface:
"telegram" }` for engine passthrough. Polling stamps baggage per update in the
poller; webhook stamps it in the HTTP handler.

## 10. Concurrency model

`RuntimeState` ([`deps.rs`](../src/deps.rs)) is the single hub of in-memory
state, all built from `DashMap`s, async `Mutex`es, and `Notify`s:

- **Streaming**: `stream_sessions`, `pending_entries`, `finalized_entries`,
  `revisions`, `edit_times`, `draft_times`, `draft_disabled_chats`.
- **Per-entry / per-chat locks**: `entry_locks`, `chat_create_locks`,
  `chat_create_notifies`.
- **Ordering**: `chat_create_order` (BTreeSet of slots), `chat_pending_materialization`,
  `last_created_order`.
- **Steering**: `fifo_queues`.
- **Ingress**: `poll_offset`, `poller_cancel`, `poller_handle`, and
  `webhook_trigger` (the retained HTTP-route handle — see
  [configuration.md §4](configuration.md#4-the-updates-adapter-lifecycle)).
- **Typing**: `typing_tasks`, `typing_suppressed`, `typing_output_seen`,
  `typing_generation`.
- **Tracing**: `active_turns` (session → latest turn_id).

`reset_for_chat` (called by `/start`) drops all per-chat sequencing/streaming
state and, for the old session, its stream/pending/finalized/entry-lock/revision
entries plus typing and `active_turns` — and bumps the typing generation so
stale refresh ticks can't fire.

## 11. Idempotency and sharp edges

- **Redelivery / restart safety** comes from persisting entry message ids, chunk
  ids, order keys, and finalized flags in KV; in-memory `finalized_entries` plus
  the KV finalized flag gate re-rendering. `harness::send` dedupes via
  `idempotency_key`.
- **Approval token collisions** are possible (32-bit hash of `function_call_id`)
  across concurrent holds in one chat; resolution is idempotent so a stale token
  is a no-op rather than a wrong-approval.
- **Best-effort bindings**: if a sibling worker is absent at boot, its trigger
  binding only warns — that binding silently never fires. The webhook ingress
  route, by contrast, is engine-native and not subject to sibling availability.
- **No key TTLs**: KV grows until explicitly cleared; long-lived chats accumulate
  per-entry keys across sessions (cleared on `/start` for the prior session).
