# session-tree::messages

Load every `AgentMessage` on the active path of a session, oldest first.

`({ session_id, branch_leaf? }) → { messages: [AgentMessage] }` — walks the parent
chain from the specified leaf (or the most-recently-appended entry if `branch_leaf`
is omitted) back to the root, then returns the `AgentMessage` values in root-first
order. Only `SessionEntry::Message` entries are included; `BranchSummary`,
`Compaction`, and `CustomMessage` entries on the path are silently skipped.

## When to use

- Reconstructing the context window for a resuming agent: pass the returned messages
  directly as the conversation history.
- Diffing two branches of a session tree: call with different `branch_leaf` values
  and compare the resulting message lists.
- Extracting the human-readable transcript of a single branch for display or export
  (see also `session-tree::export_html` for a styled HTML rendering).

## Notes

- `branch_leaf` is optional. If omitted, the function walks back from the last
  appended entry, which is the most recently recorded point in the main thread.
- Only `AgentMessage`-typed entries contribute to the output; other entry types
  (`BranchSummary`, `Compaction`) are traversed for parent-link resolution but
  do not appear in the returned array.
- The result reflects the active path only — sibling branches are not included.
  Use `session-tree::tree` to see the full DAG structure.
- For a complete `AgentContext` (including system prompt), assemble one from the
  returned messages in the calling agent rather than here.
