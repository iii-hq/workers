---
name: slack
description: >-
  Drive Slack from iii — expose the Slack Web API as slack::* functions
  (post messages, manage channels, reactions, files, users, views, search,
  assistant threads), configurable from the console. Anything the Slack API
  can do, this worker can do.
---

# slack

The slack worker exposes the Slack Web API as `slack::*` iii functions, callable
by an agent or from the console. It owns no opinion about what you do with Slack:
it is a faithful, typed surface over the API plus a generic `slack::call` escape
hatch for any method without a typed wrapper.

Reach for it to let an agent (or an operator) post messages, read history, react,
upload files, look up users, open modals, publish App Home, or search — using a
single configured bot token.

## When to Use

- Post or edit messages, ephemeral messages, or scheduled messages in a channel,
  DM, or thread (`slack::chat::*`).
- List/inspect conversations and read history or thread replies
  (`slack::conversations::*`).
- React, pin, bookmark, upload files, look up users, or open Block Kit modals /
  App Home views.
- Search messages (with a configured user token).
- Call any other Slack method via `slack::call { method, params }`.

## Boundaries

- The **bridge** (inbound `app_mention`/DM → `harness::send`, streamed replies,
  approvals) activates only when `signing_secret` + `public_base_url` are set and
  the harness stack is connected. Without them the worker is API-surface-only.
- In channels the bridge acts only on an explicit @mention; DMs always trigger.
  Untagged messages are captured as context, never acted on alone.
- Identity (team, workspace, bot user) is discovered via `auth.test`, never
  configured. Models/providers come from `router::models::list` — no vendor names
  are baked in.
- `slack::search::messages` needs a `user_token` (`xoxp-`); bot tokens cannot
  search.

## Configuration

Set `bot_token` (required) — and `user_token` for search — in the console
**Configuration → Workers → slack**, or seed from env (`${SLACK_BOT_TOKEN}`).
Run `slack::config-status` to validate the token and see the resolved identity.
All fields hot-reload.

## Functions

Each typed function names the common Slack parameters and passes any extra
parameters through; the full Slack response payload is returned. See the
[README](https://github.com/iii-hq/workers/blob/main/slack/README.md) for the full list. Use `slack::call` for anything not yet
typed.
