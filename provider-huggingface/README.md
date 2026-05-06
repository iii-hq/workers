# provider-huggingface

OpenAI-compatible provider under provider::huggingface::*.

## Installation

```bash
iii worker add provider-huggingface
```

## Run

```bash
iii-provider-huggingface
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
