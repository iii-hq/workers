# session::tree

Return the full session history as a nested `TreeNode` DAG, rooted at the first
entry appended with no parent.

`({ session_id }) → TreeNode` — loads all entries for the session and assembles them
into a recursive structure: `{ entry: SessionEntry, children: [TreeNode] }`. Branches
in the tree correspond to entries that share the same `parent_id`.

## When to use

- Visualising the complete branching history of a session (all forks, not just the
  active path).
- Debugging unexpected branching: inspect the tree to find entries with multiple
  children or orphaned nodes.
- Building a UI that lets users navigate between conversation branches and pick a
  leaf to resume from.

## Notes

- Returns an error if the session has no entries or no root entry (an entry with
  `parent_id: null`).
- In a well-formed session exactly one entry has `parent_id: null`; if multiple
  root-less entries exist, the first one appended is treated as the root.
- `tree` returns the full DAG (every branch); contrast with `session::messages`,
  which returns only the messages on a single active path.
- `TreeNode.entry` is a `SessionEntry` (may be `Message`, `CustomMessage`,
  `BranchSummary`, or `Compaction`).
- For large sessions with many forks the response payload can be significant; prefer
  `session::messages` when only the active path is needed.
