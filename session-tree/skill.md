# session-tree

Persistent branching session storage: create, append to, read, fork, clone, compact,
and export conversation histories as parent-linked entry trees.

- [`session-tree`](iii://session-tree)
  - [`session::create`](iii://session-tree/create) — create a new empty session record
  - [`session::append`](iii://session-tree/append) — append an AgentMessage entry to a session

  - [`session::messages`](iii://session-tree/messages) — load every AgentMessage on the active path, oldest first
  - [`session::tree`](iii://session-tree/tree) — return the full session DAG as a nested TreeNode

  - [`session::fork`](iii://session-tree/fork) — copy the active path up to a given entry into a new session
  - [`session::clone`](iii://session-tree/clone) — duplicate an entire session with remapped ids
  - [`session::compact`](iii://session-tree/compact) — append a Compaction entry summarising the active path

  - [`session::export_html`](iii://session-tree/export_html) — render the active path as a self-contained HTML document

Storage backend is selected at startup via `SESSION_TREE_STORE` (`iii_state` by
default; set to `memory` for ephemeral in-process use).
