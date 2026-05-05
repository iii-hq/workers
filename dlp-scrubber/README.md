# dlp-scrubber

DLP secret-scrubber subscriber on `agent::after_tool_call`. Redacts common
secret shapes in the result's text content and replies with the rewritten
content so the runtime's `merge_after` overrides the original result.

## Installation

```bash
iii worker add dlp-scrubber
```

## Run

```bash
iii-dlp-scrubber
```

When started by the engine, the worker reads its `config:` block from
`--config <path>`. Defaults:

```yaml
topic: agent::after_tool_call
```

`DLP_SCRUBBER_TOPIC` overrides the config file for direct runtime overrides.

## Patterns matched

| Provider | Regex |
|---|---|
| AWS | `AKIA[0-9A-Z]{16}` |
| OpenAI | `sk-[A-Za-z0-9]{32,}` |
| GitHub | `ghp_[A-Za-z0-9]{36}` |
| Stripe | `sk_live_[A-Za-z0-9]{24,}` |
| Google | `AIza[0-9A-Za-z_-]{35}` |

Each match becomes `[REDACTED:<kind>]`.

## Registered functions

| Function | Description |
|---|---|
| `policy::dlp_scrubber` | Subscriber bound to the configured topic. Replies `{ content: [...] }` when scrubbing changed anything. |

## Runtime expectations

Same as `policy-denylist` and `audit-log` — by default needs a publisher of
`agent::after_tool_call` on the bus (typically `provider-router`).

## Build

```bash
cargo build --release
```
