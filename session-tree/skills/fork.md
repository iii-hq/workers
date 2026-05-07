# session-tree::fork

Copy the active path up to a given entry into a new session and return the new
session id.

`({ source_session_id, from_entry_id }) → { session_id }` — walks the parent chain
from `from_entry_id` back to the root of `source_session_id`, then copies those
entries (with fresh UUIDs and rewired parent links) into a newly created session.
If the path contains more than 50 entries, a single `BranchSummary` placeholder is
written instead of copying every entry individually.

## When to use

- Branching a conversation at an earlier point to explore an alternative reply
  without disturbing the original session.
- Creating a "what-if" variant from a decision point in an agent's history.
- Isolating a sub-task context: fork at the entry where the sub-task began and work
  in the new session independently.

## Notes

- `from_entry_id` must exist in `source_session_id`; an error is returned otherwise.
- Only the active path from root to `from_entry_id` is copied — sibling branches are
  not included.
- When the path exceeds 50 entries, a `BranchSummary` entry is generated in the
  forked session in place of the full history. This keeps the fork lightweight while
  preserving a readable summary.
- Entry ids in the forked session are all new UUIDs; they do not collide with those
  in the source session.
- The new session's display name is `"<source name> (fork)"` if the source has a
  display name.
