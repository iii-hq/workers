# session::compact

Append a `Compaction` entry to a session, recording a summary of the context that
was compressed and the token count before compaction.

`({ session_id, summary, tokens_before, details?, parent_id? }) → { entry_id }` —
creates a `SessionEntry::Compaction` with the provided summary text, `tokens_before`
count, and optional file-operation details (`read_files`, `modified_files`). Anchors
the entry at `parent_id` if given, or at the tail of the current active path if
omitted.

## When to use

- After a context-compaction step: record that the conversation was summarised and
  note which files were read or modified so future agents have a breadcrumb.
- Marking a checkpoint in a long-running agent loop: the compaction entry acts as a
  divider between history segments.
- Logging the before-token count for cost-tracking or monitoring dashboards.

## Notes

- `summary` (required): human-readable description of what was compacted.
- `tokens_before` (optional, defaults to `0`): token count before the compaction
  step; used for reporting and monitoring.
- `details.read_files` and `details.modified_files` are optional string arrays;
  omit the `details` field entirely if no file operations occurred.
- `parent_id` is optional; if omitted, the compaction entry is anchored at the
  most recent entry on the active path.
- `session::compact` is not idempotent: calling it twice creates two compaction
  entries. Check existing entries before calling if idempotency is required.
