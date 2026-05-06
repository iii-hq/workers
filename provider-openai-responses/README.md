# provider-openai-responses

OpenAI Responses API provider under provider::openai-responses::*.

## Installation

```bash
iii worker add provider-openai-responses
```

## Run

```bash
iii-provider-openai-responses
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
