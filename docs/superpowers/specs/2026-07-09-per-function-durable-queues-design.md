# Per-function durable queues design

**Date:** 2026-07-09
**Status:** approved; implementation in progress
**Ticket:** MOT-3944
**Repository:** `iii-hq/workers` queue/harness. The `iii` engine is an
existing compatibility boundary for the migration, not a target of this
change.
**Scope decisions (locked):** one logical queue per registered function id;
function id is the canonical queue identity; existing `TriggerAction::Enqueue {
queue }` wire shape stays compatible; harness lanes become three internal
function ids backed by one handler; standalone queue configuration defaults to
restart-safe file storage; no new engine-side queue behavior is introduced.

## Context

PR #464 currently provisions three named queues (`harness-turn`,
`harness-subagent`, and `harness-reactive`) but sends every job to the same
registered function, `harness::turn`. The standalone queue worker stores an
arbitrary `function_id` in each job, so one queue can mix several functions and
share one concurrency budget. That is lane-based scheduling, not a dedicated
queue per function.

The queue implementation is moving out of the engine and into this workers
repository. The existing `TriggerAction::Enqueue` wire shape remains for
legacy callers, while migrated workers call the registered
`engine::queue::enqueue` provider directly. No engine source change or engine
release is part of this work.

## 1. Invariants

1. Every provisioned function queue belongs to exactly one registered iii
   function id.
2. A function id has at most one active queue configuration.
3. The logical queue identity is the function id. Broker-specific physical
   names are derived deterministically and remain an implementation detail.
4. A queued job may invoke only the function that owns its queue. Mismatched
   `queue` and `function_id` values fail before persistence.
5. Enqueue success is returned only after the standalone worker has persisted
   the job to its selected adapter.
6. Queue readiness means a live consumer exists, not merely that configuration
   exists.
7. Queue configuration changes reconcile the existing consumer instead of
   making later deployments fail.

The existing `TriggerAction::Enqueue { queue }` protocol remains on the wire so
current SDKs continue to deserialize it, but migrated workers do not use it.
They call `engine::queue::enqueue` directly with `queue`, `function_id`,
`data`, and `messageReceiptId`; the workers-side provider validates
`queue == function_id`. The engine is treated as an external compatibility
surface and is not modified here.

## 2. Migration boundary

The standalone worker owns queue configuration, persistence, consumers, and
function dispatch. Harness reaches it through the registered
`engine::queue::*` functions rather than the legacy enqueue wire action. This
workers change must not add a second engine implementation or depend on an
engine-side fallback commit.

## 3. Standalone queue control plane

### 3.1 Provisioning contract

`engine::queue::ensure` changes from queue-name provisioning to function
provisioning:

```json
{
  "function_id": "harness::turn::root",
  "config": {
    "type": "standard",
    "concurrency": 10,
    "max_retries": 3,
    "backoff_ms": 1000
  }
}
```

The result returns the function id, deterministic logical queue id, and whether
the runtime changed:

```json
{
  "function_id": "harness::turn::root",
  "queue": "harness::turn::root",
  "changed": true
}
```

The persisted `queue_configs` map is keyed by function id. Repeating the same
configuration is a no-op. Supplying different configuration updates the entry
and restarts only that function's consumer after the replacement consumer is
ready. If replacement fails, the previous config and consumer remain active.

### 3.2 Enqueue contract

`engine::queue::enqueue` retains the engine-facing fields for wire
compatibility, but validates `queue == function_id`. It rejects empty ids,
unknown/unprovisioned functions, mismatches, and empty receipt ids before
calling the adapter.

The runtime consumer is bound to its function id at creation. Messages no
longer select an arbitrary function dynamically. The adapter envelope may keep
the function id for diagnostics and compatibility, but the runtime verifies it
before invocation and dead-letters malformed legacy jobs rather than invoking
another function.

### 3.3 Physical naming

Adapters derive physical names from the logical function id using one shared,
stable naming helper. The helper emits a readable sanitized prefix plus a
stable digest, keeping RabbitMQ names below its length limit while avoiding
collisions between function ids that sanitize to the same text. Console and
operator APIs display the original function id, never the encoded broker name.

### 3.4 Health and self-healing

An active queue record contains its config, function id, and consumer task.
Reconciliation treats a finished task as unhealthy even when config is
unchanged and starts a replacement. The consumer removes or marks its active
record unhealthy when its adapter receiver closes.

