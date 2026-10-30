# queue

Durable function queues for iii. This worker hosts `queue::on-message`, the
function its topic subscribers observe, and the queue/DLQ service functions
that replace the legacy built-in queue worker.

## Install

```bash
iii call compose::add worker=queue
```

`iii call compose::add` resolves the worker and its dependencies, writes
exact declarations to `worker-compose.yaml`, and reconciles the Compose project.

## Subscribing: `queue::on-message`

A subscriber of a topic is an observer of `queue::on-message`. The function is
private (only this worker calls it) and its handler returns the message it is
called with. The worker calls it once per message for each subscriber, and
calls the subscriber's target through its observer: the target's result acks
the message, an error retries it with backoff and then moves it to the topic's
DLQ.

| | |
|---|---|
| Args | `{ topic, max_retries?, backoff_ms?, queue_config?, group? }` (below) |
| Delivery | custom: each message goes to the one subscriber it was queued for |
| Event (`input` and `output`) | `QueueMessage`: the data published to the topic, as published (any JSON value) |
| Target receives | the published data |
| The queue gets back | the target's result: a result acks the message, an error retries it and then dead-letters it; a target that has not answered within the engine's `observers.activation_timeout_ms` fails the delivery with `TIMEOUT` (see **Timeouts** below) |
| Filters | read the message as `input`; a message an observer's filters reject is acked without calling its target |
| Rejections | `INVALID_ARGS` (no topic, a blank topic, an unknown field), `OBSERVER_ID_CONFLICT` (an observer id already subscribed) |

Args:

