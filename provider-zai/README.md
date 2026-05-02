# provider-zai

OpenAI-compatible provider under provider::zai::*.

## Installation

```bash
iii worker add provider-zai
```

## Run

```bash
iii-provider-zai
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
