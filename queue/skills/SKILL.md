---
type: index
name: queue
tags: queue, durable, retries, dlq, pubsub
description: >-
  Durable topic queues for iii: subscribe functions to topics, publish work
  asynchronously, retry failed deliveries with backoff, and inspect/redrive
  dead-lettered messages.
---

# queue

The `queue` worker provides durable topic delivery for iii. Register a consumer
with a `durable:subscriber` trigger, publish with `iii::durable::publish`, and
the worker persists messages, retries failed deliveries, moves exhausted jobs to
DLQ, and exposes redrive/discard inspection functions.

Install it with `iii worker add queue`. The built-in `iii-queue` worker must be
removed from the engine config before this worker starts because both would own
the same `durable:subscriber` trigger type.

## When to Use

- A topic event must be processed reliably by a subscribed function.
- A failed consumer should retry automatically before the message lands in DLQ.
- You need to inspect, redrive, or discard dead-lettered messages.
- Local or single-instance durable queues are enough, using in-memory storage
  for development or file-backed storage for restart survival.

## Boundaries

- Not fire-and-forget broadcast. Use `pubsub` when missed events are acceptable.
- Three transports ship today: `builtin` (in-process, full DLQ/retry/fifo),
  `redis` (pub/sub only, no DLQ), and `rabbitmq` (full: retry/DLQ/priority/fifo).
  The `bridge` adapter is not ported — it was engine-internal and is superseded
  by the engine's `QueueEnqueuer` cut (MOT-3829).
- Engine `TriggerAction.Enqueue` / named function queues require the separate
  `QueueEnqueuer` engine cut. Until that lands, use `iii::durable::publish` and
  `durable:subscriber` triggers for this standalone worker.
- File-backed mode survives worker restarts. In-memory mode loses pending jobs
  on restart or transport hot-swap. An unreachable `redis`/`rabbitmq` target at
  boot fails the boot (no fallback to `builtin`).
- `fifo` mode is strictly serial — there is no grouped-fifo partitioning by
  `group_id` like the engine's `GroupedFifoWorker`.
- This worker carries work items, not key/value or stream state. Use `state` or
  `stream` for those.

## Functions

- `iii::durable::publish`: publish a JSON payload to a queue/topic. Input accepts
  `{ "queue": "...", "data": ... }` and also accepts `topic` as an alias.
- `iii::queue::redrive`: move all DLQ messages for a queue back to the main
  queue. Input accepts `queue` or `topic`.
- `iii::queue::redrive_message`: move one DLQ message back by `message_id`.
- `iii::queue::discard_message`: purge one DLQ message by `message_id`.
- `engine::queue::list_topics`: list known queue topics.
- `engine::queue::topic_stats`: return `depth`, `consumer_count`, and
  `dlq_depth` for one topic.
- `engine::queue::dlq_topics`: list DLQ topics with message counts.
- `engine::queue::dlq_messages`: browse DLQ messages with `offset` and `limit`.

## Reactive Triggers

Bind a `durable:subscriber` trigger when a function should consume every
message published to a queue/topic with retries and DLQ handling.

```typescript
iii.registerTrigger({
  type: 'durable:subscriber',
  function_id: 'orders::process',
  config: {
    queue: 'orders.created',
    max_retries: 3,
    backoff_ms: 1000,
  },
})
```

Trigger config:

- `queue` (required): topic/queue name to consume. `topic` is accepted as a
  compatibility alias.
- `max_retries` (default `3`): failed attempts before moving to DLQ.
- `backoff_ms` (default `1000`): base retry delay. Retries use the same
  exponential curve as the builtin queue: `backoff_ms * 2^(attempts - 1)`.
- `condition_function_id` (optional): called before the handler; only explicit
  `false` skips the handler.

## Configuration

Use the `configuration` worker entry named `queue`. The default is in-memory:

```yaml
adapter:
  name: builtin
```

For restart survival, use file-backed storage:

```yaml
adapter:
  name: builtin
  config:
    store_method: file_based
    file_path: ./data/queue
    save_interval_ms: 5000
```

For pub/sub-only delivery against a shared Redis (no DLQ, no durability):

```yaml
adapter:
  name: redis
  config:
    redis_url: redis://localhost:6379
```

For full retry/DLQ/priority/fifo delivery against RabbitMQ:

```yaml
adapter:
  name: rabbitmq
  config:
    amqp_url: amqp://localhost:5672
    max_attempts: 3
    prefetch_count: 10
    queue_mode: standard
    priority_field: priority
```

Changing the adapter config hot-swaps the transport and restarts every
consumer — the new adapter is built first, so a bad config leaves the
previous one serving. File-backed jobs survive by construction; in-memory
jobs are lost, matching the builtin adapter's in-process durability profile.
