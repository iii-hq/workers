# provider-opencode-go

OpenAI-compatible provider under provider::opencode-go::*.

## Installation

```bash
iii worker add provider-opencode-go
```

## Run

```bash
iii-provider-opencode-go
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
