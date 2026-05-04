# provider-anthropic

Native Anthropic Messages API streaming provider on the iii bus.

## Installation

```bash
iii worker add provider-anthropic
```

## Run

```bash
iii-provider-anthropic
```

`III_URL` env var picks the engine endpoint (default `ws://127.0.0.1:49134`).

## Registered functions

`provider::anthropic::complete` (canonical) plus the deprecated alias
`provider::anthropic::stream_assistant` for one release.

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | `provider-base::fetch_credential` calls `auth::get_token` to look up the Anthropic API key or OAuth token. |

## Build

```bash
cargo build --release
```
