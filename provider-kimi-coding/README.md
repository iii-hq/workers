# provider-kimi-coding

OpenAI-compatible provider under provider::kimi-coding::*.

## Installation

```bash
iii worker add provider-kimi-coding
```

## Run

```bash
iii-provider-kimi-coding
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
