# queue

Durable function queues for iii. This worker registers the
`durable:subscriber` trigger type and the queue/DLQ service functions that
replace the built-in `iii-queue` worker.

## Install

```bash
iii worker add queue
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next
`iii start`.

## Trigger Type

Bind a function to `durable:subscriber` to consume a topic/queue:

| Field | Required | Default | Description |
|---|---|---|---|
| `queue` | yes | - | Topic/queue name to consume. `topic` is accepted as a compatibility alias. |
| `max_retries` | no | `3` | Maximum failed deliveries before the message moves to DLQ. |
| `backoff_ms` | no | `1000` | Base retry delay in milliseconds. Retries use exponential backoff. |
| `condition_function_id` | no | - | Function invoked first. Only explicit `false` skips the handler. |

The worker also accepts the built-in subscriber `queue_config` shape for
compatibility, including `maxRetries` and `backoffDelayMs`.

## Functions

| Function id | Input | Output |
|---|---|---|
| `iii::durable::publish` | `{ "queue" \| "topic", "data" }` | `null` |
| `engine::queue::ensure` | `{ "function_id", "config" }` | `{ "function_id", "queue", "changed" }` |
| `engine::queue::enqueue` | `{ "queue", "function_id", "data", "messageReceiptId" }` | `null` |
| `iii::queue::redrive` | `{ "queue" \| "topic" }` | `{ "queue", "redriven" }` |
| `iii::queue::redrive_message` | `{ "queue" \| "topic", "message_id" }` | `{ "queue", "message_id", "redriven" }` |
| `iii::queue::discard_message` | `{ "queue" \| "topic", "message_id" }` | `{ "queue", "message_id", "redriven" }` |
| `engine::queue::list_topics` | `{}` | topic list |
| `engine::queue::topic_stats` | `{ "topic" \| "queue" }` | `{ "depth", "consumer_count", "dlq_depth", "config", "function_id", "healthy" }` |
| `engine::queue::dlq_topics` | `{}` | DLQ topic list |
| `engine::queue::dlq_messages` | `{ "topic" \| "queue", "offset", "limit" }` | DLQ messages |

## Configuration

Configuration is owned by the `configuration` worker under id `queue`.
Seed it once with `--config <file>.yaml`; runtime edits come from the
configuration worker after that.

`adapter.name` selects the transport: `builtin` (default), `redis`, or
`rabbitmq`. Changing the adapter config hot-swaps the transport and
restarts every consumer — the new adapter is built first, so a bad config
leaves the previous adapter serving. Pending in-memory jobs are lost on
swap/restart; file-backed jobs survive. An unreachable `redis`/`rabbitmq`
target at boot fails the boot itself (no fallback to `builtin`).

Standalone function queues are keyed by the registered function ID. Provision
one with `engine::queue::ensure`; the standalone enqueue provider requires
`queue == function_id`. The public APIs show the original function ID while
the adapters derive a hidden stable broker name. A configured function queue
is ready only when its consumer reports `healthy: true`.

### Adapters

#### `builtin` (default)

In-process, single-worker transport. Full fan-out (every subscriber on a
topic receives every published message), retries, DLQ, redrive/discard,
and both `fifo` and `concurrent` subscriber modes. The legacy aliases
`in_memory` and `file_based` are also accepted as `adapter.name` and both
resolve to this transport.

```yaml
adapter:
  name: builtin
  config:
    store_method: file_based
    file_path: ./data/queue
    save_interval_ms: 5000
