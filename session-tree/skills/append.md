# session::append

Append an `AgentMessage` entry to an existing session and return the new entry id.

`({ session_id, message, parent_id? }) → { entry_id }` — wraps the provided
`AgentMessage` in a `SessionEntry::Message`, assigns a fresh UUID and the current
timestamp, and persists it in the store. The `updated_at` timestamp on the session
metadata is advanced.

## When to use

- Recording each turn of a conversation as it occurs (user messages, assistant
  replies, tool results).
- Building a branching history: pass the previous entry's id as `parent_id` to
  chain entries into a linked list; omit `parent_id` to start a new root (rare).
- Writing replays or synthetic sessions entry-by-entry before calling
  `session::messages` or `session::export_html`.

## Notes

- `message` must be a valid serialised `AgentMessage` (variants: `user`,
  `assistant`, `tool_result`, `custom`). The schema is defined in `harness-types`.
- `parent_id` is optional; if omitted, the entry has no parent (`parent_id: null`).
  In a linear conversation thread every entry except the first should supply the
  previous `entry_id` as `parent_id`.
- Entry ids are UUIDs generated server-side; do not supply your own id.
- `session::append` is not idempotent: calling it twice with the same payload
  creates two distinct entries.
