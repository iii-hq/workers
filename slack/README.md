# slack

Slack as an iii worker. Two surfaces in one worker:

1. **Slack Web API** as `slack::*` functions — agent-callable and
   console-callable, so anything you can do through the Slack API you can do
   through this worker. Needs only a bot token; plain outbound HTTPS, no ingress.
2. **Harness bridge** — @mention the bot in Slack and it runs a `harness::send`
   turn, streaming the reply back into the thread using Slack's native streaming
   (`chat.startStream`/`appendStream`/`stopStream`), with inline Block Kit
   approvals. The bridge is additive: it activates only when a public engine URL +
   signing secret are configured and the harness stack is connected.

## Install

```bash
# API surface only:
iii worker add slack
# For the bridge, also install the harness stack and console:
iii worker add harness console
```

## Configure

Set a bot token (and optionally a user token for search) in the console
**Configuration → Workers → slack**, or seed it from the environment:

```yaml
bot_token: "${SLACK_BOT_TOKEN}"   # xoxb-, required
user_token: "${SLACK_USER_TOKEN}" # xoxp-, optional (only slack::search::messages)
timeout_ms: 10000
```

Identity (workspace, team id, bot user) is **not** configured — it is discovered
at boot via `auth.test` and reported by `slack::config-status`. All fields
hot-reload through the `configuration` worker; no restart.

Run `slack::config-status` to verify the token and see the resolved identity.

### Enable the bridge

```yaml
signing_secret: "${SLACK_SIGNING_SECRET}"      # required for the bridge
public_base_url: "https://your-engine.example" # Slack posts events here
require_mention: true        # channels fire only on @bot mention (DMs always)
backfill_thread: true        # pull prior thread replies into the session on first mention
default_model: null          # optional; else the first model from router::models::list
functions_allow: ["slack::*"]
```

Point the Slack app's **Event Subscriptions** request URL at
`{public_base_url}/slack/events` and the **Interactivity** request URL at
`{public_base_url}/slack/interactions`. Subscribe to `app_mention`, `message.im`,
`message.channels`, and `assistant_thread_started`. Every inbound request is
HMAC-verified against `signing_secret`.

**Triggering:** in channels the bot acts only when explicitly @mentioned; untagged
messages are captured as thread context and folded into the next mention. DMs
always trigger. Sessions are keyed per thread.

## Functions

Messaging — `slack::chat::post-message` · `update` · `delete` · `post-ephemeral`
· `schedule-message` · `get-permalink`

Conversations — `slack::conversations::list` · `info` · `history` · `replies` ·
`create` · `invite` · `join` · `members` · `open` · `set-topic` · `set-purpose` ·
`archive`

Reactions — `slack::reactions::add` · `remove` · `get`

Files — `slack::files::upload` (reserve URL → upload bytes → finalize) · `info` ·
`list`

Users — `slack::users::list` · `info` · `lookup-by-email` · `profile-get`

Views — `slack::views::open` · `publish` · `update` · `push`

Pins / bookmarks — `slack::pins::add` · `list` · `slack::bookmarks::add`

Search — `slack::search::messages` (requires `user_token`)

Assistant — `slack::assistant::set-status` · `set-title` · `set-suggested-prompts`

Admin — `slack::auth::test` · `slack::config-status`

Escape hatch — `slack::call { method, params, as_user? }` calls any Slack Web API
method by name, so every method is reachable even before it has a typed wrapper.

Bridge — `slack::notify { session_id, text|blocks }` reaches the Slack thread
bound to a session out-of-band (e.g. a scheduled reminder). `slack::events` and
`slack::interactions` are the HMAC-verified HTTP ingress endpoints; the
`slack::on-*` functions are internal bindings that stream harness output and post
approvals.

Typed functions accept the well-known parameters explicitly and pass any
additional Slack parameters through. Each returns the full Slack response payload.

## Local development

```bash
cargo run -- --url ws://127.0.0.1:49134 --config ./config.collect.yaml
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```
