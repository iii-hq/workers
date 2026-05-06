# provider-deepseek

OpenAI-compatible provider under provider::deepseek::*.

## Installation

```bash
iii worker add provider-deepseek
```

## Run

```bash
iii-provider-deepseek
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
