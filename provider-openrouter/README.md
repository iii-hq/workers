# provider-openrouter

OpenAI-compatible provider under provider::openrouter::*.

## Installation

```bash
iii worker add provider-openrouter
```

## Run

```bash
iii-provider-openrouter
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
