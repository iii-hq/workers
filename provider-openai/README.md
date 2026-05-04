# provider-openai

OpenAI Chat Completions provider under provider::openai::*.

## Installation

```bash
iii worker add provider-openai
```

## Run

```bash
iii-provider-openai
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
