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

The default backend is in-memory — sessions are lost on restart. Production
deployments swap in a persistent `Store` implementation (filesystem, SQLite,
or an iii-state-backed adapter).

## Registered functions (P0 + P2 surface)

P0: `session::create`, `session::load`, `session::append`,
`session::active_path`, `session::list`, `session::load_messages`.

P2: `session::fork`, `session::clone_session`, `session::compact`,
`session::export_html`, `session::tree`.

## Build

```bash
cargo build --release
```
