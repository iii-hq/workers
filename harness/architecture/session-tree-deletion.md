# Durable session-subtree deletion

This is an operator/Console control-plane operation. It **never resolves the
selected session to the global root**. Its root is exactly `session_id`.

## Wire contract

- `harness::delete-session-tree { session_id: string }` accepts work and returns
  a snapshot after the admission/conflict check. It does not wait for cancellation
  or deletion.
- `harness::delete-session-tree-status { operation_id: string }` returns the
  saved snapshot, or `null` for an unknown operation.
- Trigger `harness::session-tree-deletion` accepts
  `{ session_id?: string, operation_id?: string }`. Both filters are conjunctive.
  The delivered event is the snapshot itself, with no wrapper:

```json
{
  "operation_id": "delete_<sha256 of selected session id>",
  "attempt": 1,
  "session_id": "selected-child",
  "status": "deleting",
  "deleted_session_ids": [],
  "error": "present only on failure"
}
```

`status` is `deleting | completed | failed`. Bind by session id **before** the
command; after acceptance read status once by operation id for race recovery.
Wait for a terminal event, not a poll loop. Repeated pending/completed commands
retain the operation identity and attempt. The required `attempt: u32` starts at
1 and increments **only** on an explicit `failed -> deleting` transition, under
the same operation lock as the runner. Concurrent duplicate retries share that
increment. Recovery, pending repeats and completed repeats never increment it.
A failed command retries with the **same** operation id, retains partial progress,
and gets a new 120-second cancellation deadline. Correlate terminal events by
**both `operation_id` and `attempt`**: a delayed `failed` event from an earlier
attempt must not settle the new request. Emission preserves the snapshot's attempt.
Pre-review stored operations without the field are read as attempt 1; this
storage-only migration does not make the public wire field optional.
A partial cleanup is not reported as success. `deleted_session_ids` reports the
acknowledged deletions; a lost acknowledgement can underreport until retry.

## Runtime and durability

The existing queue worker owns `harness-session-deletion`, FIFO by operation id,
with restart redelivery. Both turn and deletion queues use one provisioning
helper with 20 readiness attempts and 250 ms backoff (5-second RPC timeout).
Only the turn queue retains its legacy-schema fallback. Deletion **requires** a
queue worker accepting `redeliver_on_engine_restart`; an unsupported field fails
startup explicitly rather than silently weakening recovery. Both queues must be
ready before the startup recovery scan and ready event. The definitions match
`queue/src/adapter.rs::FunctionQueueConfig` and `queue/src/runtime.rs::DefineQueueInput`.
`harness::delete-session-tree-run` is internal. Boot
performs one recovery scan of pending operations. No extra runtime process,
HTTP server, or polling task is introduced. A persisted deadline and turn/tool
completion notifications bound the wait; the deadline is recomputed on recovery.

The state worker's private harness namespace owns `harness_deletion`,
`harness_deletion_guard`, and `harness_deletion_dispatch`. Operation plans retain
membership, direct parent, selected-session name, notification acknowledgement,
and partial progress. Admission checks descendants as well as ancestors under
`topology` **before** writing the selected-root guard. If a child already owns a
deletion, an ancestor request fails without tombstoning the parent or siblings.
Recovery revalidates the live tree plus persisted members and claims each guard
by CAS (absent or same owner only). It can remove the previous implementation's
unplanned ancestor reservation when a child already owns its root; planned
ancestor operations are never treated as stale. Command transitions and `run`
share an operation lock; pending/completed read-only repeats do not wait for the
runner. Lock order is operation -> topology, with no nested operation locks.
Tombstones intentionally survive completion and execution failure;
a deleted id is not reusable. Unknown remote tool completion is retained as a
durable witness, not reclassified as cancellation just because its caller timed
out. A subsequent confirmed reply clears its witness and signals the waiter.

