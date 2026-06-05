# session

Session storage (`session-tree::*` parent-linked tree) on the iii bus.

## Purpose

The session worker owns the persistent shape of an agent run. Each session
is a parent-linked tree of entries (`AgentMessage` entries plus
`Compaction` summary entries); the FSM appends messages there, the
compactor writes summary entries, and the UI loads the active path. A
session can be forked at any entry (producing a sibling branch sharing
history up to that point) or cloned (deep copy with new ids).

Production storage uses `IiiStateSessionStore`: entries under iii state
scopes `session_tree:<session_id>` (per-entry) and `session_tree_meta`
(per-session metadata). Unit tests use `InMemoryStore` directly (not a
worker config option).

## Registered functions

### `session-tree::*`

- `session-tree::fork` — Fork a session at a given entry into a new session id.
- `session-tree::clone` — Duplicate a session with re-mapped ids.
- `session-tree::compact` — Append a Compaction entry summarising the active path.
- `session-tree::tree` — Return the session tree as a nested TreeNode.
- `session-tree::export_html` — Render the active path as a self-contained HTML document.
- `session-tree::create` — Create a new empty session record.
- `session-tree::ensure` — Idempotently ensure a session exists with the given id.
- `session-tree::append` — Append an AgentMessage entry to a session.
- `session-tree::messages` — Load every AgentMessage on the active path of a session, paired with its entry_id, oldest first.
- `session-tree::list` — List sessions with optional pagination and ordering.
- `session-tree::compactions` — Return all Compaction entries for a session, sorted by timestamp ascending.
- `session-tree::append_synthetic` — Append a synthetic user-role message entry to a session (used for the post-compaction continue nudge).
- `session-tree::update_part` — Replace the content of a `function_result` message entry with compacted output.
- `session-tree::update_parts` — Batch variant of `update_part`; loads target entries once and rewrites all of them.

## Triggers

None — this worker is a pure storage surface.

## State keys

`session-tree::*` (under `IiiStateSessionStore`):

| Scope | Key shape | Value |
|---|---|---|
| `session_tree:<session_id>` | `<entry_id>` | `SessionEntry` — one of `message`, `custom_message`, `branch_summary`, `compaction`. Every variant carries `id`, `parent_id`, and an explicit `timestamp`. |
| `session_tree_meta` | `<session_id>` | `SessionMeta` (display_name, cwd, created_at, updated_at, branch_count). |

`state::list` returns values without keys, so entries are re-sorted in
`loadEntries` by `(timestamp, id)` rather than `id` alone — this keeps
resumed approval replies in the correct transcript position when their
ids are non-monotonic relative to wall-clock order.

## Dependencies

From [src/session/iii.worker.yaml](harness/src/session/iii.worker.yaml):
`iii-state ^0.11.0`.

## Source layout

| File | Purpose |
|---|---|
| [src/session/main.ts](harness/src/session/main.ts) | Binary entry point (`iii-session`). |
| [src/session/register.ts](harness/src/session/register.ts) | Registers `session-tree::*` on `IiiStateSessionStore`. |
| [src/session/tree/register.ts](harness/src/session/tree/register.ts) | Registers all 15 `session-tree::*` functions; exports `FUNCTION_IDS`. |
| [src/session/tree/operations.ts](harness/src/session/tree/operations.ts) | Pure tree algorithms: create, fork, clone, compact, active path, messages, reconcile, tree, export_html, list. |
| [src/session/tree/store.ts](harness/src/session/tree/store.ts) | `SessionStore` interface + `InMemoryStore` + `IiiStateSessionStore`. |
| [src/session/tree/types.ts](harness/src/session/tree/types.ts) | `SessionEntry` (`message` / `custom_message` / `branch_summary` / `compaction`, each with an explicit `timestamp`), `SessionMeta`, `TreeNode`, `ReconcileResult`, `SessionError`, plus the `entryTimestamp` helper used by the `(timestamp, id)` sort. |
| [src/session/iii.worker.yaml](harness/src/session/iii.worker.yaml) | Worker manifest. |
