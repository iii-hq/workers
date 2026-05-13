# Reconstructing the active path for a resuming agent

## When to use

- Reconstructing the conversation history for a resuming agent. The returned messages slot into the next request.
- Diffing two branches: call once per `branch_leaf` and compare the resulting arrays.
- Extracting a single branch's transcript for display (use `session-tree::export_html` when you want it styled).

## Notes

- `branch_leaf` is optional; without it the function walks back from the most recently appended entry on the main thread.
- Only `AgentMessage`-typed entries appear in the output. `BranchSummary` and `Compaction` are traversed for parent-link resolution but do not contribute to the returned array.
- Sibling branches are excluded; call `session-tree::tree` when you need the full DAG.
- For an `AgentContext` that includes the system prompt, assemble it in the calling agent rather than here.
