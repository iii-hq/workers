---
name: pubsub
tags: pubsub, publish, observe, events, redis
description: >-
  DEPRECATED (will be removed in an upcoming release; migration guide
  https://github.com/iii-hq/iii/blob/6901a589dd39d9c34f9d5b6bd8bc82170ee21548/docs/next/upgrading/migrate-from-streams.mdx). Fire-and-forget topic pub/sub: broadcast an event with `publish` and every
  observer of `publish` filtered on that topic receives it. Use for real-time
  notifications where missed events are acceptable.
---

# pubsub

> **Deprecated:** pubsub is deprecated (pubsub) and will be removed in an upcoming release. Behavior is unchanged for now. Migration guide: https://github.com/iii-hq/iii/blob/6901a589dd39d9c34f9d5b6bd8bc82170ee21548/docs/next/upgrading/migrate-from-streams.mdx
>
> Applies to `publish` and the `pubsub` configuration entry. Do not start new work on it: observe a worker-owned event function for fire-and-forget fan-out, or use the `queue` worker (`iii::durable::publish` observed through `queue::on-message`) when every event must be processed. The worker logs rate-limited deprecation warnings (target `iii::deprecation`); topic `stream.events`, used by the engine's iii-stream bridge, is exempt.

The `pubsub` worker is topic-based publish/subscribe messaging. Publish an event to a named topic with the `publish` function, and every observer of `publish` whose filter selects that topic receives `{ topic, data }` — no persistence, no retries. It is fire-and-forget broadcast: observers receive each event as it arrives, and a target that is offline simply misses it.

Install it with `iii call compose::add worker=pubsub`. It replaces the former in-engine pub/sub service: the function id (`publish`) is unchanged, so existing publishers keep working — only the owner moved from an in-process builtin to this standalone worker.

The worker exposes one function, `publish`, registered with the bare function id `"publish"` (no namespace prefix). Any worker may call it, and subscribers observe it. Two adapters: `local` (default; events stay on this engine; no external dependency) and `redis` (`redis_url: ${REDIS_URL:redis://localhost:6379}`; uses Redis Pub/Sub so events also reach the observers on other engines). The adapter is set in the `pubsub` configuration entry and hot-swaps at runtime.

## When to Use

- Real-time notifications consumers may miss without consequence — UI live updates, ephemeral signals, telemetry mirroring.
- Loose coupling where an event source should not know about its consumers.
- Cross-engine fan-out of ephemeral events (configure the `redis` adapter).

## Boundaries

- No persistence, ordering, or retries — a target that is down misses events permanently. Use the `queue` worker (`queue::on-message`) when every consumer must process every event.
- Topics are plain strings; select them with filter operators (`$eq`, `$in`, …) on `input.topic`. There are no hierarchical topic patterns.
- An empty topic is rejected: `publish` returns `topic_not_set` and the call reaches no observer.
- For key/value or stream change reactivity, observe `state::on-changed` or the `stream::on-*` functions instead — `pubsub` carries discrete events, not state.

## Functions

- `publish` — broadcast `{ topic, data }`; every observer of `publish` whose filters match the call receives it. Empty topic returns `topic_not_set`. Returns `null`.

## Observing a topic

Observe `publish` when a function should run every time something publishes to a topic. It takes no args: select the topic with a filter on `input.topic`, and use `deliver: 'input'` so the target receives `{ topic, data }`. Several observers of the same topic each get an independent copy (true fan-out).

Reach for it when:

- A domain event from one part of the system should kick off side effects elsewhere without the publisher knowing the consumers.
- You want cross-engine fan-out via the `redis` adapter.

If targets must reliably process every event with retries and dead-letter handling, observe the `queue` worker's `queue::on-message` instead — `publish` is fire-and-forget.

### How to observe

1. Register a handler: `iii.registerFunction('notifications::on-order-shipped', handler)`.
2. Observe `publish` with it:

```typescript
iii.observe(
  {
    function_id: 'publish',
    filters: { 'input.topic': { $eq: 'orders.shipped' } },  // exact topic; $in for several
    deliver: 'input',
  },
  'notifications::on-order-shipped',
)
```

The handler receives `{ topic, data }`, so one handler can serve several topics. Its return value is ignored and there is no retry. Pass `{ metadata }` as the third argument of `iii.observe` to hand the handler a sidecar with every event.

For the published payload shape, call `iii get function info` on `publish` or the handler function id.
