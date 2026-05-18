# hook-fanout

Generic publish-collect primitive on the iii bus
(`hook-fanout::publish_collect`).

## Purpose

`hook-fanout::publish_collect` is the generic shape of every iii hook:
mint an `event_id`, publish a topic envelope via `iii::durable::publish`,
collect every reply that subscribers append to the `agent::hook_reply`
stream (filtered by `group_id == event_id`), apply a merge rule, return
`{ event_id, replies, merged }`. The approval gate uses this indirectly
via the orchestrator's `consultBefore`; operators can build their own
multi-subscriber flows on the same primitive.

The poll loop exits on the first of four conditions:

| Reason | Trigger |
|---|---|
| `expected_replies` | `replies.length >= expected_replies` (when caller supplied a count). |
| `quiescence` | `quiescence_ms` has elapsed since the last new reply. |
| `deadline` | `timeout_ms` reached and at least one reply arrived. |
| `deadline_no_replies` | `timeout_ms` reached with no replies. |

Three merge rules are implemented in
[src/hook-fanout/merge.ts](harness-node/src/hook-fanout/merge.ts):

| Merge rule | Behaviour |
|---|---|
| `first_block_wins` | Returns the first reply with `block: true` (carrying its `reason` and optional `denial` envelope), else `{ block: false }`. This is the rule the approval gate uses. |
| `field_merge` | Starts from the caller's `payload`, deep-merges each reply's fields on top, last writer wins. |
| `pipeline_last_wins` | Like `field_merge` but treats each reply as a full replacement payload. |

## Registered functions

- `hook-fanout::publish_collect` — Publish a topic, collect subscriber replies until timeout, apply `merge_rule`.

## Triggers

None — the worker doesn't subscribe to anything itself.

## State keys

None. Replies are read from the `agent::hook_reply` iii stream (via
`stream::list` polling), not from state.

## Configuration

From the optional `hook_fanout` section of
[config.yaml](harness-node/config.yaml) (defaults from
[src/hook-fanout/publish-collect.ts](harness-node/src/hook-fanout/publish-collect.ts)):

- `default_timeout_ms` (default `10000`) — fallback for callers that omit
  `timeout_ms`.
- `min_timeout_ms` (default `50`) — floor enforced on the effective
  timeout so tests don't accidentally race to zero.
- `poll_interval_ms` (default `25`) — `stream::list` poll cadence.
- `quiescence_ms` (default `200`) — fallback for callers that omit
  `quiescence_ms`.

## Dependencies

From
[src/hook-fanout/iii.worker.yaml](harness-node/src/hook-fanout/iii.worker.yaml):
`iii-stream ^0.11.0`. The worker also calls `iii::durable::publish` and
`stream::list` over the bus.

## Source layout

| File | Purpose |
|---|---|
| [src/hook-fanout/main.ts](harness-node/src/hook-fanout/main.ts) | Binary entry point (`iii-hook-fanout`). |
| [src/hook-fanout/register.ts](harness-node/src/hook-fanout/register.ts) | Wires the worker's single function. |
| [src/hook-fanout/config.ts](harness-node/src/hook-fanout/config.ts) | Loads the `hook_fanout` config section. |
| [src/hook-fanout/types.ts](harness-node/src/hook-fanout/types.ts) | `FUNCTION_ID`, `HOOK_REPLY_STREAM`, `MergeRule`, request/response wire types. |
| [src/hook-fanout/publish-collect.ts](harness-node/src/hook-fanout/publish-collect.ts) | Handler — builds the envelope, runs the poll loop, applies merge rule. |
| [src/hook-fanout/exit.ts](harness-node/src/hook-fanout/exit.ts) | Pure `decideExit` for the poll loop. |
| [src/hook-fanout/merge.ts](harness-node/src/hook-fanout/merge.ts) | Pure implementations of the three merge rules. |
| [src/hook-fanout/iii.worker.yaml](harness-node/src/hook-fanout/iii.worker.yaml) | Worker manifest. |
