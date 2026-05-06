# provider-xai

OpenAI-compatible provider under provider::xai::*.

## Installation

```bash
iii worker add provider-xai
```

## Run

```bash
iii-provider-xai
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
