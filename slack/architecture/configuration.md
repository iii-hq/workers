# slack — configuration

All fields hot-reload through the `configuration` worker and render as a form in
the console (Configuration → Workers → slack). The schema is registered at boot;
nothing is committed as a static `config.yaml` (defaults live in
`WorkerConfig::default()` and appear as the schema `example`).

| Field | Description |
|---|---|
| `bot_token` | **Required.** Slack bot token (`xoxb-`). Env-expandable: `"${SLACK_BOT_TOKEN}"`. |
| `user_token` | Optional user token (`xoxp-`); required only by `slack::search::messages`. |
| `signing_secret` | Required to enable the inbound bridge (HMAC verify). |
| `public_base_url` | Public iii engine root; Slack posts to `{public_base_url}/slack/events`. Required for the bridge. |
| `default_channel` | Optional default channel id for proactive sends. |
| `allowed_channels` | Optional allowlist of channel ids the bridge responds in (empty = all). |
| `allowed_teams` | Optional allowlist of team ids accepted (empty = all). |
| `require_mention` | Channels fire a turn only on `@bot` mention. DMs always trigger. Default true. |
| `backfill_thread` | Pull prior thread replies into the session on first mention. Default true. |
| `backfill_max_messages` | Cap on backfilled/pending context messages. Default 50. |
| `default_model` | Optional `{provider,id}`, validated against `router::models::list`. Provider-agnostic. |
| `system_prompt` | Optional system prompt appended to every harness send. |
| `functions_allow` | Globs for `harness::send` `options.functions.allow`. Default `["slack::*"]`. |
| `timeout_ms` | Slack Web API and engine RPC timeout (ms). Default 10000. |

## Identity is discovered, not configured

The workspace, team id, bot user id, and org (enterprise) id are resolved at boot
via `auth.test` and reported by `slack::config-status`. Channel ids are per-call
parameters / arrive in events; channel names are never stored (resolve via
`conversations.list`/`info`).

## Secrets

Config reads expand `${VAR}` against the live process env on every read, so token
fields default to env indirection (`${SLACK_BOT_TOKEN}`) with the real secret in
the engine env; a raw paste in the console also works. Token values are never
logged, including on rotation.

## Bridge enablement and reload

`bridge_enabled()` is true only when both `signing_secret` and `public_base_url`
are set. Toggling `public_base_url` registers or unregisters the HTTP routes on
reload (`ingress::apply`); all other fields are read from the live snapshot per
call.
