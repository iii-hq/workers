---
name: cursor
description: Run and resume Cursor coding agents through iii using local sandboxed workspaces or isolated cloud agents.
---

# Cursor worker

Use this worker when a task should be handled by a Cursor coding agent and the separately installed Cursor SDK Bridge is available.

- Call `cursor::run` for a blocking turn. `run::start_and_wait` is its standard alias.
- Call `cursor::start`, subscribe to `agent::events` with the returned session ID as `group_id`, and call `cursor::stop` when asynchronous lifecycle control is needed.
- Reuse `session_id` for follow-up turns. Local sessions must keep their original `cwd` and tool list.
- Call `cursor::status` or `cursor::sessions::list` for durable lifecycle discovery.
- Call `cursor::models::list` instead of guessing model IDs.
- Call `cursor::usage` only for cloud agents. Treat missing usage or cost as unreported, never as zero.

Local execution is sandboxed and defaults to the read-only `read`, `grep`, `glob`, and `ls` built-in tools. Passing a broader tool list expands what Cursor can do inside its sandbox. Cloud runs default to neither working on the current branch nor creating a pull request. All public Cursor functions require approval under the default policy.
