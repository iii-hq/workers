# Visualising the full session DAG

## When to use

- Visualising the complete branching history of a session, including every fork.
- Debugging unexpected branching: inspect the tree to find entries with multiple children or orphans.
- Driving a UI that lets users navigate between branches and pick a leaf to resume from.

## Notes

- Returns an error if the session has no entries or no root (an entry with `parent_id: null`).
- A well-formed session has exactly one root; if multiple parent-less entries exist, the first appended is treated as the root.
- `TreeNode.entry` may be any `SessionEntry` variant: `Message`, `CustomMessage`, `BranchSummary`, or `Compaction`.
- For large sessions with many forks the response can be significant; prefer `session-tree::messages` when only the active path is needed.
