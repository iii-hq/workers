# provider-vercel-ai-gateway

OpenAI-compatible provider under provider::vercel-ai-gateway::*.

## Installation

```bash
iii worker add provider-vercel-ai-gateway
```

## Run

```bash
iii-provider-vercel-ai-gateway
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
