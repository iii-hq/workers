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
| `iii::queue::redrive` | `{ "queue" \| "topic" }` | `{ "queue", "redriven" }` |
| `iii::queue::redrive_message` | `{ "queue" \| "topic", "message_id" }` | `{ "queue", "message_id", "redriven" }` |
| `iii::queue::discard_message` | `{ "queue" \| "topic", "message_id" }` | `{ "queue", "message_id", "redriven" }` |
| `engine::queue::list_topics` | `{}` | topic list |
| `engine::queue::topic_stats` | `{ "topic" \| "queue" }` | `{ "depth", "consumer_count", "dlq_depth", "config" }` |
| `engine::queue::dlq_topics` | `{}` | DLQ topic list |
| `engine::queue::dlq_messages` | `{ "topic" \| "queue", "offset", "limit" }` | DLQ messages |

## Configuration

Configuration is owned by the `configuration` worker under id `queue`.
Seed it once with `--config <file>.yaml`; runtime edits come from the
configuration worker after that.

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
| `adapter.name` | `builtin` | In-process queue transport. `redis` and `rabbitmq` are follow-up transports. |
| `adapter.config.store_method` | `in_memory` | `in_memory` or `file_based`. |
| `adapter.config.file_path` | `queue_store_data` | Directory used by `file_based`. |
| `adapter.config.save_interval_ms` | `5000` | Accepted for parity; this worker persists on mutation. |

Changing the adapter config hot-swaps the transport and restarts every
consumer. Pending in-memory jobs are lost on swap/restart, matching the
built-in adapter's in-process durability profile. File-backed jobs survive.

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
| Transports | builtin/memory/redis/rabbitmq | builtin memory/file; redis+rabbitmq follow-up |
| Enqueue failure when worker offline | n/a, in-process | invocation fails explicitly once the engine remote-enqueue cut lands |

The full engine `TriggerAction::Enqueue` path depends on the separate
`QueueEnqueuer` engine cut tracked in the migration master plan.