```

| Field | Default | Description |
|---|---|---|
| `adapter.config.store_method` | `file_based` | `in_memory` or `file_based`. Use `in_memory` explicitly for development. |
| `adapter.config.file_path` | `./data/queue` | Directory used by `file_based`; relative paths are resolved from the worker's startup directory. |
| `adapter.config.save_interval_ms` | `5000` | Accepted for parity; this worker persists on mutation rather than on an interval. |

#### `redis`

Pub/sub only — 1:1 port of the engine builtin's `RedisAdapter`, including
its limitations. There is no DLQ, no retries, and no message durability: a
message published with no subscriber connected is lost.

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

### Named function queue configuration

`engine::queue::ensure` accepts the engine-compatible shape below. `type` is
`standard` or `fifo`; FIFO requires `message_group_field`. RabbitMQ uses
`max_priority` and `priority_field`; builtin preserves the fields but ignores
broker priority. `poll_interval_ms` applies to polling adapters.

| Field | Default | Description |
|---|---|---|
| `type` | `standard` | Concurrent standard delivery or serial FIFO delivery. |
| `concurrency` | `10` | Consumer concurrency; FIFO uses an effective prefetch of one. |
| `max_retries` | `3` | Attempts before DLQ. |
| `backoff_ms` | `1000` | Exponential retry base delay. |
| `poll_interval_ms` | `100` | Poll interval for builtin/file-backed delivery. |
| `message_group_field` | — | Required payload field for FIFO. |
| `max_priority` | — | RabbitMQ priority levels, 1-255. |
| `priority_field` | — | Payload integer field used for RabbitMQ priority. |

Per-subscriber queue tuning uses the trigger's `queue_config` below;
`maxPriority` is RabbitMQ-only and ignored by the other adapters.

#### `memory` (dev/test only)

An in-process, test-only transport (`adapters::memory::MemoryAdapter`)
used by this worker's own hot-swap tests. It is gated behind the
`test-adapters` cargo feature, off in normal builds, and not a supported
`adapter.name` value for real deployments.

### Trigger `queue_config`

The `durable:subscriber` trigger's `queue_config` accepts the full builtin
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

## Requires Removing The Built-In `iii-queue` Worker

The built-in `iii-queue` worker also owns `durable:subscriber`. Two owners of
the same trigger type on one engine collide, so this worker requires
`iii-queue` to be absent from the engine config.

On boot, this worker queries `engine::workers::list` and refuses to start if
`iii-queue` is active.

## Parity Vs Builtin

| Behavior | Builtin | This worker |
|---|---|---|
| Trigger type | `durable:subscriber` | same |
| Function ids | 8 public ids listed above | same, verbatim |
| Retry | `max_retries` + exponential `backoff_ms * 2^(attempts - 1)` | same |
| DLQ | after retries exhausted; redrive/redrive_message/discard | same |
| Restart survival | file-backed store | same (`file_based`) |
| In-memory mode | jobs lost on restart/swap | same |
| Transports | builtin (memory/file), redis (pub/sub, no DLQ), rabbitmq (full: retry/DLQ/priority/fifo) | same as builtin |
| function-bound enqueue | engine-internal (`TriggerAction::Enqueue`) | workers call `engine::queue::enqueue` directly |
| Enqueue failure when worker offline | n/a, in-process | invocation fails explicitly with `enqueue_error` |
| Latency benchmark | legacy engine baseline | standalone-worker benchmark is tracked separately |

Queue ownership and function-bound dispatch are implemented here. Harness and
other migrated workers call `engine::queue::enqueue` directly, so this
migration does not add or modify engine queue code.

## Known Gaps / Parity Notes

- **`bridge` adapter — not ported.** Migrated workers call the registered
  standalone enqueue provider directly.
- **No grouped-fifo.** The engine's `GroupedFifoWorker` partitions `fifo`
  delivery by `job.group_id` when `concurrency > 1`, running each group
  serially but different groups concurrently. This worker's `fifo` mode is
  strictly serial regardless of `concurrency` — there is no group
  partitioning.
- **`resolve_dlq_name` bare-topic behavior ported verbatim, bug and all.**
  DLQ operations (`redrive`, `dlq_count`, `topic_stats`, `dlq_peek`)
  address a bare topic name and, on the builtin adapter, aggregate across
  every subscriber's internal queue on that topic — matching the engine's
  behavior (and its naming quirk) exactly, rather than fixing it here.
  On the rabbitmq transport this makes DLQ browse/redrive against a
  subscriber topic 404 (retries write to the per-function DLQ), and the
  AMQP channel closure stops every consumer on the worker. Upstream engine
  fix tracked as MOT-3904; this worker follows once the engine lands it.
- **`dlq_topics` filters to `dlq_count > 0`.** The engine's equivalent also
  iterates every known topic but returns all of them regardless of DLQ
  depth; this worker only returns topics that currently have dead-lettered
  messages. Documented divergence, not a bug to reconcile.
- **Fifo retry via `nack` can be overtaken by newer arrivals.** A failed
  fifo message is re-queued via `nack` rather than blocking the poller
  in-place (the engine's `FifoWorker` blocks); a message enqueued after
  the failure can be delivered before the retried one catches up.
