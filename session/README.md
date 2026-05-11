# session

Consolidated session worker. Registers two iii function namespaces from a single binary:

- `session-tree::*` — persistent branching session storage (create, append, messages, tree, fork, clone, compact, export_html, list, ensure, reconcile).
- `session-inbox::*` — per-session inbox queues (push, drain, peek) backed by iii state.

Both surfaces share the same engine connection, runtime, and release cadence.

## Library layout

- `session::tree` — parent-id tree of typed `SessionEntry` values.
- `session::inbox` — append/drain/peek per `inbox_key(name, session_id)`.

The wire-level function ids (`session-tree::*`, `session-inbox::*`) are unchanged
from the original split workers; callers do not need to migrate.

## Configuration

`config.yaml` (loaded via `--config`):

```yaml
# Backend for session-tree (iii_state | memory).
store_backend: iii_state
# WebSocket URL when --url / III_URL is unset.
engine_url: ws://127.0.0.1:49134
# iii state scope used by session-inbox keys.
state_scope: agent
```
