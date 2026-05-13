# Recording a context compaction in the history

## When to use

- After compacting context: record that the conversation was summarised and note which files were read or modified.
- Marking a checkpoint in a long-running agent loop: the compaction entry divides history segments.
- Logging the pre-compaction token count for cost-tracking or monitoring dashboards.

## Notes

- `summary` is required; `tokens_before` defaults to zero.
- `details.read_files` and `details.modified_files` are optional string arrays. Omit `details` entirely when no file operations occurred.
- `parent_id` is optional. Without it, the compaction entry anchors at the most recent entry on the active path.
- The call is not idempotent. Invoking it twice writes two distinct compaction entries. Inspect existing entries first when idempotency matters.
