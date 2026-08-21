---
name: cursor
description: Use Cursor through iii as a login-backed text provider or as a direct coding agent with durable local and cloud sessions.
---

# Cursor worker

Use this worker when a task should be handled by a Cursor coding agent. Local execution defaults to the official Cursor Agent CLI ACP and reuses credentials created by `cursor-agent login`; it does not require `CURSOR_API_KEY` or the SDK Bridge. Use the official CLI's absolute path when an installation exposes it only as `agent`.

- Select a `cursor/*` model in LLM Router for text-only chat through the normal Cursor login. The provider does not advertise router tools, vision, structured output, usage, or cost because Cursor ACP does not expose those raw-model contracts.

- Call `cursor::run` for a blocking turn. `run::start_and_wait` is its standard alias.
- Call `cursor::start`, subscribe to `agent::events` with the returned session ID as `group_id`, and call `cursor::stop` when asynchronous lifecycle control is needed.
- Reuse `session_id` for follow-up turns. Local sessions must keep their original `cwd` and tool list.
- Call `cursor::status` or `cursor::sessions::list` for durable lifecycle discovery.
- Call `cursor::auth::status` to check login availability without returning account details or credentials.
- Call `cursor::models::list` instead of guessing account-scoped model IDs. Login-backed results contain only ACP-compatible IDs and expose ACP `default` as `auto`; `cursor-agent --list-models` also contains CLI-only IDs that ACP rejects. Pass `{ "backend": "sdk-bridge" }` for the cloud/API-key catalog.
- Call `cursor::usage` only for cloud agents. Treat missing usage or cost as unreported, never as zero.

Login-backed local execution stays in Cursor's `ask` mode and cancels every permission request. Cursor ACP cannot enforce a per-request tool list, so omit `tools`; any explicit value is rejected. Cloud runs require the separately installed SDK Bridge and `CURSOR_API_KEY`, and default to neither working on the current branch nor creating a pull request.

The LLM Router provider functions are internal and denied to agents; use `router::chat` or `router::complete` through a selected `cursor/*` model. Read-only status, session, auth, and model discovery are allowed by the default policy; direct run, start, stop, and usage stay approval-gated.
