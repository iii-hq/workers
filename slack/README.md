# slack

Slack as an iii worker. It exposes the Slack Web API as `slack::*` functions —
agent-callable and console-callable — so anything you can do through the Slack
API you can do through this worker. Configure it from the console (or env) and
call `slack::chat::post-message`, `slack::conversations::list`, and the rest.

> **Milestone 1.** This ships the **Slack Web API surface + console config**. The
> harness bridge (inbound messages → `harness::send`, native streaming, approvals)
> lands in later milestones. The API surface works standalone — it needs only a
> bot token and makes plain outbound HTTPS calls to Slack; no ingress, no public
> URL.

## Install

```bash
iii worker add slack
```

## Configure

Set a bot token (and optionally a user token for search). Edit it in the console
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

Typed functions accept the well-known parameters explicitly and pass any
additional Slack parameters through. Each returns the full Slack response payload.

## Local development

```bash
cargo run -- --url ws://127.0.0.1:49134 --config ./config.collect.yaml
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```
