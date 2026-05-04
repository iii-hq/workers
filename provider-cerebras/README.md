# provider-cerebras

OpenAI-compatible provider under provider::cerebras::*.

## Installation

```bash
iii worker add provider-cerebras
```

## Run

```bash
iii-provider-cerebras
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
