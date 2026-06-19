# Configuration and the update-adapter lifecycle

Every operator-facing setting lives in the external `configuration` worker and
**hot-reloads without a restart** — including `bot_token` and the choice between
polling and webhook ingress. This document covers the config schema, the
`configuration`-worker integration, the boot sequence, the hot-reload path, and
the `updates` adapter lifecycle (including how the webhook HTTP route is created
and removed on demand).

Source: [`src/config.rs`](../src/config.rs) (schema + types),
[`src/configuration.rs`](../src/configuration.rs) (worker integration + reload),
[`src/ingress.rs`](../src/ingress.rs) (adapter lifecycle),
[`src/functions/mod.rs`](../src/functions/mod.rs) (trigger registration),
[`src/main.rs`](../src/main.rs) (boot).

## 1. Mental model

`WorkerConfig` is the single source of truth. It is:

- **Serialized to JSON Schema** and registered with the `configuration` worker so
  the console can render an editor (the `updates` field becomes a `oneOf`
  variant picker for polling/webhook).
- **Held in a `ConfigCell`** = `Arc<RwLock<Arc<WorkerConfig>>>` — an atomically
  swappable snapshot read everywhere via `deps.cfg().await`.
- **Reloaded reactively**: a `configuration:updated` event for this worker's id
  re-fetches, re-validates, swaps the cell, and re-applies any side effects
  (ingress, command menu).

## 2. The config schema

