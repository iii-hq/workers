# provider-mistral

OpenAI-compatible provider under provider::mistral::*.

## Installation

```bash
iii worker add provider-mistral
```

## Run

```bash
iii-provider-mistral
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