Admission for session creation, message enqueue/append, binding registration and
target dispatch checks the selected session **and its durable ancestors**.
These `ensure_live`/ancestry guards perform state and session RPCs on regular
send/spawn/wake/dispatch paths (roughly proportional to lineage depth). This is
an intentional current latency/load cost; this review does not add caching or
speculative optimization that could weaken the deletion boundary.
Enumeration uses `session_tree::collect` / `SessionClient::children`, not the
last turn's spawn checkpoints. Creation metadata is serialized against tree
planning. All members receive cancellation signals before the first busy tool
lock is awaited. Deletion waits for terminal records and the full running-turn
lifetime, including generation outside the record lock. Parked approval holds
can finalize as cancelled; unknown external pending jobs cannot.

Cleanup removes owned bindings, parked messages, turn records, filesystem
grants, context accounting and the member's budget ledger, then delegates
transcript/attachment deletion to `session::delete`. A response must contain a
boolean `deleted`; malformed replies fail without recording that member as
deleted or erasing its turn record. Session absence is verified after either
boolean value, so `deleted: false` can safely acknowledge a replay of an earlier
delete whose response was lost. The approval worker's
existing deletion handler is called and its pending inbox and settings source
are checked afterwards.
A budget shared with an ancestor is not removed: cleanup uses member ids only.
Old queue deliveries find a missing/terminal record and cannot reseed it.

## Surviving parent notification

One consolidated model-visible user message is queued durably to the **direct**
parent outside the subtree after cancellation is confirmed and before member
metadata is removed. It names the selected session and members, states that the
user requested cancellation/deletion, and says not to await results or recreate
the work automatically. A deterministic queue/entry id deduplicates retries.
The ordinary turn drain appends it to history and model context. A terminal
parent gets a new turn with its existing options; a running parent continues
unchanged. An approval-parked parent keeps its pending calls and consumes the
message when its normal continuation resumes. Missing/tombstoned parents are
never created or woken. Cancellation is never supplied as a successful child
result to a legacy wait.

## Safety boundaries and remaining limitations

- As with `SessionLocks` / `TurnCancels`, concurrency guarantees assume one
  harness process owns these sessions. Multiple independently running harness
  replicas need a distributed lifecycle/admission lock; private CAS storage
  alone is not such a lock.
- Writers bypassing harness and directly editing session-manager lineage or
  creating sessions are outside the admission boundary. Route Console subtree
  deletion through the new API, not raw `session::delete`.
- The runtime cannot cancel arbitrary remote function execution. A transport
  timeout, engine interruption, or unknown pending job fails closed, retains
  data and tombstones, and may require operator confirmation/reconciliation
  before retry can succeed. There is intentionally no unsafe force-delete.
- Approval cleanup's existing handler can log errors clearing filesystem
  denial/attempt memory while returning `ok`; this harness verifies the pending
  inbox and that settings fall back to defaults, but cannot prove those ancillary
  scopes were purged through the approval worker's current public contract.
  No other worker is modified here.
- Session-owned post-turn hook SDK handles are unregistered when still known in
  this process. The pre-existing hook registry loses ownership across restart;
  legacy engine hooks without a durable owner stamp need operator cleanup.
  Normal durable harness bindings are enumerated and removed strictly.
- Engine trigger delivery remains the SDK's existing fire-and-forget event
  transport; durable status is authoritative. Queue redelivery republishes
  terminal snapshots. A disconnected client should recover status once when
  reconnecting.
- Cancellation deadline bounds preparation; cleanup checks the deadline between
  members and each RPC has its normal timeout. This is not a transactional
  cross-worker delete: partial cleanup is recoverable, not rolled back.

## Isolated validation

`tests/delete_session_tree.rs` uses an in-test SDK websocket fixture on an
OS-assigned loopback port, an in-memory store, and synthetic session ids. It
executes production handlers and never connects to a deployed engine or model.
The model delivery test stops at `context::assemble` after inspecting the real
context request, proving that delivery is not merely a UI event. Production
registration schemas and trigger schemas are golden-tested.

```sh
SKIP_UI_BUILD=1 cargo check --locked --offline -p harness
SKIP_UI_BUILD=1 cargo test --locked --offline -p harness --test delete_session_tree
SKIP_UI_BUILD=1 cargo test --locked --offline -p harness --lib --test schemas --test manifest
cargo fmt --all -- --check
```