| Field | Required | Default | Description |
|---|---|---|---|
| `topic` | yes | - | Topic to consume. `queue` is accepted as an alias. |
| `max_retries` | no | `3` | Failed deliveries before the message moves to the DLQ. `maxRetries` is accepted as an alias. |
| `backoff_ms` | no | `1000` | Base retry delay in milliseconds; retries back off exponentially. `backoffDelayMs` is accepted as an alias. |
| `queue_config` | no | - | Per-subscriber tuning ([Subscriber `queue_config`](#subscriber-queue_config)). |
| `group` | no | - | Consumer group: the observers of the topic with the same group share one durable queue, so each message is handled once by the group, whatever their target, namespace or worker. |

```typescript
iii.registerFunction('orders::process', async (order) => {
  await fulfil(order)            // returning acks the message
  return { ok: true }            // throwing retries it, then moves it to the DLQ
})

// one subscriber
const observer = iii.observe(
  { function_id: 'queue::on-message', args: { topic: 'orders.created', max_retries: 3, backoff_ms: 1000 } },
  'orders::process',
  { id: 'orders-process' },      // a stable id keeps the subscriber's queue across restarts
)
await observer.ready()

// one subscriber with filters: other messages are acked unseen
iii.observe(
  {
    function_id: 'queue::on-message',
    args: { topic: 'orders.created' },
    filters: { 'input.priority': { $eq: 'high' } },
  },
  'orders::expedite',
)

// instances in one consumer group: each message is handled once
iii.observe(
  { function_id: 'queue::on-message', args: { topic: 'orders.created', group: 'billing' } },
  'billing::charge',
)
```

```rust
iii.observe_with(
    Source::function("queue::on-message").args(json!({ "topic": "orders.created" })),
    "orders::process",
    ObserverOptions::default().id("orders-process"),
);
```

- **Fan-out.** Every subscriber of a topic receives every message, with the
  same payload, except that the members of a group share one queue.
- **Durable identity.** A subscriber outside a group has the durable queue
  `<topic>::observer:<observer id>` (builtin) or
  `iii.<topic>.observer:<observer id>.queue` (RabbitMQ); a group has
  `@<group>`. The SDK keeps an observer's id across reconnects, and the `id`
  observe option pins it across restarts of the observing worker: the same id
  finds the same queue and its backlog (with a file-backed store or RabbitMQ).
  A second live observer under the same id is refused with
  `OBSERVER_ID_CONFLICT`.
- **Lifecycle.** `dispose()`, or the observing worker gone past the engine's
  grace period, stops the subscriber's consumer; its queue and backlog stay
  for a later observer with the same id. While the observer is suspended (its
  worker briefly disconnected, or its target unregistered), the call through
  it fails with `OBSERVER_NOT_ACTIVE`: the message stays queued and is
  retried.
- **Timeouts.** The worker calls each target through its observer, and the
  engine bounds every call through an observer by
  `observers.activation_timeout_ms` (30 s by default). A target that has not
  answered by then fails the delivery with `TIMEOUT`: the message is retried
  after its backoff while that first call keeps running, and moves to the DLQ
  once `max_retries` deliveries have failed, even if the target finishes
  later. For targets that run longer, raise the setting in the engine's
  `config.yaml` (it applies to every observer on the engine; the worker's own
  bound is 30 minutes):

  ```yaml
  observers:
    activation_timeout_ms: 600000   # 10 minutes
  ```
- **Private.** Another worker's call of `queue::on-message` is refused
  (`FORBIDDEN`): messages come from publishes only.

## Functions

| Function id | Input | Output |
|---|---|---|
| `queue::define` | `{ "queue", "config" }` | `{ "queue", "changed" }` |
| `engine::queue::enqueue` | `{ "queue", "function_id", "data", "messageReceiptId", "traceparent"?, "baggage"? }` | `{ "messageReceiptId" }` |
| `iii::durable::publish` | `{ "queue" \| "topic", "data" }` | `null` |
| `iii::queue::redrive` | `{ "queue" \| "topic" }` | `{ "queue", "redriven" }` |
| `iii::queue::redrive_message` | `{ "queue" \| "topic", "message_id" }` | `{ "queue", "message_id", "redriven" }` |
| `iii::queue::discard_message` | `{ "queue" \| "topic", "message_id" }` | `{ "queue", "message_id", "redriven" }` |
| `engine::queue::list_topics` | `{}` | topic list |
| `engine::queue::topic_stats` | `{ "topic" \| "queue" }` | `{ "depth", "consumer_count", "dlq_depth", "config" }` |
| `engine::queue::dlq_topics` | `{}` | DLQ topic list |
| `engine::queue::dlq_messages` | `{ "topic" \| "queue", "offset", "limit" }` | DLQ messages |
| `queue::on-message` | the message (private: only this worker calls it; observe it, see above) | the same message |

### Namespace connections

When the worker runs in a project namespace, the process opens two engine
connections. This keeps project functions isolated while engine queue provider
functions remain available from `default`.

| Worker name | Namespace | Registered functions |
|---|---|---|
| `queue` | project namespace | `queue::define`, publish, redrive, redrive-message, and discard functions |
| `queue-engine` | `default` | `engine::queue::enqueue`, list, stats, and DLQ query functions |

Both names therefore appear in `engine::workers::list`. They belong to one
queue process and stop together.

## Configuration

Configuration is owned by the `configuration` worker under id `queue`.
Seed it once with `--config <file>.yaml`; runtime edits come from the
configuration worker after that.

Named queues live under `queue_configs` and can also be converged at runtime
with `queue::define`. Definitions are durable: the worker recreates their
topology and consumers on restart. A definition succeeds only after its
consumer is ready and the merged configuration has been persisted.

```yaml
queue_configs:
  harness-turn:
    type: fifo
    message_group_field: session_id
    concurrency: 10
    max_retries: 3
    backoff_ms: 1000
```

For FIFO named queues, messages with the same group-field value run in order;
different groups run concurrently up to `concurrency`.
Each job of a named queue calls its function directly, with the queue's
`timeout_ms`; when omitted, it defaults to 1,800,000 milliseconds (30
minutes).
After a restart, a delivery waits until its target function is registered and
does not consume retry budget while the target worker is still booting. The
wait is event-driven: the worker observes the engine's
`engine::on-functions-changed` and `engine::on-workers-changed` with the
internal `queue::on-engine-change` function and re-checks the target once per
change, never on a timer. The same feed drives the
`redeliver_on_engine_restart` restart check.

`run_jobs_in` (read at boot) says which namespace runs a function-queue job:

- `enqueuer` (default): the namespace the job was enqueued in.
- `worker`: the namespace of the queue worker that takes the job, whichever
  namespace enqueued it. A job whose function is not registered there goes
  back to the broker (unspent) after one engine change or a second, for another
  worker. Use it when one app is scaled out as several namespaces, each with
  its own queue worker. Needs the `rabbitmq` adapter and
  `engine::functions::info`.

`poll_interval_ms` is deprecated and ignored (still accepted so existing
definitions validate): consumers wake on enqueue, and a backed-off retry is
delivered at exactly its due time.

`adapter.name` selects the transport: `builtin` (default), `redis`, or
`rabbitmq`. Changing the adapter config hot-swaps the transport and
restarts every consumer — the new adapter is built first, so a bad config
leaves the previous adapter serving. Pending in-memory jobs are lost on
swap/restart; file-backed jobs survive. An unreachable `redis`/`rabbitmq`
target at boot fails the boot itself (no fallback to `builtin`).

### Adapters

#### `builtin` (default)

In-process, single-worker transport. Full fan-out (every subscriber on a
topic receives every published message), retries, DLQ, redrive/discard,
and both `fifo` and `concurrent` subscriber modes. The legacy aliases
`in_memory` and `file_based` are also accepted as `adapter.name` and both
resolve to this transport. Consumers do not poll the store: an idle consumer
parks until an enqueue, retry, requeue, or redrive on its queue wakes it, or
until the earliest backed-off retry reaches its due time.

```yaml
adapter:
  name: builtin
  config:
    store_method: file_based
    file_path: data/queue
    save_interval_ms: 5000
```

| Field | Default | Description |
|---|---|---|
| `adapter.config.store_method` | `file_based` | `in_memory` or `file_based`. |
| `adapter.config.file_path` | `data/queue` | Directory used by `file_based`. |
| `adapter.config.save_interval_ms` | `5000` | Accepted for parity; this worker persists on mutation rather than on an interval. |

#### `redis`

Pub/sub only — 1:1 port of the engine builtin's `RedisAdapter`, including
its limitations. There is no DLQ, no retries, and no message durability: a
message published with no subscriber connected is lost.
Redis does not currently support named function-queue consumers, so a
persisted `queue_configs` entry fails boot with an explicit unsupported error.

```yaml
adapter:
  name: redis
  config:
    redis_url: redis://localhost:6379
```

| Field | Default | Description |
|---|---|---|
| `adapter.config.redis_url` | `redis://localhost:6379` | Redis connection string. |

Every DLQ operation (`redrive`, `redrive_message`, `discard_message`,
`dlq_count`, `dlq_peek`) returns the error
`RedisAdapter does not support DLQ operations (pub/sub only)`, verbatim
from the engine.

#### `rabbitmq`

Full-featured transport: retries, DLQ, redrive, message priority, and both
standard and FIFO queue modes. On by default (the `rabbitmq` cargo feature
is feature-default-on); building with `--no-default-features` drops it,
and selecting `adapter.name: rabbitmq` in that build fails boot with an
error naming the missing feature.

RabbitMQ's `x-max-priority` queue argument is immutable. An existing named
queue cannot change `max_priority` in place; delete and recreate its broker
topology first.

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

| Field | Default | Description |
|---|---|---|
| `adapter.config.amqp_url` | `amqp://localhost:5672` | AMQP connection string. |
| `adapter.config.max_attempts` | `3` | Delivery attempts before a message moves to DLQ. The retry budget is stamped on each message at publish time from this adapter-level value, so per-subscriber `queue_config.maxRetries` does not override it on this transport (it applies to the builtin adapter). |
| `adapter.config.prefetch_count` | `10` | Consumer prefetch (QoS), used in `standard` queue mode. |
| `adapter.config.queue_mode` | `standard` | `standard` or `fifo`. Unrecognized values fall back to `standard`. |
| `adapter.config.priority_field` | none | JSON field read from each published message's payload to set the AMQP message priority. Only applies to subscribers whose `queue_config.maxPriority` declares the queue as a priority queue (`x-max-priority`). |

Per-subscriber queue tuning uses the subscriber's `queue_config` arg below;
`maxPriority` is RabbitMQ-only and ignored by the other adapters.

#### `memory` (dev/test only)

An in-process, test-only transport (`adapters::memory::MemoryAdapter`)
used by this worker's own hot-swap tests. It is gated behind the
`test-adapters` cargo feature, off in normal builds, and not a supported
`adapter.name` value for real deployments.

### Subscriber `queue_config`

A `queue::on-message` observer's `queue_config` arg accepts the full builtin
`SubscriberQueueConfig` shape:

| Field | Default | Description |
|---|---|---|
| `type` | `concurrent` | `fifo` (strictly serial) or `concurrent`. |
| `maxRetries` | `3` | Failed deliveries before DLQ, per subscriber (builtin adapter; RabbitMQ uses adapter-level `max_attempts`). |
| `concurrency` | `10` (builtin) | In-flight invocations for `concurrent` mode. `0` pauses consumption. Ignored by `fifo` (always serial — this worker has no grouped-fifo partitioning; see Known Gaps). |
| `visibilityTimeout` | — | Accepted for parity. |
| `delaySeconds` | — | Accepted for parity. |
| `backoffType` | — | Accepted for parity. |
| `backoffDelayMs` | `1000` | Base retry delay; exponential backoff. |
| `maxPriority` | none | RabbitMQ-only: declares the subscriber's queue as an AMQP priority queue with this many levels (1-255). |

## Engine Compatibility

Current engines do not load the legacy built-in by default. Engines with the
`QueueEnqueuer` cut route `CallAction::Enqueue` through this worker's
`engine::queue::enqueue` provider. Where that support is missing, publish with
`iii::durable::publish` to `queue::on-message` subscribers rather than use
named function queues.

`queue::on-message` needs an engine with `observe.v2`. When connecting to an
older engine, remove the legacy built-in from its config first: it serves the
same queues, and the two cannot run together.

On boot, this worker queries `engine::workers::list` and refuses to start if
the legacy built-in is active.

## Parity Vs Builtin

| Behavior | Builtin | This worker |
|---|---|---|
| Subscribers | per-function subscriptions of a topic | `queue::on-message` observers |
| Function ids | 8 legacy public ids | same 8 plus `queue::define` and the engine enqueue provider |
| Retry | `max_retries` + exponential `backoff_ms * 2^(attempts - 1)` | same |
| DLQ | after retries exhausted; redrive/redrive_message/discard | same |
| Restart survival | file-backed store | same (`file_based`) |
| In-memory mode | jobs lost on restart/swap | same |
| Transports | builtin (memory/file), redis (pub/sub, no DLQ), rabbitmq (full: retry/DLQ/priority/fifo) | same as builtin |
| bridge adapter | engine-internal (`CallAction::Enqueue`) | replaced by `engine::queue::enqueue` |
| Enqueue failure when worker offline | n/a, in-process | invocation fails explicitly through the engine provider route |
| Latency benchmark | in-process baseline required | final budget is a pre-deprecation gate (tracked in the migration master plan) |

The `CallAction::Enqueue` path requires an engine version that routes named
enqueue actions through the registered `engine::queue::enqueue` provider.

## Known Gaps / Parity Notes

- **`bridge` adapter — not ported.** It was engine-internal plumbing for
  `CallAction::Enqueue`; the registered `engine::queue::enqueue` provider
  serves that path.
- **Subscriber FIFO is global.** Grouped FIFO is implemented for named
  function queues created by `queue::define`; a topic subscriber's `fifo` mode
  is strictly serial.
- **`resolve_dlq_name` bare-topic behavior ported verbatim, bug and all.**
  DLQ operations (`redrive`, `dlq_count`, `topic_stats`, `dlq_peek`)
  address a bare topic name and, on the builtin adapter, aggregate across
  every subscriber's internal queue on that topic — matching the engine's
  behavior (and its naming quirk) exactly, rather than fixing it here.
  On the rabbitmq transport this makes DLQ browse/redrive against a
  subscriber topic 404 (retries write to the per-subscriber DLQ), and the
  AMQP channel closure stops every consumer on the worker. Upstream engine
  fix tracked as MOT-3904; this worker follows once the engine lands it.
- **`dlq_topics` filters to `dlq_count > 0`.** The engine's equivalent also
  iterates every known topic but returns all of them regardless of DLQ
  depth; this worker only returns topics that currently have dead-lettered
  messages. Documented divergence, not a bug to reconcile.
- **Fifo retry via `nack` can be overtaken by newer arrivals.** A failed
  fifo message is re-queued via `nack` rather than blocking the consumer
  in-place (the engine's `FifoWorker` blocks); a message enqueued after
  the failure can be delivered before the retried one catches up.
