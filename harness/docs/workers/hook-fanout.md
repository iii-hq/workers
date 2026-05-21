# hook-fanout

Generic publish-collect primitive on the iii bus
(`hook-fanout::publish_collect`). Fully reactive — replies are routed by a
stream trigger, not a poll loop.

## Purpose

`hook-fanout::publish_collect` is the generic shape of every iii hook:
mint an `event_id`, install an in-process `Collector`, publish a topic
envelope via `iii::durable::publish`, and resolve when subscribers append
replies to the `agent::hook_reply` stream (filtered by
`group_id == event_id`). A single stream trigger on `agent::hook_reply` —
`hook-fanout::reply_handler` — dispatches every reply frame into the
right collector through a process-local `Map<event_id, Collector>`. There
is no `stream::list` poll loop.

The collector resolves on the first of four conditions:

| Reason | Trigger |
|---|---|
| `expected_replies` | `replies.length >= expected_replies` (when caller supplied a count). |
| `quiescence` | `quiescence_ms` has elapsed since the last new reply (timer is reset on each reply). |
| `deadline` | `timeout_ms` reached and at least one reply arrived. |
| `deadline_no_replies` | `timeout_ms` reached with no replies. |

The response shape always includes a `publish` field so callers can fail
closed when `iii::durable::publish` itself errored: `{ event_id, replies,
merged, publish: { ok: true } | { ok: false, error? }, publish_failed?:
true }`. The orchestrator's `consultBefore` treats `publish_failed` as a
`gate_unavailable` denial.

Three merge rules are implemented in
[src/hook-fanout/merge.ts](harness-node/src/hook-fanout/merge.ts):

| Merge rule | Behaviour |
|---|---|
| `first_block_wins` | Returns a shallow clone of the first reply with `block: true` — preserving the full envelope (`reason`, `denial`, `status`, `subscriber`, `approval_gate` markers) so the orchestrator can tell `pending` apart from `denied` and verify the gate actually replied. Falls back to `{ block: false }` if no reply blocks. This is the rule the approval gate uses. |
| `field_merge` | Starts from the caller's `payload`, deep-merges each reply's `content` / `details` / `terminate` fields on top, last writer wins. |
| `pipeline_last_wins` | Each reply is treated as a full replacement payload (an array, or `{ messages: [...] }`); the last decoded value wins. |

## Registered functions

- `hook-fanout::publish_collect` — Publish a topic, collect subscriber replies until exit-reason, apply `merge_rule`.
- `hook-fanout::reply_handler` — Internal stream-trigger handler that routes each `agent::hook_reply` frame to its pending collector.

## Triggers

- **Stream trigger** on `agent::hook_reply` → `hook-fanout::reply_handler`.
  This replaces the previous `stream::list` poll loop; replies are
  delivered into the collector synchronously as the engine fans them out.

## State keys

None. Replies are routed through the `agent::hook_reply` iii stream and
held in the process-local pending map until the call resolves.

## Configuration

From the optional `hook_fanout` section of
[config.yaml](harness-node/config.yaml) (defaults from
[src/hook-fanout/publish-collect.ts](harness-node/src/hook-fanout/publish-collect.ts)):

- `default_timeout_ms` (default `10000`) — fallback for callers that omit
  `timeout_ms`.
- `min_timeout_ms` (default `50`) — floor enforced on the effective
  timeout so tests don't accidentally race to zero.
- `quiescence_ms` (default `200`) — fallback for callers that omit
  `quiescence_ms`.

## Dependencies

From
[src/hook-fanout/iii.worker.yaml](harness-node/src/hook-fanout/iii.worker.yaml):
`iii-stream ^0.11.0`. The worker calls `iii::durable::publish` over the
bus and consumes `agent::hook_reply` via the stream trigger.

## Source layout

| File | Purpose |
|---|---|
| [src/hook-fanout/main.ts](harness-node/src/hook-fanout/main.ts) | Binary entry point (`iii-hook-fanout`). |
| [src/hook-fanout/register.ts](harness-node/src/hook-fanout/register.ts) | Wires the public function and the internal stream-trigger handler. |
| [src/hook-fanout/config.ts](harness-node/src/hook-fanout/config.ts) | Loads the `hook_fanout` config section. |
| [src/hook-fanout/types.ts](harness-node/src/hook-fanout/types.ts) | `FUNCTION_ID`, `REPLY_HANDLER_FN_ID`, `HOOK_REPLY_STREAM`, `MergeRule`, request/response wire types. |
| [src/hook-fanout/publish-collect.ts](harness-node/src/hook-fanout/publish-collect.ts) | `execute` (installs the collector, publishes, awaits the exit reason, applies merge_rule, builds the response) + `handleStreamReply` (stream-trigger callback that dispatches replies into pending collectors). |
| [src/hook-fanout/exit.ts](harness-node/src/hook-fanout/exit.ts) | `ExitReason` enum shared with the publish-collect waiter. |
| [src/hook-fanout/merge.ts](harness-node/src/hook-fanout/merge.ts) | Pure implementations of the three merge rules. |
| [src/hook-fanout/iii.worker.yaml](harness-node/src/hook-fanout/iii.worker.yaml) | Worker manifest. |