`engine::queue::list_topics` and `topic_stats` report function-queue health with
at least:

```json
{
  "name": "harness::turn::root",
  "function_id": "harness::turn::root",
  "consumer_count": 10,
  "healthy": true
}
```

Harness boot requires all three entries to report `healthy: true`.
Configuration presence alone is not readiness.

## 4. Queue feature parity and durability

The standalone `FunctionQueueConfig` becomes the full engine shape:

- `type`: `standard` or `fifo`;
- `concurrency`;
- `max_retries`;
- `backoff_ms`;
- `poll_interval_ms`;
- `message_group_field` for FIFO;
- `max_priority` and `priority_field` for RabbitMQ.

Validation matches the engine: concurrency is at least one; FIFO requires a
non-empty group field; priority bounds are enforced; standard queues ignore
FIFO-only fields. Enqueue extracts group and priority data before persistence
and adapters preserve them through retry and DLQ paths.

The registry manifest defaults the builtin adapter to `file_based` storage at
`./data/queue`, so a normal `iii worker add harness` installation survives queue
worker restarts. Operators may explicitly select `in_memory` for development or
Redis with its documented non-durable semantics. Documentation must not call
those modes restart-safe.

## 5. Harness execution functions

Harness registers three internal functions, all using the existing typed
`TurnStepPayload`/`TurnStepResult` contract and delegating to `turn::handle`:

| Lane | Function and logical queue id |
|---|---|
| Root | `harness::turn::root` |
| Sub-agent | `harness::turn::subagent` |
| Reactive | `harness::turn::reactive` |

`TurnLane` maps to the function id rather than a separate queue name. The
persisted lane remains frozen on the turn record, so every continuation and
resume selects the same endpoint. `harness::turn` remains registered as a
non-enqueued compatibility alias during the pre-release transition but new
jobs never target it.

At boot, harness calls `engine::queue::ensure` once per lane function and waits
for healthy status. `build_enqueue_request` calls `engine::queue::enqueue`
directly, placing the selected lane function ID in both `queue` and
`function_id` and adding a fresh `messageReceiptId`.

## 6. Existing consumers

The repository-wide `TriggerAction::Enqueue` audit includes:

- `workflow::tick`, currently using `default`;
- workflow completion notifications, currently accepting an arbitrary queue;
- worktree land steps, currently accepting an arbitrary queue.

They are not silently converted in PR #464. Before those workers switch from
the built-in queue module to the standalone provider, each must provision and
target its own function id. Dynamic workflow notifications use the notified
function id as their queue identity. Documentation and examples stop teaching
shared `default` queues for the standalone provider.

## 7. Failure handling

- If direct provider enqueue fails, harness marks the persisted turn failed
  using its existing `enqueue_step` failure path.
- If a function invocation fails, the adapter retries with the configured
  exponential backoff and dead-letters after the configured attempt budget.
- Ack or nack failures remain visible as errors and leave the delivery eligible
  for transport redelivery; they are never reported as a successful terminal
  delivery.
- A consumer that exits is unhealthy immediately and is recreated by the next
  reconciliation/readiness pass.
- A failed config replacement does not discard the prior healthy consumer.

## 8. Verification and delivery

### Workers repository

1. Unit-test the function-id provisioning API, mismatch rejection, config
   updates, physical naming, finished-consumer restart, and health reporting.
2. Adapter-test standard, FIFO, priority, retry, DLQ, and trace propagation.
3. Harness-test the three function registrations, lane routing, legacy-record
   inference, enqueue-failure terminal state, and healthy boot barrier.
4. Run queue and harness formatting, clippy, unit tests, compatibility-path
   tests, interface boot smoke, and one live harness turn through the
   standalone provider.

The current uncommitted queue test fixes (`QueueConfig::default()` completion,
the required queue `type`, and handling the publish result) are included in the
workers implementation so PR CI is green.

## 9. Non-goals

- Changing the `iii` engine or SDK wire protocol in this change.
- Changing the public `TriggerAction` wire representation across SDKs.
- Automatically provisioning queues for every registered function regardless
  of whether it is ever enqueued.
- Adding cancellation by `messageReceiptId`.
- Migrating unrelated workers to the standalone provider in PR #464.

## Delivery

One workers change enforces per-function queues, splits the harness execution
endpoints, and updates durability, health, parity, tests, and documentation.
It calls the existing standalone provider surface without requiring an engine
commit or release.
