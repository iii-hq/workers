# provider-groq

OpenAI-compatible provider under provider::groq::*.

## Installation

```bash
iii worker add provider-groq
```

## Run

```bash
iii-provider-groq
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
