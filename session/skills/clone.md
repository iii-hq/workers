# session-tree::clone

Duplicate an entire session — all entries across all branches — with remapped ids,
and return the new session id.

`({ source_session_id }) → { session_id }` — copies every entry in the source
session, assigns each a fresh UUID, rewires all `parent_id` references to point at
the new ids, and persists everything into a new session record.

## When to use

- Taking a snapshot of a session before a destructive or experimental operation.
- Creating an identical starting point for a parallel agent run that must not share
  state with the original.
- Archiving a completed session while continuing to append to the original.

## Notes

- Unlike `session-tree::fork`, clone copies the entire entry set (all branches, not just
  the active path).
- All entry ids in the clone are new UUIDs; none overlap with those in the source
  session.
- Parent links in the clone are fully rewired: an entry whose source parent had id
  `X` will have a `parent_id` pointing at the clone's equivalent of `X`.
- The new session's display name is `"<source name> (clone)"` if the source has a
  display name.
- For very large sessions with many entries, clone can be expensive; prefer
  `session-tree::fork` when only the active-path context is needed.
