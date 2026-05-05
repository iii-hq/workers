# session-tree

Session storage as a parent-id tree of typed entries on the iii bus. Stores
agent messages, custom messages, and tool results, addressable by parent id
so multi-turn forks share a common history.

## Installation

```bash
iii worker add session-tree
```

## Run

```bash
iii-session-tree --engine-url ws://127.0.0.1:49134
```

Defaults to an iii-state-backed store (sessions survive worker restart).
Set `SESSION_TREE_STORE=memory` for ephemeral in-process storage; see
[Storage backends](#storage-backends) below.

## Registered functions (P0 + P2 surface)

P0: `session::create`, `session::load`, `session::append`,
`session::active_path`, `session::list`, `session::load_messages`.

P2: `session::fork`, `session::clone_session`, `session::compact`,
`session::export_html`, `session::tree`.

## Storage backends

`session-tree` supports two storage backends, selected via the
`SESSION_TREE_STORE` env var:

| Value | Persistence | Use case |
|---|---|---|
| `iii_state` (default) | Survives worker restart via `iii-state` | Production |
| `memory` | In-process only; lost on restart | Tests, local dev |

The `iii_state` backend uses a scope-per-session layout for bounded scan cost:

- Scope `session_tree:<session_id>`, key `<entry_id>`, value `SessionEntry`
- Scope `session_tree_meta`, key `<session_id>`, value `SessionMeta`

`load_entries(sid)` lists the per-session scope and sorts by lexicographic
`entry_id`. `append` is `O(1)` regardless of session length: one `state::set`
for the entry, plus a non-fatal `state::set` to refresh `SessionMeta::updated_at`.
If the meta refresh fails, the entry still persists and a warning is logged;
`updated_at` may be slightly stale.

### Failure semantics

`SessionStore` methods return `Result<_, SessionError>`. With the `iii_state`
backend, transient bus failures (engine restart, IPC hiccup) map to
`SessionError::Storage(...)`. With `memory`, errors are limited to logical
conditions like `NotFound` and `RwLock` poison.

`load_meta` returns `SessionError::NotFound(<sid>)` when the session doesn't
exist; `load_entries` returns an empty vec for a session with no appended
entries (never `NotFound`).

## Build

```bash
cargo build --release
```
