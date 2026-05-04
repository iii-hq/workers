# provider-azure-openai

Azure OpenAI Responses provider under provider::azure-openai::*.

## Installation

```bash
iii worker add provider-azure-openai
```

## Run

```bash
iii-provider-azure-openai
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