| Field | Type / default | Meaning |
|---|---|---|
| `bot_token` | string, **required** | Telegram Bot API token. Env-expandable (`"${TELEGRAM_BOT_TOKEN}"`). Empty is fatal at boot, rejected (keep-previous) on reload. |
| `updates` | adapter, default `polling` | Ingress selection — `polling` or `webhook` (see [§4](#4-the-updates-adapter-lifecycle)). |
| `default_model` | `{provider, id}`, optional | When set, `/start` skips the model picker and uses this model. |
| `verbosity` | `none`\|`minimal`\|`high`\|`debug`, default `none` | How much transcript is mirrored to Telegram. |
| `default_thinking_level` | `minimal`\|`low`\|`medium`\|`high`\|`xhigh`, optional | Default harness reasoning depth (`options.thinking_level`). |
| `streaming` | object | Transport (`auto`/`draft`/`edit`), `draft_id_seed`, `draft_throttle_ms`, `create_settle_ms`. |
| `steering_mode` | `steering`\|`fifo`, default `steering` | `steering` merges mid-turn messages into the running turn; `fifo` queues locally and drains one per turn. |
| `functions_allow` | `[glob]` | Passed to `harness::send` `options.functions.allow`. |
| `system_prompt` | string, optional | System prompt added to every send. |
| `timeout_ms` | u64, default `10000` | Timeout for all harness/approval/state/configuration RPCs. |

The `updates` adapter is **adjacently tagged** (`name` discriminator + nested
`config`), mirroring session-manager's storage adapter shape:

```yaml
updates:
  name: polling                # or: webhook
  config:
    timeout_seconds: 50         # polling: long-poll seconds (Telegram max 50)
```

```yaml
updates:
  name: webhook
  config:
    base_url: "https://engine.example"   # iii engine root; bot appends the path
    secret: "your-webhook-secret"        # recommended (header validation)
```

### Migration tolerance

`WorkerConfigRaw` (`#[serde(deny_unknown_fields)]`) absorbs legacy keys so old
stored configs keep parsing: deprecated timeout aliases
(`harness_send_timeout_ms`, `approval_timeout_ms`, `state_timeout_ms`) fold into
`timeout_ms`; removed display fields (`thinking_display`, `use_rich`,
`edit_throttle_ms`) are ignored. The webhook `base_url` field accepts the legacy
`url` key via `#[serde(alias = "url")]`.

## 3. The configuration-worker integration

| Function | Role |
|---|---|
| `register_config` | Registers the schema + id/name/description via `configuration::register` (with retry). Seeds `initial_value` from the `--config` seed if given, else from defaults **only when the store has no value yet**. |
| `fetch_config` | `configuration::get` for this id; a `null` value → built-in defaults, missing → error. |
| `apply_config` | Validates a candidate snapshot, logs token rotation, atomically swaps the `ConfigCell`. Returns `false` (keep previous) on validation failure. |
| `register_config_trigger` | Registers the `on-config-change` function and binds it to a `configuration` trigger filtered to `configuration:updated` for this id. |
| `on_config_change` | The reload handler (see [§5](#5-hot-reload)). |

All `configuration::*` RPCs go through `trigger_with_retry` (3 attempts, linear
250 ms × attempt backoff).

## 4. The `updates` adapter lifecycle

This is the core of how ingress is wired, and where the webhook HTTP route is
created and torn down. The whole flow runs through one function,
`ingress::apply_updates_adapter`, which is reached both at boot
(`ingress::start`, with `prev = None`) and on every adapter-changing reload
(`ingress::apply_config_change`). It is **idempotent** and **restart-free**.

### Why `base_url` and not a full URL

The iii SDK gives a worker no way to discover the engine's **public** base URL —
there is no `public_url`/`base_url` accessor and no env var for it (the only URL
the SDK exposes is the WebSocket control address, `ws://127.0.0.1:49134`). The
worker therefore cannot self-derive the endpoint Telegram should POST to, so the
operator supplies the **iii engine root** (`base_url`) and the worker appends its
own route path. The path is a single constant,
`config::WEBHOOK_API_PATH = "telegram-bot/webhook"`, reused for **both** the
engine route registration and the Telegram URL — so the two can never drift:

```
endpoint_url = {base_url trimmed of trailing '/'} + "/telegram-bot/webhook"
```

`WebhookConfig::endpoint_url()` returns `None` for an empty `base_url`, and
returns the value as-is if it already ends with the path (so a legacy full-URL
config is not double-suffixed).

### Switching to **webhook**

`apply_updates_adapter` (webhook branch):

1. `stop_poller` — cancel any running poller and await its handle.
2. Compute `endpoint_url()`. If `None` (no `base_url`), warn and return —
   ingress is left inactive (no route, no `setWebhook`).
3. **`ensure_webhook_trigger`** — if no webhook route handle is held yet,
   `register_trigger` the `telegram-bot/webhook` HTTP POST route and **retain the
   returned `Trigger` handle** in `RuntimeState.webhook_trigger`. The engine route
   must exist *before* Telegram is told to POST, or early updates would hit an
   unregistered path. If registration fails, the handle is left unset (so the next
   reload retries) and `setWebhook` is skipped.
4. `setWebhook` with the derived URL and the optional `secret`.

### Switching to **polling**

`apply_updates_adapter` (polling branch):

1. `stop_poller`.
2. `deleteWebhook` — tell Telegram to **stop POSTing first**.
3. **`unregister_webhook_trigger`** — `take()` the retained handle and call
   `.unregister()`. (Order matters: removing the route before `deleteWebhook`
   would leave a window where Telegram POSTs to a dead path.)
4. `start_poller` — spawn the background `getUpdates` loop.

### Idempotency and no route leaks

- `ensure_webhook_trigger` guards on "handle already held", so re-entering the
  webhook branch (e.g. a webhook→webhook change of `secret`) does **not**
  re-register the route — it only re-issues `setWebhook` with the new value. This
  matters because `register_trigger` mints a fresh UUID per call, so blind
  re-registration would accumulate duplicate engine routes.
- `ingress_changed` short-circuits no-op reloads: `polling → polling` is treated
  as unchanged (the poller picks up a new `timeout_seconds` live), and
  `webhook → webhook` only re-applies when the config actually differs.
- The `Trigger` handle has **no `Drop`**: dropping it does *not* unregister. The
  code always `take()`s and explicitly `.unregister()`s — never overwrites the
  `Option` — so the route can never silently linger.

### Boot and shutdown

- At **boot**, only the always-on control route `telegram-bot/set-webhook` is
  registered statically (`bind_http_triggers`). The webhook ingress route is
  registered by `ingress::start` → `apply_updates_adapter` **only if** the
  configured adapter is webhook. So a polling deployment never creates the route.
- At **shutdown**, `ingress::shutdown` stops the poller and unregisters the
  webhook route, leaving a clean slate for the next start rather than relying on
  the engine to garbage-collect a disconnected worker's triggers.

### The `set-webhook` control endpoint

`POST /telegram-bot/set-webhook` (function `telegram-bot::set-webhook`) is a
manual re-arm: it re-issues `setWebhook` with the derived `endpoint_url()` from
current config. It is redundant in steady state (switching adapters already does
this) but useful to re-register if Telegram drops the webhook. Its route is
always registered; it errors if the active adapter is not webhook or `base_url`
is unset.

## 5. Hot-reload

```
configuration worker emits configuration:updated (id = telegram-bot)
        │
        ▼
on-config-change fn fires ─► on_config_change:
        1. read prev snapshot (for its timeout_ms)
        2. fetch_config_with_timeout (re-read authoritative value)
              └─ fetch failure → keep previous, return
        3. apply_config (validate + swap the ConfigCell)
              └─ empty bot_token → keep previous (non-fatal)
        4. ingress::apply_config_change(prev, next)
              └─ only if ingress_changed → apply_updates_adapter (see §4)
        5. set_my_commands (refresh the command menu)
```

Functions and sibling/HTTP triggers are **not** re-registered on reload — only
the config cell is swapped, ingress conditionally re-applied, and the command
menu refreshed. `bot_token` rotation is picked up by the next API call (the token
is read from the snapshot per call).

## 6. Boot sequence (`main.rs`)

Exact order, all once at boot:

1. init tracing/telemetry; parse CLI (`--config` seed, `--url`, `--manifest`).
2. `register_worker` over the engine WebSocket.
3. `configuration::register_config` — schema + conditional seed.
4. `configuration::fetch_config` — load the authoritative value.
5. `cfg.validate()` — **empty `bot_token` here is fatal** and aborts boot.
6. build `ConfigCell` + `Deps`.
7. `functions::register_all` — register all handler functions.
8. `functions::bind_triggers` — best-effort bind the six sibling-event handlers
   (missing siblings only warn).
9. `functions::bind_http_triggers` — register the always-on `set-webhook` route.
10. `configuration::register_config_trigger` — bind the reload handler.
11. `ingress::start` — apply the adapter (registers the webhook route iff
    webhook; otherwise starts the poller).
12. `set_my_commands` — publish the command menu.
13. await Ctrl-C, then `ingress::shutdown` + `iii.shutdown_async`.

## 7. Environment expansion

`${VAR}` references in a YAML config seed are expanded from the process
environment at parse time (`expand_env`), so `bot_token: "${TELEGRAM_BOT_TOKEN}"`
works. (The authoritative store value from the `configuration` worker is JSON and
is not env-expanded — expansion applies to file/YAML seeds.)
