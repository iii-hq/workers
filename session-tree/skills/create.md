# session::create

Create a new empty session record and return its generated id.

`({ display_name?, cwd? }) → { session_id }` — allocates a UUID, persists session
metadata (display name, working directory, timestamps), and initialises an empty
entry list. The caller must follow up with `session::append` to populate the session
with messages.

## When to use

- An agent or orchestration flow is starting a fresh conversation and needs a session
  to record its history.
- A new task is launched and the harness needs to allocate a storage handle before
  the first message is produced.
- Building a replay or fork workflow where a fresh session root is required before
  copying entries into it.

## Notes

- `display_name` and `cwd` are both optional; omit them for anonymous sessions.
- The returned `session_id` is a UUID string; pass it verbatim to all other
  `session::*` functions.
- Sessions are persisted in whichever backend is configured at startup (`iii-state`
  by default; override with `SESSION_TREE_STORE=memory` for ephemeral use).
- `created_at` and `updated_at` are set to the current millisecond epoch at creation
  time; `updated_at` advances on every `session::append` call.
